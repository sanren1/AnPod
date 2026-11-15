// Tablet ADB-stream session on the hdc rport stream: port of the adbStream branch of
// AoaReceiverCoordinator.java (authenticate + controlLoop). Peer: AoaOwner::connect_adb_stream in
// src-tauri/src/aoa_owner.rs. ANPU records carry the P5-A v2 handshake (on the ADB-stream derived binding),
// HMAC'd P5-C control frames (strictly sequential: any replay or gap ends the session) and ANPD audio.
// Wired audio is Float32 only (profiles 4 / 5); any other negotiated format is rejected at capabilities.
#pragma once

#include "anpd_ingress.h"
#include "anpu_codec.h"
#include "byte_reader.h"
#include "p5c_control.h"
#include "pairing_protocol.h"

#include <atomic>
#include <condition_variable>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>

namespace anpod {

using PlaybackStats = p5c::ProgressBase;

class PlaybackSink {
public:
    virtual ~PlaybackSink() = default;
    // Opens exactly this format; false when the output cannot (no fallback to another format or mode).
    virtual bool open(const AudioFormatV2& format) = 0;
    // Called on the session reader thread; must not block. false: the frames were dropped (counted by the ingress).
    virtual bool write(const float* interleaved, size_t frames) = 0;
    // P5-C PAUSE / RESUME after an open (AoaReceiverCoordinator calls playback.pause / resume).
    virtual void pause() = 0;
    virtual void resume() = 0;
    // STOP or session end after an open; called once, on the reader thread.
    virtual void close() = 0;
    // Why the last open() failed, when the sink knows more than "cannot open" (e.g. "low latency unsupported").
    virtual std::string failureReason() const { return {}; }
    // playbackAdvancing == true reports the STREAMING state.
    virtual PlaybackStats stats() const = 0;
};

// P5-C receiver states (wire values).
enum class SessionState : uint8_t {
    Authenticated = 0,
    TransportConnected = 1,
    ReceiverRunning = 2,
    Streaming = 3,
    Paused = 4,
    Stopped = 5
};

// All callbacks run on the session reader thread; they must not destroy the session.
struct SessionCallbacks {
    std::function<void(SessionState)> onState;
    std::function<void(const AudioFormatV2&)> onFormat;  // after the sink opened the negotiated format
    std::function<void(const std::string& error)> onEnded;  // exactly once; empty after a clean stop
};

class StreamSession {
public:
    static std::unique_ptr<StreamSession> start(std::unique_ptr<ByteStream> stream, BindingMaterial saved,
                                                PlaybackSink& sink, SessionCallbacks callbacks);
    static std::unique_ptr<StreamSession> startWireless(int acceptedTcpFd, BindingMaterial saved,
                                                        std::optional<Id16> intentNonce, PlaybackSink& sink,
                                                        SessionCallbacks callbacks);
    // Stops (as stop(false)) and joins the session threads.
    ~StreamSession();
    StreamSession(const StreamSession&) = delete;
    StreamSession& operator=(const StreamSession&) = delete;

    // Non-blocking. userInitiated: the ACK of the PC's next command carries RESULT_USER_STOPPED (the PC then
    // drops its reconnect intent); the stream is closed anyway after the 5 s heartbeat timeout. Otherwise closes now.
    void stop(bool userInitiated);

    // Test seam: the UDP socket opened by a wireless BIND, -1 before.
    int udpSocketForTest() const { return udpFd_; }

private:
    StreamSession(std::unique_ptr<ByteStream> stream, BindingMaterial derived, PlaybackSink& sink,
                  SessionCallbacks callbacks);
    void launch();
    std::optional<AnpuRecord> readRecord(AnpuType expected, size_t wirelessHandshakeLength = 0);
    uint16_t bindUdp(uint16_t hostPort);
    void receiveUdp();
    int wirelessFd_ = -1;  // owned by stream_
    std::atomic<int> udpFd_{-1};  // opened and closed by the reader
    std::optional<Id16> intentNonce_;
    std::atomic<bool> udpStopping_{false};
    std::thread udpThread_;
    std::mutex udpOutputMutex_;
    bool udpPaused_ = false;
    std::string runReader();
    std::string authenticate();
    std::string controlLoop();
    bool writeRecord(AnpuType type, std::span<const uint8_t> body);
    void end(const std::string& message);
    void watchdog();

    std::unique_ptr<ByteStream> stream_;
    BindingMaterial derived_;
    PlaybackSink& sink_;
    const SessionCallbacks callbacks_;
    SessionKeys keys_;
    AudioFormatV2 format_;
    uint64_t sessionId_ = 0;
    bool playbackStarted_ = false;  // reader thread only
    std::unique_ptr<AnpdIngress> ingress_;

    std::mutex mutex_;
    std::condition_variable wake_;
    bool ended_ = false;
    std::string endMessage_;
    int64_t userStopAt_ = 0;  // steady ns, 0 = none
    std::atomic<bool> authenticated_{false};
    std::atomic<bool> userStopRequested_{false};
    std::atomic<int64_t> startupDeadline_{0};
    std::atomic<int64_t> lastControlAt_{0};
    std::thread reader_;
    std::thread watchdog_;
};

}  // namespace anpod
