#include "stream_session.h"

#include <chrono>
#include <cstdint>
#include <limits>

namespace anpod {
namespace {

constexpr int64_t kHelloTimeoutNanos = 30'000'000'000;   // AoaReceiverCoordinator helloTimeoutMillis
constexpr int64_t kFinishTimeoutNanos = 3'000'000'000;   // FINISH_TIMEOUT_NANOS
constexpr int64_t kHeartbeatTimeoutNanos = 5'000'000'000;  // HEARTBEAT_TIMEOUT_NANOS
// The PC sends a command at least every second (heartbeat), so the terminal ACK normally goes out at once;
// the grace matches the heartbeat timeout, after which the stream is closed without it.
constexpr int64_t kUserStopGraceNanos = kHeartbeatTimeoutNanos;
constexpr uint8_t kResultOk = 0, kResultRejected = 1;
constexpr uint32_t kVersionCode = 1000000;  // AppScope/app.json5 versionCode

int64_t nowNanos() {
    return std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now().time_since_epoch())
        .count();
}

bool float32Wired(const AudioFormatV2& f) { return (f.profile == 4 || f.profile == 5) && f.sampleFormat == 3; }

uint32_t readU32(std::span<const uint8_t> b, size_t at) {
    return uint32_t{b[at]} << 24 | uint32_t{b[at + 1]} << 16 | uint32_t{b[at + 2]} << 8 | b[at + 3];
}

}  // namespace

std::unique_ptr<StreamSession> StreamSession::start(std::unique_ptr<ByteStream> stream, BindingMaterial saved,
                                                     PlaybackSink& sink, SessionCallbacks callbacks) {
    std::unique_ptr<StreamSession> session(
        new StreamSession(std::move(stream), saved.deriveAdbStream(), sink, std::move(callbacks)));
    session->startupDeadline_ = nowNanos() + kHelloTimeoutNanos;
    StreamSession* self = session.get();
    session->reader_ = std::thread([self] {
        std::string error = self->runReader();
        self->end(error);
        if (self->playbackStarted_) self->sink_.close();
        std::string message;
        {
            std::lock_guard<std::mutex> lock(self->mutex_);
            message = self->endMessage_;
        }
        if (self->callbacks_.onEnded) self->callbacks_.onEnded(message);
    });
    session->watchdog_ = std::thread([self] { self->watchdog(); });
    return session;
}

StreamSession::StreamSession(std::unique_ptr<ByteStream> stream, BindingMaterial derived, PlaybackSink& sink,
                             SessionCallbacks callbacks)
    : stream_(std::move(stream)), derived_(std::move(derived)), sink_(sink), callbacks_(std::move(callbacks)) {}

StreamSession::~StreamSession() {
    stop(false);
    if (reader_.joinable()) reader_.join();
    if (watchdog_.joinable()) watchdog_.join();
}

void StreamSession::stop(bool userInitiated) {
    if (userInitiated && authenticated_) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!ended_ && userStopAt_ == 0) {
            userStopAt_ = nowNanos();
            userStopRequested_ = true;
        }
        return;
    }
    end("");
}

// First caller wins the end message; closing the stream unblocks the reader.
void StreamSession::end(const std::string& message) {
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (ended_) return;
        ended_ = true;
        endMessage_ = message;
    }
    stream_->close();
    wake_.notify_all();
}

void StreamSession::watchdog() {
    std::unique_lock<std::mutex> lock(mutex_);
    while (!ended_) {
        wake_.wait_for(lock, std::chrono::milliseconds(100));
        if (ended_) return;
        int64_t now = nowNanos();
        const char* expired = nullptr;
        if (userStopAt_ != 0 && now - userStopAt_ >= kUserStopGraceNanos) expired = "";
        else if (!authenticated_ && now >= startupDeadline_) expired = "session startup timeout";
        else if (authenticated_ && now - lastControlAt_ >= kHeartbeatTimeoutNanos) expired = "session heartbeat timeout";
        if (expired) {
            lock.unlock();
            end(expired);
            return;
        }
    }
}

bool StreamSession::writeRecord(AnpuType type, std::span<const uint8_t> body) {
    Bytes record = encodeAnpu(type, body);
    return !record.empty() && stream_->writeAll(record);
}

std::string StreamSession::runReader() {
    std::string error = authenticate();
    if (!error.empty()) return error;
    lastControlAt_ = nowNanos();
    authenticated_ = true;
    if (callbacks_.onState) callbacks_.onState(SessionState::Authenticated);
    return controlLoop();
}

std::string StreamSession::authenticate() {
    auto hello = readAnpu(*stream_);
    if (!hello || hello->type != AnpuType::Handshake) return "expected the P5-A hello record";
    startupDeadline_ = nowNanos() + kFinishTimeoutNanos;
    auto started = v2::acceptHello(derived_, hello->body);
    derived_ = BindingMaterial(Id16{}, Id16{}, Key32{});  // acceptHello keeps its own copy
    auto* awaiting = std::get_if<v2::ReceiverAwaitFinish>(&started);
    if (!awaiting) return "P5-A hello rejected";
    if (!writeRecord(AnpuType::Handshake, awaiting->challenge())) return "P5-A challenge write failed";
    auto finish = readAnpu(*stream_);
    if (!finish || finish->type != AnpuType::Handshake) return "expected the P5-A finish record";
    auto finished = awaiting->acceptFinish(finish->body);
    auto* done = std::get_if<v2::FinishResult>(&finished);
    if (!done) return "P5-A finish rejected";
    keys_ = done->keys;
    format_ = done->format;
    sessionId_ = done->sessionId;
    if (!writeRecord(AnpuType::Handshake, done->ready)) return "P5-A ready write failed";
    return "";
}

std::string StreamSession::controlLoop() {
    uint64_t expectedCommand = 0;
    uint64_t ackSequence = 0;
    bool capabilitiesAccepted = false, bound = false, modeSelected = false;
    bool& playbackStarted = playbackStarted_;
    SessionState state = SessionState::Authenticated;
    auto setState = [&](SessionState next) {
        if (next == state) return;
        state = next;
        if (callbacks_.onState) callbacks_.onState(next);
    };
    for (;;) {
        auto record = readAnpu(*stream_);
        if (!record) return "session stream closed";
        if (record->type == AnpuType::Audio) {
            if (!playbackStarted || !ingress_ || state == SessionState::Stopped) return "audio arrived before START";
            auto result = ingress_->accept(record->body);
            if (result != AnpdIngress::Result::Format && result != AnpdIngress::Result::Audio &&
                result != AnpdIngress::Result::LossLedger)
                return "authenticated audio rejected";
            continue;
        }
        if (record->type != AnpuType::Control) return "unexpected record type";
        auto command = p5c::decodeRaw(record->body, keys_.hostToReceiverControl, p5c::kHostToReceiver, sessionId_);
        if (!command) return "control frame rejected";
        // Replay protection: commands must arrive exactly in sequence (decodeRaw only authenticates).
        if (command->sequence != expectedCommand ||
            expectedCommand == static_cast<uint64_t>(std::numeric_limits<int64_t>::max()))
            return "control replay or sequence gap";
        expectedCommand++;
        lastControlAt_ = nowNanos();
        const Bytes& payload = command->payload;

        if (!capabilitiesAccepted) {
            if (command->type != p5c::kCapabilitiesQuery || payload.size() != 8 || payload[3] != 0)
                return "capability query must be first";
            uint16_t requestedControl = static_cast<uint16_t>(payload[0] << 8 | payload[1]);
            uint8_t requestedAudio = payload[2];
            uint32_t required = readU32(payload, 4);
            bool supportedFormat = float32Wired(format_);
            uint32_t supported = p5c::kCapAuthenticatedStatus | p5c::kCapPlaybackProgress | p5c::kCapTerminalCause |
                                 (supportedFormat ? p5c::kCapAudioV2 | p5c::kCapAudioFloat32 | p5c::kCapAudioAdbStream
                                                  : 0);
            constexpr uint32_t kWiredRequired = p5c::kCapAudioAdbStream | p5c::kCapAudioV2 | p5c::kCapAudioFloat32;
            bool compatible = requestedControl == p5c::kManagementControlMajor &&
                              requestedAudio == p5c::kAudioProtocolMajor && (required & ~supported) == 0 &&
                              supportedFormat && (required & kWiredRequired) == kWiredRequired;
            Bytes info = p5c::capabilitiesInfoPayload(command->sequence, p5c::kManagementControlMajor,
                                                      p5c::kAudioProtocolMajor,
                                                      compatible ? kResultOk : kResultRejected, supported, kVersionCode);
            if (!writeRecord(AnpuType::Control, p5c::encode(keys_.receiverToHostControl, p5c::kReceiverToHost,
                                                            sessionId_, ackSequence++, p5c::kCapabilitiesInfo, info)))
                return "control write failed";
            if (!supportedFormat)
                return "wired audio needs a Float32 profile (HiFi or Hi-Res), negotiated profile " +
                       std::to_string(format_.profile);
            if (!compatible) return "capabilities rejected";
            capabilitiesAccepted = true;
            continue;
        }

        Bytes extra;
        uint8_t result = kResultOk;
        bool terminate = false;
        if (userStopRequested_) {
            result = p5c::kResultUserStopped;
            setState(SessionState::Stopped);
            terminate = true;
        } else {
            switch (command->type) {
            case p5c::kBind:
                if (bound || payload.size() != 2 || payload[0] != 0 || payload[1] != 0) return "BIND must contain zero port";
                bound = true;
                setState(SessionState::TransportConnected);
                break;
            case p5c::kSelectMode:
                if (state != SessionState::TransportConnected || !bound || payload.size() != 1 ||
                    payload[0] != format_.mode)
                    return "selected mode differs from descriptor";
                modeSelected = true;
                break;
            case p5c::kStart:
                if (!bound || !modeSelected || !payload.empty() || playbackStarted) return "invalid START state";
                ingress_ = AnpdIngress::create(sessionId_, keys_.hostToReceiverAudio, format_,
                                               [this](uint64_t, std::span<const float> samples, uint32_t frames) {
                                                   return sink_.write(samples.data(), frames);
                                               });
                if (!ingress_) return "audio ingress refused the negotiated format";
                if (!sink_.open(format_))
                    return "audio output cannot open " + std::to_string(format_.sampleRate) + " Hz Float32 " +
                           std::to_string(format_.channels) + "ch " + (format_.mode == 1 ? "fast" : "normal");
                playbackStarted = true;
                if (callbacks_.onFormat) callbacks_.onFormat(format_);
                setState(SessionState::ReceiverRunning);
                break;
            case p5c::kPause:
                if (!payload.empty() || !playbackStarted ||
                    (state != SessionState::ReceiverRunning && state != SessionState::Streaming &&
                     state != SessionState::Paused))
                    return "invalid PAUSE state";
                if (state != SessionState::Paused) sink_.pause();
                setState(SessionState::Paused);
                break;
            case p5c::kResume:
                if (!payload.empty() || !playbackStarted ||
                    (state != SessionState::Paused && state != SessionState::ReceiverRunning &&
                     state != SessionState::Streaming))
                    return "invalid RESUME state";
                if (state == SessionState::Paused) sink_.resume();
                setState(SessionState::ReceiverRunning);
                break;
            case p5c::kHeartbeat: {
                if (!payload.empty()) return "invalid HEARTBEAT";
                bool streaming = playbackStarted && sink_.stats().playbackAdvancing.value_or(false);
                if (playbackStarted && state == SessionState::ReceiverRunning && streaming)
                    setState(SessionState::Streaming);
                else if (state == SessionState::Streaming && !streaming)
                    setState(SessionState::ReceiverRunning);
                break;
            }
            case p5c::kPlaybackProgressQuery: {
                // Only the base v2 schema: CAP_PROGRESS_EXT is not advertised.
                if (!payload.empty()) return "invalid progress query";
                PlaybackStats stats = sink_.stats();
                extra = p5c::playbackProgressPayloadV2(p5c::playbackProgressPayload(stats), playbackStarted ? 3 : 0,
                                                       playbackStarted ? format_.channels : 0,
                                                       playbackStarted ? static_cast<int32_t>(format_.sampleRate) : 0);
                if (extra.empty()) return "progress payload unavailable";
                break;
            }
            case p5c::kStop:
                // A restart request (payload 1) needs CAP_AOA_SESSION_RESTART, never advertised on this transport.
                if (!payload.empty()) return "invalid STOP";
                setState(SessionState::Stopped);
                terminate = true;
                break;
            default:
                return "unsupported control command";
            }
        }
        Bytes ack = p5c::encode(keys_.receiverToHostControl, p5c::kReceiverToHost, sessionId_, ackSequence++, p5c::kAck,
                                p5c::ackPayload(command->type, command->sequence, result,
                                                static_cast<uint8_t>(state), 0, extra));
        if (!writeRecord(AnpuType::Control, ack)) return "control write failed";
        if (terminate) return "";
    }
}

}  // namespace anpod
