// Task 4 self-tests: TabletBootstrap and StreamSession against a simulated PC over an in-memory duplex pipe,
// and BindingStore against the real Asset Store of the device. The simulated PC follows host_pair /
// host_proceed (src-tauri/src/hdc_bootstrap.rs) and the host half of the P5-A v2 handshake as written in
// PairingProtocol.java; a mistake in it can only make these tests fail, never pass.
#include "anhb_codec.h"
#include "anpu_codec.h"
#include "binding_store.h"
#include "crypto.h"
#include "hdc_bootstrap.h"
#include "p5c_control.h"
#include "stream_session.h"

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstring>
#include <deque>
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
    ok = ok && BindingStore::list().has_value() && found && sameBinding(*found, first) && listed() && BindingStore::save(second);
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
    void write(const float*, size_t frames) override {
        std::lock_guard<std::mutex> lock(mutex_);
        frames_ += frames;
    }
    PlaybackStats stats() const override { return {}; }
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
    PipeEnd& pc;
    uint64_t sessionId = 0x0102030405060708ULL;
    Key32 h2rControl{}, r2hControl{};
    uint64_t command = 0;

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
        if (!pc.writeAll(encodeAnpu(AnpuType::Handshake, hello))) return false;
        auto challenge = readAnpu(pc);
        if (!challenge || challenge->body.size() != 104 || challenge->body[5] != 2) return false;

        Bytes transcript = {'A', 'N', 'P', '5', 2, 1, 2};
        append(transcript, derived.hostId());
        append(transcript, derived.receiverId());
        append(transcript, hostNonce);
        append(transcript, std::span<const uint8_t>(challenge->body).subspan(24, 32));
        for (int shift = 56; shift >= 0; shift -= 8) transcript.push_back(static_cast<uint8_t>(sessionId >> shift));
        append(transcript, descriptor);
        append(transcript, descriptor);
        Bytes proofInput;
        append(proofInput, ascii("AnPod P5-A proof host v2"));
        proofInput.push_back(0);
        append(proofInput, transcript);
        Bytes finish = {'A', 'N', 'P', '5', 2, 3, 0, 40};
        append(finish, hmacSha256(secret, proofInput));
        if (!pc.writeAll(encodeAnpu(AnpuType::Handshake, finish))) return false;
        auto ready = readAnpu(pc);
        if (!ready || ready->body.size() != 40 || ready->body[5] != 4) return false;

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
        return pc.writeAll(encodeAnpu(AnpuType::Control, frame));
    }
    std::optional<p5c::Frame> reply() {
        auto record = readAnpu(pc);
        if (!record || record->type != AnpuType::Control) return std::nullopt;
        return p5c::decodeRaw(record->body, r2hControl, p5c::kReceiverToHost, sessionId);
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
    // CAPABILITIES_QUERY with the PC's wired requirements; returns the 20-byte info payload.
    Bytes capabilities() {
        uint32_t required = 0x82 | p5c::kCapAudioAdbStream | p5c::kCapAudioV2 | p5c::kCapAudioFloat32 |
                            p5c::kCapTerminalCause;
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
        {"session_user_stop_sends_result_user_stopped", sessionUserStopSendsResultUserStopped},
        {"session_rejects_replayed_command", sessionRejectsReplayedCommand},
        {"session_rejects_non_float32_profile", sessionRejectsNonFloat32Profile},
    };
    for (const Case& c : cases)
        if (!c.fn()) failures.emplace_back(c.name);
}

}  // namespace anpod
