#include "wireless_listener.h"
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
// Task 4 self-tests: TabletBootstrap and StreamSession against a simulated PC over an in-memory duplex pipe,
// and BindingStore against the real Asset Store of the device. The simulated PC follows host_pair /
// host_proceed (src-tauri/src/hdc_bootstrap.rs) and the host half of the P5-A v2 handshake as written in
// PairingProtocol.java; a mistake in it can only make these tests fail, never pass.
#include "anhb_codec.h"
#include "anpu_codec.h"
#include "binding_store.h"
#include "crypto.h"
#include "hdc_bootstrap.h"
#include "ohaudio_output.h"
#include "p5c_control.h"
#include "socket_stream.h"
#include "stream_session.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace anpod {
namespace {

using namespace std::chrono_literals;

template <size_t N> std::array<uint8_t, N> filled(uint8_t v) {
    std::array<uint8_t, N> a;
    a.fill(v);
    return a;
}

std::span<const uint8_t> ascii(const char* s) { return {reinterpret_cast<const uint8_t*>(s), std::strlen(s)}; }
void append(Bytes& out, std::span<const uint8_t> part) { out.insert(out.end(), part.begin(), part.end()); }

// ---- in-memory duplex pipe -------------------------------------------------------------------------------

struct PipeShared {
    std::mutex mutex;
    std::condition_variable changed;
    std::deque<uint8_t> toEnd[2];
    bool closed = false;
};

// Socket-like: after close() the local end fails at once, the peer drains what was sent and then sees EOF.
class PipeEnd final : public ByteStream {
public:
    PipeEnd(std::shared_ptr<PipeShared> shared, int side) : shared_(std::move(shared)), side_(side) {}
    bool readExact(uint8_t* out, size_t count) override {
        std::unique_lock<std::mutex> lock(shared_->mutex);
        auto& in = shared_->toEnd[side_];
        shared_->changed.wait_for(lock, 10s, [&] { return localClosed_ || in.size() >= count || shared_->closed; });
        if (localClosed_ || in.size() < count) return false;
        std::copy_n(in.begin(), count, out);
        in.erase(in.begin(), in.begin() + static_cast<std::ptrdiff_t>(count));
        return true;
    }
    bool writeAll(std::span<const uint8_t> data) override {
        std::lock_guard<std::mutex> lock(shared_->mutex);
        if (shared_->closed) return false;
        shared_->toEnd[1 - side_].insert(shared_->toEnd[1 - side_].end(), data.begin(), data.end());
        shared_->changed.notify_all();
        return true;
    }
    size_t available() {
        std::lock_guard<std::mutex> lock(shared_->mutex);
        return shared_->toEnd[side_].size();
    }
    void close() override {
        std::lock_guard<std::mutex> lock(shared_->mutex);
        shared_->closed = true;
        localClosed_ = true;
        shared_->changed.notify_all();
    }

private:
    std::shared_ptr<PipeShared> shared_;
    int side_;
    bool localClosed_ = false;
};

struct Pipe {
    std::unique_ptr<PipeEnd> tablet, pc;
};

Pipe makePipe() {
    auto shared = std::make_shared<PipeShared>();
    return {std::make_unique<PipeEnd>(shared, 0), std::make_unique<PipeEnd>(shared, 1)};
}

// ---- bootstrap -------------------------------------------------------------------------------------------

bool sendAnhb(PipeEnd& pc, AnhbKind kind, std::span<const uint8_t> body = {}) {
    Bytes frame = encodeAnhb(kind, body);
    return !frame.empty() && pc.writeAll(frame);
}

struct BootstrapRun {
    std::mutex mutex;
    std::string code;
    bool pcConfirmedSeen = false;
    std::unique_ptr<TabletBootstrap> bootstrap;
    std::optional<BootstrapOutcome> outcome;
    std::thread thread;
    PipeEnd& pc;

    // onCode / onPcConfirmed run on the bootstrap thread and may call answerPairing.
    BootstrapRun(PipeEnd& tablet, PipeEnd& pcEnd, const Id16& hostId, std::function<void(TabletBootstrap&)> onCode,
                 std::function<void(TabletBootstrap&)> onPcConfirmed,
                 TabletBootstrap::FindBinding find = BindingStore::find)
        : pc(pcEnd) {
        BootstrapUi ui;
        ui.showCode = [this, onCode](const std::string& c) {
            {
                std::lock_guard<std::mutex> lock(mutex);
                code = c;
            }
            if (onCode) onCode(*bootstrap);
        };
        ui.codeConfirmedByPc = [this, onPcConfirmed] {
            {
                std::lock_guard<std::mutex> lock(mutex);
                pcConfirmedSeen = true;
            }
            if (onPcConfirmed) onPcConfirmed(*bootstrap);
        };
        bootstrap = std::make_unique<TabletBootstrap>(tablet, filled<16>(0x7a), hostId, ui, std::move(find));
        thread = std::thread([this] { outcome.emplace(bootstrap->run()); });
    }
    ~BootstrapRun() {
        if (!thread.joinable()) return;
        pc.close();  // a failed check must not leave run() waiting for the 120 s deadline
        thread.join();
    }
    BootstrapOutcome& join() {
        thread.join();
        return *outcome;
    }
    std::string shownCode() {
        std::lock_guard<std::mutex> lock(mutex);
        return code;
    }
};

bool helloIs(PipeEnd& pc, uint8_t hasBinding) {
    auto hello = readAnhb(pc);
    if (!hello || hello->kind != AnhbKind::TabletHello) return false;
    auto token = filled<16>(0x7a);
    return std::equal(token.begin(), token.end(), hello->body.begin()) && hello->body[16] == hasBinding;
}

// PC side of the offer: returns the tablet nonce, or nullopt.
std::optional<Nonce16> offer(PipeEnd& pc, const BindingMaterial& material, const Nonce16& pcNonce) {
    auto record = material.encode();
    Bytes body(record.begin(), record.end());
    append(body, pcNonce);
    if (!sendAnhb(pc, AnhbKind::PairOffer, body)) return std::nullopt;
    auto reply = readAnhb(pc);
    if (!reply || reply->kind != AnhbKind::PairNonce) return std::nullopt;
    Nonce16 tabletNonce;
    std::memcpy(tabletNonce.data(), reply->body.data(), 16);
    return tabletNonce;
}

bool frameIs(PipeEnd& pc, AnhbKind kind, std::span<const uint8_t> body) {
    auto frame = readAnhb(pc);
    return frame && frame->kind == kind && frame->body.size() == body.size() &&
           std::equal(body.begin(), body.end(), frame->body.begin());
}

bool sameBinding(const BindingMaterial& a, const BindingMaterial& b) { return a.encode() == b.encode(); }

// A store error counts as "stored" so that it can never make a no-binding assertion pass.
bool stored(const Id16& id) {
    std::optional<BindingMaterial> found;
    return !BindingStore::find(id, found) || found.has_value();
}

std::optional<BindingMaterial> findOk(const Id16& id) {
    std::optional<BindingMaterial> found;
    return BindingStore::find(id, found) ? std::move(found) : std::nullopt;
}

bool bootstrapWithBindingProceedsWithoutCode() {
    Id16 id = filled<16>(0xb1);
    BindingMaterial material(id, filled<16>(0x02), filled<32>(0x03));
    if (!BindingStore::save(material)) return false;
    Pipe pipe = makePipe();
    bool ok;
    {
        BootstrapRun run(*pipe.tablet, *pipe.pc, id, nullptr, nullptr);
        ok = helloIs(*pipe.pc, 1) && sendAnhb(*pipe.pc, AnhbKind::Proceed);
        // The first ANPU byte after Proceed must still be readable by the session (not eaten by bootstrap).
        const uint8_t next[1] = {'A'};
        ok = pipe.pc->writeAll(next) && ok;
        auto& outcome = run.join();
        auto* proceed = std::get_if<BootstrapProceed>(&outcome);
        uint8_t first = 0;
        ok = ok && proceed && sameBinding(proceed->binding, material) && run.shownCode().empty() &&
             pipe.tablet->readExact(&first, 1) && first == 'A';
    }
    return BindingStore::remove(id) && ok;
}

bool bootstrapPairsOnlyAfterPcConfirmAndLocalAccept() {
    Id16 id = filled<16>(0xb2);
    BindingStore::remove(id);
    BindingMaterial material(id, filled<16>(0x22), filled<32>(0x33));
    Nonce16 pcNonce = filled<16>(0x04);
    Pipe pipe = makePipe();
    bool ok;
    {
        // The local user accepts as soon as the code shows; the PC has not confirmed yet.
        BootstrapRun run(*pipe.tablet, *pipe.pc, id, [](TabletBootstrap& b) { b.answerPairing(true); }, nullptr);
        ok = helloIs(*pipe.pc, 0);
        auto tabletNonce = offer(*pipe.pc, material, pcNonce);
        ok = ok && tabletNonce.has_value();
        std::this_thread::sleep_for(300ms);
        ok = ok && !stored(id) && run.shownCode() == material.pairingCode(pcNonce, *tabletNonce);
        ok = ok && sendAnhb(*pipe.pc, AnhbKind::PairConfirmed) &&
             frameIs(*pipe.pc, AnhbKind::PairSaved, material.commitment());
        auto saved = findOk(id);
        ok = ok && saved && sameBinding(*saved, material) && sendAnhb(*pipe.pc, AnhbKind::Proceed);
        auto& outcome = run.join();
        auto* proceed = std::get_if<BootstrapProceed>(&outcome);
        ok = ok && proceed && sameBinding(proceed->binding, material);
    }
    return BindingStore::remove(id) && ok;
}

bool bootstrapCancelLeavesNoBinding() {
    Id16 id = filled<16>(0xb3);
    BindingStore::remove(id);
    BindingMaterial material(id, filled<16>(0x22), filled<32>(0x33));
    const uint8_t user[1] = {1};
    bool ok = true;
    {
        // PC confirms, then the tablet user declines: PairCancel{1}, nothing saved.
        Pipe pipe = makePipe();
        BootstrapRun run(*pipe.tablet, *pipe.pc, id, nullptr, [](TabletBootstrap& b) { b.answerPairing(false); });
        ok = helloIs(*pipe.pc, 0) && offer(*pipe.pc, material, filled<16>(4)) &&
             sendAnhb(*pipe.pc, AnhbKind::PairConfirmed) && frameIs(*pipe.pc, AnhbKind::PairCancel, user);
        ok = std::holds_alternative<BootstrapCancelled>(run.join()) && ok && !stored(id) && !readAnhb(*pipe.pc);
    }
    {
        // The tablet user accepted, then the PC cancels: nothing saved.
        Pipe pipe = makePipe();
        BootstrapRun run(*pipe.tablet, *pipe.pc, id, [](TabletBootstrap& b) { b.answerPairing(true); }, nullptr);
        ok = helloIs(*pipe.pc, 0) && offer(*pipe.pc, material, filled<16>(4)) &&
             sendAnhb(*pipe.pc, AnhbKind::PairCancel, user) && ok;
        ok = std::holds_alternative<BootstrapCancelled>(run.join()) && ok && !stored(id);
    }
    {
        // Saved and PairSaved sent, then the PC cancels instead of Proceed: the saved binding is removed.
        Pipe pipe = makePipe();
        BootstrapRun run(*pipe.tablet, *pipe.pc, id, [](TabletBootstrap& b) { b.answerPairing(true); }, nullptr);
        ok = helloIs(*pipe.pc, 0) && offer(*pipe.pc, material, filled<16>(4)) &&
             sendAnhb(*pipe.pc, AnhbKind::PairConfirmed) &&
             frameIs(*pipe.pc, AnhbKind::PairSaved, material.commitment()) && findOk(id).has_value() &&
             sendAnhb(*pipe.pc, AnhbKind::PairCancel, user) && ok;
        ok = std::holds_alternative<BootstrapCancelled>(run.join()) && ok && !stored(id);
    }
    return ok;
}

bool bootstrapStoreErrorSendsNoHello() {
    Pipe pipe = makePipe();
    BootstrapRun run(*pipe.tablet, *pipe.pc, filled<16>(0xb7), nullptr, nullptr,
                     [](const Id16&, std::optional<BindingMaterial>&) { return false; });
    auto& outcome = run.join();
    auto* failed = std::get_if<BootstrapFailed>(&outcome);
    // No TabletHello{hasBinding=0}: the stream is closed with nothing written.
    return failed && failed->message == "binding store unavailable" && !readAnhb(*pipe.pc);
}

bool bootstrapRejectsOfferForOtherHost() {
    Id16 id = filled<16>(0xb4), other = filled<16>(0xb5);
    BindingStore::remove(id);
    BindingStore::remove(other);
    BindingMaterial material(other, filled<16>(0x22), filled<32>(0x33));
    Pipe pipe = makePipe();
    BootstrapRun run(*pipe.tablet, *pipe.pc, id, [](TabletBootstrap& b) { b.answerPairing(true); }, nullptr);
    const uint8_t user[1] = {1};
    auto record = material.encode();
    Bytes body(record.begin(), record.end());
    append(body, filled<16>(4));
    bool ok = helloIs(*pipe.pc, 0) && sendAnhb(*pipe.pc, AnhbKind::PairOffer, body) &&
              frameIs(*pipe.pc, AnhbKind::PairCancel, user);
    return std::holds_alternative<BootstrapFailed>(run.join()) && ok && run.shownCode().empty() && !stored(id) &&
           !stored(other);
}

// A local app can launch AnPod with its own port and a known hostId: an offer must never replace a stored binding.
bool bootstrapRejectsOfferWhenAlreadyBound() {
    Id16 id = filled<16>(0xb9);
    BindingMaterial saved(id, filled<16>(0x02), filled<32>(0x03)), forged(id, filled<16>(0x22), filled<32>(0x33));
    Pipe pipe = makePipe();
    BootstrapRun run(*pipe.tablet, *pipe.pc, id, [](TabletBootstrap& b) { b.answerPairing(true); }, nullptr,
                     [saved](const Id16&, std::optional<BindingMaterial>& out) {
                         out = saved;
                         return true;
                     });
    auto record = forged.encode();
    Bytes body(record.begin(), record.end());
    append(body, filled<16>(4));
    // No PairNonce and no code: the stream is closed with nothing more written.
    if (!helloIs(*pipe.pc, 1) || !sendAnhb(*pipe.pc, AnhbKind::PairOffer, body) || readAnhb(*pipe.pc)) return false;
    auto* failed = std::get_if<BootstrapFailed>(&run.join());
    return failed && failed->message == "pairing offer for an already paired PC" && run.shownCode().empty();
}

bool bootstrapFailedPairSavedSendRemovesBinding() {
    Id16 id = filled<16>(0xb8);
    BindingStore::remove(id);
    BindingMaterial material(id, filled<16>(0x22), filled<32>(0x33));
    Pipe pipe = makePipe();
    // Accepted locally first; the PC confirms and drops the stream before PairSaved can be written.
    BootstrapRun run(*pipe.tablet, *pipe.pc, id, [](TabletBootstrap& b) { b.answerPairing(true); }, nullptr);
    bool ok = helloIs(*pipe.pc, 0) && offer(*pipe.pc, material, filled<16>(4)) &&
              sendAnhb(*pipe.pc, AnhbKind::PairConfirmed);
    pipe.pc->close();
    return std::holds_alternative<BootstrapFailed>(run.join()) && ok && !stored(id);
}

bool bindingStoreSaveFindRemove() {
    Id16 id = filled<16>(0xb6);
    BindingStore::remove(id);
    BindingMaterial first(id, filled<16>(0x22), filled<32>(0x33));
    BindingMaterial second(id, filled<16>(0x44), filled<32>(0x55));
    auto listed = [&] {
        auto ids = BindingStore::list();
        return ids && std::find(ids->begin(), ids->end(), id) != ids->end();
    };
    bool ok = !stored(id) && !listed() && BindingStore::save(first);
    auto found = findOk(id);
    ok = ok && BindingStore::list().has_value() && found && sameBinding(*found, first) && listed() &&
         BindingStore::save(second);
    found = findOk(id);  // a second save for the same PC overwrites
    ok = ok && found && sameBinding(*found, second);
    return BindingStore::remove(id) && ok && !stored(id) && !listed() && BindingStore::remove(id);
}

// ---- stream session ------------------------------------------------------------------------------------

class MemorySink final : public PlaybackSink {
public:
    bool open(const AudioFormatV2& format) override {
        std::lock_guard<std::mutex> lock(mutex_);
        opened_ = format;
        return true;
    }
    bool write(const float*, size_t frames) override {
        std::lock_guard<std::mutex> lock(mutex_);
        frames_ += frames;
        return true;
    }
    void pause() override { pauses++; }
    void resume() override { resumes++; }
    void close() override { closes++; }
    PlaybackStats stats() const override { return {}; }
    std::atomic<int> pauses{0}, resumes{0}, closes{0};
    std::optional<AudioFormatV2> opened() {
        std::lock_guard<std::mutex> lock(mutex_);
        return opened_;
    }

private:
    mutable std::mutex mutex_;
    std::optional<AudioFormatV2> opened_;
    size_t frames_ = 0;
};

struct Ended {
    std::mutex mutex;
    std::condition_variable changed;
    std::optional<std::string> message;

    SessionCallbacks callbacks() {
        SessionCallbacks c;
        c.onEnded = [this](const std::string& m) {
            std::lock_guard<std::mutex> lock(mutex);
            message = m;
            changed.notify_all();
        };
        return c;
    }
    std::optional<std::string> wait() {
        std::unique_lock<std::mutex> lock(mutex);
        changed.wait_for(lock, 5s, [&] { return message.has_value(); });
        return message;
    }
};

struct Host {
    ByteStream& pc;
    uint64_t sessionId = 0x0102030405060708ULL;
    Key32 h2rControl{}, r2hControl{};
    uint64_t command = 0;
    bool wireless = false;  // P5c framing: raw handshake records, control frames behind a 2-byte length

    bool writeHandshake(const Bytes& body) {
        return wireless ? pc.writeAll(body) : pc.writeAll(encodeAnpu(AnpuType::Handshake, body));
    }
    std::optional<Bytes> readHandshake(size_t size) {
        if (!wireless) {
            auto record = readAnpu(pc);
            return record && record->type == AnpuType::Handshake ? std::optional<Bytes>(record->body) : std::nullopt;
        }
        Bytes body(size);
        return pc.readExact(body.data(), size) ? std::optional<Bytes>(std::move(body)) : std::nullopt;
    }

    // Host half of the P5-A v2 handshake (PairingProtocol.java), on the ADB-stream derived material.
    bool handshake(const BindingMaterial& derived, const AudioFormatV2& format) {
        auto record = derived.encode();
        Key32 secret;
        std::memcpy(secret.data(), record.data() + 37, 32);
        auto descriptor = format.encode();
        Nonce32 hostNonce = filled<32>(0x44);
        Bytes hello = {'A', 'N', 'P', '5', 2, 1, 0, 96};
        append(hello, derived.hostId());
        append(hello, derived.receiverId());
        append(hello, hostNonce);
        for (int shift = 56; shift >= 0; shift -= 8) hello.push_back(static_cast<uint8_t>(sessionId >> shift));
        append(hello, descriptor);
        if (!writeHandshake(hello)) return false;
        auto challenge = readHandshake(104);
        if (!challenge || challenge->size() != 104 || (*challenge)[5] != 2) return false;

        Bytes transcript = {'A', 'N', 'P', '5', 2, 1, 2};
        append(transcript, derived.hostId());
        append(transcript, derived.receiverId());
        append(transcript, hostNonce);
        append(transcript, std::span<const uint8_t>(*challenge).subspan(24, 32));
        for (int shift = 56; shift >= 0; shift -= 8) transcript.push_back(static_cast<uint8_t>(sessionId >> shift));
        append(transcript, descriptor);
        append(transcript, descriptor);
        Bytes proofInput;
        append(proofInput, ascii("AnPod P5-A proof host v2"));
        proofInput.push_back(0);
        append(proofInput, transcript);
        Bytes finish = {'A', 'N', 'P', '5', 2, 3, 0, 40};
        append(finish, hmacSha256(secret, proofInput));
        if (!writeHandshake(finish)) return false;
        auto ready = readHandshake(40);
        if (!ready || ready->size() != 40 || (*ready)[5] != 4) return false;

        Bytes saltInput;
        append(saltInput, ascii("AnPod P5-A HKDF-SHA256 salt v2"));
        saltInput.push_back(0);
        append(saltInput, transcript);
        Digest prk = hmacSha256(sha256(saltInput), secret);
        auto key = [&](const char* label) {
            Bytes info;
            append(info, ascii(label));
            info.push_back(1);
            return hmacSha256(prk, info);
        };
        h2rControl = key("AnPod P5-A key host-to-receiver control HMAC-SHA256 v2");
        r2hControl = key("AnPod P5-A key receiver-to-host control HMAC-SHA256 v2");
        return true;
    }

    bool send(uint8_t type, std::span<const uint8_t> payload, uint64_t sequence) {
        Bytes frame = p5c::encode(h2rControl, p5c::kHostToReceiver, sessionId, sequence, type, payload);
        if (!wireless) return pc.writeAll(encodeAnpu(AnpuType::Control, frame));
        const uint8_t size[] = {static_cast<uint8_t>(frame.size() >> 8), static_cast<uint8_t>(frame.size())};
        return pc.writeAll(size) && pc.writeAll(frame);
    }
    std::optional<p5c::Frame> reply() {
        Bytes body;
        if (wireless) {
            uint8_t size[2];
            if (!pc.readExact(size, 2)) return std::nullopt;
            body.resize(size_t{size[0]} << 8 | size[1]);
            if (!pc.readExact(body.data(), body.size())) return std::nullopt;
        } else {
            auto record = readAnpu(pc);
            if (!record || record->type != AnpuType::Control) return std::nullopt;
            body = std::move(record->body);
        }
        return p5c::decodeRaw(body, r2hControl, p5c::kReceiverToHost, sessionId);
    }
    // Sends the next command and returns its ACK payload (empty when there is none or it does not match).
    Bytes exchange(uint8_t type, std::span<const uint8_t> payload = {}) {
        uint64_t sequence = command++;
        if (!send(type, payload, sequence)) return {};
        auto ack = reply();
        if (!ack || ack->type != p5c::kAck || ack->payload.size() < 13 || ack->payload[0] != type) return {};
        uint64_t echoed = 0;
        for (int i = 1; i <= 8; i++) echoed = echoed << 8 | ack->payload[i];
        return echoed == sequence ? ack->payload : Bytes{};
    }
    // CAPABILITIES_QUERY with the PC's wired (or wireless) requirements; returns the 20-byte info payload.
    Bytes capabilities() {
        uint32_t required = 0x82 | p5c::kCapAudioV2 | p5c::kCapAudioFloat32 | p5c::kCapTerminalCause |
                            (wireless ? 0 : p5c::kCapAudioAdbStream);
        Bytes query = p5c::capabilitiesQueryPayload(p5c::kManagementControlMajor, p5c::kAudioProtocolMajor, required);
        if (!send(p5c::kCapabilitiesQuery, query, command++)) return {};
        auto info = reply();
        return info && info->type == p5c::kCapabilitiesInfo && info->payload.size() == 20 ? info->payload : Bytes{};
    }
};

uint32_t capabilityBits(const Bytes& info) {
    return uint32_t{info[12]} << 24 | uint32_t{info[13]} << 16 | uint32_t{info[14]} << 8 | info[15];
}

BindingMaterial sessionBinding() { return {filled<16>(1), filled<16>(2), filled<32>(3)}; }

bool sessionUserStopSendsResultUserStopped() {
    Pipe pipe = makePipe();
    PipeEnd& pc = *pipe.pc;
    MemorySink sink;
    Ended ended;
    auto session = StreamSession::start(std::move(pipe.tablet), sessionBinding(), sink, ended.callbacks());
    Host host{pc};
    auto format = *AudioFormatV2::forProfile(4, 0);
    if (!host.handshake(sessionBinding().deriveAdbStream(), format)) return false;
    Bytes info = host.capabilities();
    if (info.empty() || info[11] != 0 || (capabilityBits(info) & p5c::kCapAudioFloat32) == 0) return false;
    const uint8_t zeroPort[2] = {0, 0};
    const uint8_t mode[1] = {format.mode};
    Bytes bind = host.exchange(p5c::kBind, zeroPort);
    Bytes select = host.exchange(p5c::kSelectMode, mode);
    Bytes start = host.exchange(p5c::kStart);
    Bytes progress = host.exchange(p5c::kPlaybackProgressQuery);
    auto opened = sink.opened();
    if (bind.size() != 13 || bind[9] != 0 || bind[10] != 1 || select.size() != 13 || start.size() != 13 ||
        start[9] != 0 || start[10] != 2 || progress.size() != 13 + 78 || !opened ||
        opened->encode() != format.encode())
        return false;
    session->stop(true);
    Bytes stopped = host.exchange(p5c::kHeartbeat);
    auto message = ended.wait();
    return stopped.size() == 13 && stopped[9] == p5c::kResultUserStopped && stopped[10] == 5 && message &&
           message->empty() && !readAnpu(pc);
}

// Closing the app waits for the run's end: that end must come after the USER_STOPPED ACK was written.
bool sessionUserStopAckWrittenBeforeEnd() {
    Pipe pipe = makePipe();
    PipeEnd& pc = *pipe.pc;
    MemorySink sink;
    std::mutex mutex;
    std::condition_variable changed;
    std::optional<size_t> waitingAtEnd;
    SessionCallbacks callbacks;
    callbacks.onEnded = [&](const std::string&) {
        std::lock_guard<std::mutex> lock(mutex);
        waitingAtEnd = pc.available();
        changed.notify_all();
    };
    auto session = StreamSession::start(std::move(pipe.tablet), sessionBinding(), sink, callbacks);
    Host host{pc};
    auto format = *AudioFormatV2::forProfile(4, 0);
    if (!host.handshake(sessionBinding().deriveAdbStream(), format) || host.capabilities().empty()) return false;
    session->stop(true);
    if (!host.send(p5c::kHeartbeat, {}, host.command++)) return false;
    {
        std::unique_lock<std::mutex> lock(mutex);
        changed.wait_for(lock, 5s, [&] { return waitingAtEnd.has_value(); });
    }
    // When onEnded ran, the bytes waiting for the PC were exactly the whole USER_STOPPED ACK record.
    auto ack = host.reply();
    return waitingAtEnd && *waitingAtEnd > 0 && ack && ack->type == p5c::kAck && ack->payload.size() == 13 &&
           ack->payload[9] == p5c::kResultUserStopped && !readAnpu(pc);
}

// ---- wireless (P5c over a loopback TCP connection) ---------------------------------------------------------

struct WirelessPair {
    std::unique_ptr<SocketStream> pc;
    int tabletFd = -1;
};

std::optional<WirelessPair> wirelessPair() {
    int listener = ::socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    sockaddr_in address{};
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    socklen_t size = sizeof address;
    std::optional<WirelessPair> pair;
    if (listener >= 0 && ::bind(listener, reinterpret_cast<sockaddr*>(&address), size) == 0 &&
        ::listen(listener, 1) == 0 && ::getsockname(listener, reinterpret_cast<sockaddr*>(&address), &size) == 0) {
        int pc = ::socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
        // A tablet end nobody serves fails the test instead of hanging it.
        timeval timeout{5, 0};
        int tablet = pc >= 0 && ::setsockopt(pc, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof timeout) == 0 &&
                             ::connect(pc, reinterpret_cast<sockaddr*>(&address), size) == 0
                         ? ::accept4(listener, nullptr, nullptr, SOCK_CLOEXEC)
                         : -1;
        if (tablet >= 0) pair = WirelessPair{std::make_unique<SocketStream>(pc), tablet};
        else if (pc >= 0) ::close(pc);
    }
    if (listener >= 0) ::close(listener);
    return pair;
}

struct WirelessRun {
    WirelessPair pair;
    MemorySink sink;
    Ended ended;
    std::unique_ptr<StreamSession> session;
    Host host;
    explicit WirelessRun(WirelessPair p, std::optional<Nonce16> intent)
        : pair(std::move(p)), host{*pair.pc} {
        host.wireless = true;
        session = StreamSession::startWireless(pair.tabletFd, sessionBinding(), intent, sink, ended.callbacks());
    }
    // Handshake on the plain binding (no ADB-stream derivation on P5c) and the capability exchange.
    Bytes ready() {
        if (!host.handshake(sessionBinding(), *AudioFormatV2::forProfile(5, 0))) return {};
        return host.capabilities();
    }
};

const Nonce16 kIntent = filled<16>(0x5a);

bool wirelessIntentMismatchEndsSession() {
    auto pair = wirelessPair();
    if (!pair) return false;
    WirelessRun run(std::move(*pair), kIntent);
    Bytes info = run.ready();
    if (info.size() != 20 || (capabilityBits(info) & p5c::kCapConnectionIntent) == 0) return false;
    auto wrong = filled<16>(0x5b);
    bool ok = run.host.send(p5c::kConnectionIntent, wrong, run.host.command++) && !run.host.reply();
    auto message = run.ended.wait();
    return ok && message && *message == "connection intent expired or mismatched";
}

bool wirelessSecondIntentRejected() {
    auto pair = wirelessPair();
    if (!pair) return false;
    WirelessRun run(std::move(*pair), kIntent);
    bool ok = run.ready().size() == 20 && run.host.exchange(p5c::kConnectionIntent, kIntent).size() == 13;
    ok = ok && run.host.send(p5c::kConnectionIntent, kIntent, run.host.command++) && !run.host.reply();
    auto message = run.ended.wait();
    return ok && message && *message == "connection intent expired or mismatched";
}

bool wirelessBindBeforeIntentRejected() {
    auto pair = wirelessPair();
    if (!pair) return false;
    WirelessRun run(std::move(*pair), kIntent);
    const uint8_t port[2] = {0x12, 0x34};
    Bytes rejected = run.ready().size() == 20 ? run.host.exchange(p5c::kBind, port) : Bytes{};
    // Refused with an ACK like Java; the session stays usable and nothing was bound.
    bool ok = rejected.size() == 13 && rejected[9] == 1 && rejected[10] == 0 && rejected[11] == 0 &&
              rejected[12] == 0 && run.session->udpSocketForTest() < 0 &&
              run.host.exchange(p5c::kHeartbeat).size() == 13;
    run.session->stop(false);
    return ok;
}

bool wirelessNoIntentNotAdvertised() {
    auto pair = wirelessPair();
    if (!pair) return false;
    WirelessRun run(std::move(*pair), std::nullopt);
    Bytes info = run.ready();
    bool ok = info.size() == 20 && info[11] == 0 && (capabilityBits(info) & p5c::kCapConnectionIntent) == 0 &&
              (capabilityBits(info) & p5c::kCapAudioAdbStream) == 0;
    run.session->stop(false);
    return ok;
}

bool wirelessBindPortZeroRejected() {
    auto pair = wirelessPair();
    if (!pair) return false;
    WirelessRun run(std::move(*pair), std::nullopt);
    const uint8_t zero[2] = {0, 0};
    bool ok = run.ready().size() == 20 && run.host.send(p5c::kBind, zero, run.host.command++) && !run.host.reply();
    auto message = run.ended.wait();
    return ok && message && *message == "invalid BIND port";
}

bool wirelessBindConnectsUdpToPeer() {
    auto pair = wirelessPair();
    if (!pair) return false;
    WirelessRun run(std::move(*pair), kIntent);
    const uint8_t port[2] = {0xc3, 0x51};  // 50001
    bool ok = run.ready().size() == 20 && run.host.exchange(p5c::kConnectionIntent, kIntent).size() == 13;
    Bytes ack = ok ? run.host.exchange(p5c::kBind, port) : Bytes{};
    int udp = run.session->udpSocketForTest();
    sockaddr_in local{}, peer{};
    socklen_t size = sizeof local;
    ok = ack.size() == 13 && ack[9] == 0 && ack[10] == 1 && udp >= 0 &&
         ::getsockname(udp, reinterpret_cast<sockaddr*>(&local), &size) == 0 &&
         (uint16_t{ack[11]} << 8 | ack[12]) == ntohs(local.sin_port) && local.sin_port != 0;
    size = sizeof peer;
    ok = ok && ::getpeername(udp, reinterpret_cast<sockaddr*>(&peer), &size) == 0 &&
         peer.sin_addr.s_addr == htonl(INADDR_LOOPBACK) && ntohs(peer.sin_port) == 50001;
    run.session->stop(false);
    return ok;
}

bool wirelessControlLengthOutOfBoundsRejected() {
    for (size_t length : {size_t{1}, p5c::kMaxFrameBytes + 1}) {
        auto pair = wirelessPair();
        if (!pair) return false;
        WirelessRun run(std::move(*pair), std::nullopt);
        const uint8_t prefix[2] = {static_cast<uint8_t>(length >> 8), static_cast<uint8_t>(length)};
        bool ok = run.ready().size() == 20 && run.pair.pc->writeAll(prefix);
        auto message = run.ended.wait();
        if (!ok || !message || *message != "session stream closed") return false;
    }
    return true;
}

bool sessionPauseResumeStopReachSink() {
    Pipe pipe = makePipe();
    PipeEnd& pc = *pipe.pc;
    MemorySink sink;
    Ended ended;
    auto session = StreamSession::start(std::move(pipe.tablet), sessionBinding(), sink, ended.callbacks());
    Host host{pc};
    auto format = *AudioFormatV2::forProfile(5, 1);
    if (!host.handshake(sessionBinding().deriveAdbStream(), format)) return false;
    const uint8_t zeroPort[2] = {0, 0};
    const uint8_t mode[1] = {format.mode};
    bool ok = host.capabilities().size() == 20 && host.exchange(p5c::kBind, zeroPort).size() == 13 &&
              host.exchange(p5c::kSelectMode, mode).size() == 13 && host.exchange(p5c::kStart).size() == 13;
    ok = ok && host.exchange(p5c::kPause).size() == 13 && host.exchange(p5c::kPause).size() == 13 && sink.pauses == 1;
    ok = ok && host.exchange(p5c::kResume).size() == 13 && host.exchange(p5c::kResume).size() == 13 &&
         sink.resumes == 1 && sink.closes == 0;
    Bytes stop = host.exchange(p5c::kStop);
    auto message = ended.wait();
    return ok && stop.size() == 13 && message && message->empty() && sink.closes == 1;
}

// OHAudio itself accepts 384 kHz (it resamples to the device), so the rejected cases are a rate the builder refuses
// and a non-Float32 descriptor: both fail, and no other format is tried.
bool ohaudioRejectsUnsupportedWithoutFallback() {
    AudioFormatV2 tooFast = *AudioFormatV2::forProfile(5, 0);
    tooFast.sampleRate = 1'000'000;
    OhAudioOutput a, b;
    return !a.open(tooFast) && !a.isOpen() && !b.open(*AudioFormatV2::forProfile(2, 0)) && !b.isOpen();
}

bool ohaudioOpensNegotiatedFloat32() {
    for (auto [profile, mode] : {std::pair{4, 0}, std::pair{5, 0}}) {
        OhAudioOutput output;
        if (!output.open(*AudioFormatV2::forProfile(profile, mode)) || !output.isOpen()) return false;
        const float silence[16] = {};
        bool wrote = output.write(silence, 8);
        output.close();
        if (!wrote || output.isOpen()) return false;
    }
    return true;
}

// LowLatency either really runs on the FAST path, or fails with nothing open and says why (ruling R10).
// The emulator never grants FAST, so there this exercises the rejection path.
bool ohaudioLowLatencyIsFastOrRejected() {
    OhAudioOutput output;
    bool opened = output.open(*AudioFormatV2::forProfile(4, 1));
    if (opened) return output.isOpen() && output.fast() && output.failureReason().empty();
    return !output.isOpen() && output.failureReason() == "low latency unsupported";
}

// A sink that cannot open names the reason in the session's end message.
class RejectingSink final : public PlaybackSink {
public:
    bool open(const AudioFormatV2&) override { return false; }
    bool write(const float*, size_t) override { return false; }
    void pause() override {}
    void resume() override {}
    void close() override { closes++; }
    std::string failureReason() const override { return "low latency unsupported"; }
    PlaybackStats stats() const override { return {}; }
    std::atomic<int> closes{0};
};

bool sessionReportsLowLatencyUnsupported() {
    Pipe pipe = makePipe();
    PipeEnd& pc = *pipe.pc;
    RejectingSink sink;
    Ended ended;
    auto session = StreamSession::start(std::move(pipe.tablet), sessionBinding(), sink, ended.callbacks());
    Host host{pc};
    auto format = *AudioFormatV2::forProfile(4, 1);
    if (!host.handshake(sessionBinding().deriveAdbStream(), format)) return false;
    const uint8_t zeroPort[2] = {0, 0};
    const uint8_t mode[1] = {format.mode};
    bool ok = host.capabilities().size() == 20 && host.exchange(p5c::kBind, zeroPort).size() == 13 &&
              host.exchange(p5c::kSelectMode, mode).size() == 13;
    // Like Java, START is answered with RESULT_FAILED before the session ends, so the PC reports a START
    // failure instead of a dropped connection.
    Bytes start = ok ? host.exchange(p5c::kStart) : Bytes{};
    auto message = ended.wait();
    const char* expected = "audio output cannot open 48000 Hz Float32 2ch fast: low latency unsupported";
    return ok && start.size() == 13 && start[9] == 2 && message && *message == expected && sink.closes == 0 &&
           !readAnpu(pc);
}

bool wirelessHelloSelectsHostWithoutConsumingBytes() {
    int fds[2];
    if (::socketpair(AF_UNIX, SOCK_STREAM, 0, fds) != 0) return false;
    Bytes hello(96);
    const uint8_t header[] = {'A', 'N', 'P', '5', 2, 1, 0, 96};
    std::copy_n(header, 8, hello.begin());
    Id16 host{};
    host[15] = 99;
    std::copy(host.begin(), host.end(), hello.begin() + 8);
    bool ok = ::send(fds[0], hello.data(), hello.size(), 0) == 96 && wirelessHelloHost(fds[1]) == host;
    uint8_t first = 0;
    ok = ok && ::recv(fds[1], &first, 1, 0) == 1 && first == 'A';
    ::close(fds[0]);
    ::close(fds[1]);
    return ok;
}

bool sessionRejectsReplayedCommand() {
    Pipe pipe = makePipe();
    PipeEnd& pc = *pipe.pc;
    MemorySink sink;
    Ended ended;
    auto session = StreamSession::start(std::move(pipe.tablet), sessionBinding(), sink, ended.callbacks());
    Host host{pc};
    if (!host.handshake(sessionBinding().deriveAdbStream(), *AudioFormatV2::forProfile(5, 0))) return false;
    const uint8_t zeroPort[2] = {0, 0};
    bool ok = host.capabilities().size() == 20 && host.exchange(p5c::kBind, zeroPort).size() == 13;
    // Same authenticated frame again (sequence 1): no ACK, the session ends.
    ok = ok && host.send(p5c::kBind, zeroPort, 1) && !host.reply();
    auto message = ended.wait();
    return ok && message && *message == "control replay or sequence gap";
}

bool sessionRejectsNonFloat32Profile() {
    Pipe pipe = makePipe();
    PipeEnd& pc = *pipe.pc;
    MemorySink sink;
    Ended ended;
    auto session = StreamSession::start(std::move(pipe.tablet), sessionBinding(), sink, ended.callbacks());
    Host host{pc};
    if (!host.handshake(sessionBinding().deriveAdbStream(), *AudioFormatV2::forProfile(2, 0))) return false;
    Bytes info = host.capabilities();
    auto message = ended.wait();
    return info.size() == 20 && info[11] == 1 &&
           (capabilityBits(info) & (p5c::kCapAudioFloat32 | p5c::kCapAudioAdbStream | p5c::kCapAudioV2)) == 0 &&
           message && message->find("Float32") != std::string::npos && !sink.opened();
}

}  // namespace

// PC half of one wireless session for the run-loop tests in napi_entry.cpp: hands the tablet end of a fresh
// loopback connection to `deliver`, authenticates on `binding`, reads the capabilities, calls `ready`, then sends
// `command` (when >= 0) and reports its ACK result (-1 when none came).
bool selfTestWirelessPeer(const BindingMaterial& binding, const std::function<void(int)>& deliver,
                          const std::function<void()>& ready, int command, uint32_t& capabilities, int& result) {
    auto pair = wirelessPair();
    if (!pair) return false;
    deliver(pair->tabletFd);
    Host host{*pair->pc};
    host.wireless = true;
    if (!host.handshake(binding, *AudioFormatV2::forProfile(5, 0))) return false;
    Bytes info = host.capabilities();
    if (info.size() != 20) return false;
    capabilities = capabilityBits(info);
    ready();
    result = -1;
    if (command >= 0) {
        Bytes ack = host.exchange(static_cast<uint8_t>(command));
        if (ack.size() == 13) result = ack[9];
    }
    return true;
}

void runSessionSelfTests(std::vector<std::string>& failures) {
    struct Case {
        const char* name;
        bool (*fn)();
    };
    static constexpr Case cases[] = {
        {"binding_store_save_find_remove", bindingStoreSaveFindRemove},
        {"bootstrap_with_binding_proceeds_without_code", bootstrapWithBindingProceedsWithoutCode},
        {"bootstrap_pairs_only_after_pc_confirm_and_local_accept", bootstrapPairsOnlyAfterPcConfirmAndLocalAccept},
        {"bootstrap_cancel_leaves_no_binding", bootstrapCancelLeavesNoBinding},
        {"bootstrap_rejects_offer_for_other_host", bootstrapRejectsOfferForOtherHost},
        {"bootstrap_store_error_sends_no_hello", bootstrapStoreErrorSendsNoHello},
        {"bootstrap_rejects_offer_when_already_bound", bootstrapRejectsOfferWhenAlreadyBound},
        {"bootstrap_failed_pair_saved_send_removes_binding", bootstrapFailedPairSavedSendRemovesBinding},
        {"session_user_stop_sends_result_user_stopped", sessionUserStopSendsResultUserStopped},
        {"session_user_stop_ack_written_before_end", sessionUserStopAckWrittenBeforeEnd},
        {"wireless_intent_mismatch_ends_session", wirelessIntentMismatchEndsSession},
        {"wireless_second_intent_rejected", wirelessSecondIntentRejected},
        {"wireless_bind_before_intent_rejected", wirelessBindBeforeIntentRejected},
        {"wireless_no_intent_not_advertised", wirelessNoIntentNotAdvertised},
        {"wireless_bind_port_zero_rejected", wirelessBindPortZeroRejected},
        {"wireless_bind_connects_udp_to_peer", wirelessBindConnectsUdpToPeer},
        {"wireless_control_length_out_of_bounds_rejected", wirelessControlLengthOutOfBoundsRejected},
        {"wireless_hello_selects_host", wirelessHelloSelectsHostWithoutConsumingBytes},
        {"session_rejects_replayed_command", sessionRejectsReplayedCommand},
        {"session_rejects_non_float32_profile", sessionRejectsNonFloat32Profile},
        {"session_pause_resume_stop_reach_sink", sessionPauseResumeStopReachSink},
        {"ohaudio_rejects_unsupported_without_fallback", ohaudioRejectsUnsupportedWithoutFallback},
        {"ohaudio_opens_negotiated_float32", ohaudioOpensNegotiatedFloat32},
        {"ohaudio_low_latency_is_fast_or_rejected", ohaudioLowLatencyIsFastOrRejected},
        {"session_reports_low_latency_unsupported", sessionReportsLowLatencyUnsupported},
    };
    for (const Case& c : cases)
        if (!c.fn()) failures.emplace_back(c.name);
}

}  // namespace anpod
