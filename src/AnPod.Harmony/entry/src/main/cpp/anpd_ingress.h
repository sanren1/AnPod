// Port of the V2 Float32 path of UdpPacketDecoder.java, AnpdAudioIngress.java (record stream) and
// AnpdLossLedger.java. ANPD datagram: "ANPD" | 2 | type | 0 0 | sessionId u64 | sequence u64 |
// sampleFrameIndex u64 | payload length u16 | AES-256-GCM(payload) | tag[16]; the 34 header bytes are the AAD
// and the nonce is 0x414e5048 | sequence (big-endian). Only profiles with Float32 samples are accepted.
#pragma once

#include "aod_sampler.h"
#include "crypto.h"
#include "pairing_protocol.h"

#include <atomic>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <vector>

namespace anpod {

inline constexpr size_t kAnpdHeaderBytes = 34;
inline constexpr size_t kAnpdTagBytes = 16;
inline constexpr size_t kAnpdMaxDatagramBytes = 1200;
inline constexpr size_t kAnpdMaxPlaintextBytes = kAnpdMaxDatagramBytes - kAnpdHeaderBytes - kAnpdTagBytes;
inline constexpr uint8_t kAnpdFormat = 1, kAnpdAudio = 2, kAnpdLossLedger = 3;

struct LossInterval {
    uint64_t start = 0;
    uint64_t frameCount = 0;
};

// Authenticated source-drop snapshot payload (version 2 or 3).
struct LossLedger {
    static constexpr int kVersion = 3;
    static constexpr size_t kMaxIntervals = 64;

    uint64_t periodEpoch = 0, revision = 0, captureTail = 0, settledThrough = 0;
    uint64_t sourceSegmentGeneration = 0, sourceSegmentStart = 0;
    bool hasSourceSegmentStart = false;
    bool complete = false;
    std::vector<LossInterval> intervals;

    static std::optional<LossLedger> decode(std::span<const uint8_t> payload);
    Bytes encode() const;  // version 3
};

enum class LedgerApply { Ignored, Applied, EpochChanged, Invalid };

// Applies snapshots in order and keeps what the receiver may report back. "Unknown" is nullopt.
class LossLedgerStore {
public:
    LedgerApply apply(const LossLedger& next);
    std::optional<uint64_t> periodEpoch() const;
    std::optional<uint64_t> appliedRevision() const;
    std::optional<uint64_t> captureTail() const;
    std::optional<uint64_t> settledThrough() const;
    std::optional<uint64_t> sourceSegmentGeneration() const;
    std::optional<uint64_t> sourceSegmentStart() const;
    bool complete() const;
    std::vector<LossInterval> intervals() const;

private:
    void take(const LossLedger& next);
    mutable std::mutex mutex_;
    LossLedger current_;
    bool hasRevision_ = false;
    bool complete_ = true;  // every snapshot so far was complete
    bool lastSnapshotComplete_ = false;
};

class AnpdIngress {
public:
    enum class Result { Format, Audio, LossLedger, Identity, Authentication, Replay, FormatRejected, HeaderRejected };
    // Receives each authenticated, finite Float32 packet: (sampleFrameIndex, interleaved samples, frames).
    // false means the consumer dropped it (counted in droppedFrames()).
    using FloatSink = std::function<bool(uint64_t, std::span<const float>, uint32_t)>;

    // nullptr unless the locked format carries Float32 samples (v2 sampleFormat 3) and the sink is set.
    static std::unique_ptr<AnpdIngress> create(uint64_t sessionId, const Key32& hostToReceiverAudioKey,
                                               const AudioFormatV2& format, FloatSink sink);
    ~AnpdIngress();
    AnpdIngress(const AnpdIngress&) = delete;
    AnpdIngress& operator=(const AnpdIngress&) = delete;

    // One reader thread: accept() calls the FloatSink synchronously on that same thread, so the sink must not
    // block (write into a ring and return). The counters below are safe to read from other threads.
    Result accept(std::span<const uint8_t> packet);

    AodSnapshot aodSnapshot(int64_t now) { return sampler_.snapshot(now, authenticatedAudioPayloadBytes_); }
    uint64_t authenticatedAudioFrames() const { return authenticatedAudioFrames_; }
    uint64_t authenticatedAudioPayloadBytes() const { return authenticatedAudioPayloadBytes_; }
    uint64_t droppedFrames() const { return droppedFrames_; }
    bool hasAuthenticatedAudio() const { return hasAuthenticatedAudio_; }
    // steady_clock nanoseconds of the last authenticated audio packet; meaningful once hasAuthenticatedAudio().
    int64_t lastAuthenticatedAudioAtNanos() const { return lastAuthenticatedAudioAtNanos_; }
    const LossLedgerStore& lossLedger() const { return ledger_; }

private:
    AnpdIngress(uint64_t sessionId, const Key32& key, const AudioFormatV2& format, FloatSink sink);
    bool wouldAccept(uint64_t sequence) const;
    void commit(uint64_t sequence);
    Result acceptFloat(uint8_t type, uint64_t sequence, uint64_t frameIndex, std::span<const uint8_t> payload);

    const uint64_t sessionId_;
    Key32 key_;
    const AudioFormatV2 format_;
    const FloatSink sink_;
    AodSampler sampler_;
    LossLedgerStore ledger_;
    bool formatSeen_ = false;
    bool hasHighest_ = false;
    uint64_t highest_ = 0;
    uint64_t replayBits_ = 0;
    std::atomic<bool> hasAuthenticatedAudio_{false};
    std::atomic<int64_t> lastAuthenticatedAudioAtNanos_{0};
    std::atomic<uint64_t> authenticatedAudioFrames_{0};
    std::atomic<uint64_t> authenticatedAudioPayloadBytes_{0};
    std::atomic<uint64_t> droppedFrames_{0};
};

}  // namespace anpod
