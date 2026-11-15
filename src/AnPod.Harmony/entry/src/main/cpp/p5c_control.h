// Port of P5cControlProtocol.java: fixed P5-C authenticated control frame shared with the Rust host
// (src-tauri/src/p5c/wire.rs). Frame: "ANPC" | 1 | type | 0 0 | direction u32 | sessionId u64 |
// sequence u64 | payload length u16 | payload | HMAC-SHA256 tag (all big-endian).
// The builders return an empty Bytes (the Java IllegalArgumentException) for out-of-range arguments.
#pragma once

#include "crypto.h"
#include "pairing_protocol.h"

#include <array>
#include <optional>

namespace anpod::p5c {

inline constexpr uint32_t kHostToReceiver = 0x414e5048;
inline constexpr uint32_t kReceiverToHost = 0x414e5052;
inline constexpr size_t kHeaderBytes = 30;
inline constexpr size_t kTagBytes = 32;
inline constexpr size_t kMaxFrameBytes = 1200;

inline constexpr uint8_t kBind = 1, kSelectMode = 2, kStart = 3, kPause = 4, kResume = 5, kStop = 6, kHeartbeat = 7,
                         kCapabilitiesQuery = 8, kCapabilitiesInfo = 9, kPlaybackProgressQuery = 16,
                         kConnectionIntent = 17, kBackgroundStartRequest = 18, kBackgroundStartStatus = 19,
                         kBackgroundStartCancel = 20, kBackgroundTrustQuery = 21, kBackgroundTrustRevoke = 22,
                         kAck = 127;
inline constexpr uint16_t kManagementControlMajor = 3;
inline constexpr uint8_t kAudioProtocolMajor = 1;

inline constexpr uint32_t kCapReceiverUi = 0x1, kCapAuthenticatedStatus = 0x2, kCapPlaybackProgress = 0x8,
                          kCapConnectionIntent = 0x10, kCapBackgroundStartAuth = 0x20,
                          kCapBackgroundTrustManagement = 0x40, kCapAudioV2 = 0x80, kCapTerminalCause = 0x100,
                          kCapAoaSessionRestart = 0x200, kCapAudioFloat32 = 0x400, kCapProgressExt = 0x800,
                          kCapAudioAdbStream = 0x1000, kCapBufferMetrics = 0x2000, kCapSourceFrameLoss = 0x4000,
                          kCapSourceLossDetails = 0x8000, kCapSourceSegmentStart = 0x10000;

// ACK result code (RESULT_USER_STOPPED in the Java coordinators).
inline constexpr uint8_t kResultUserStopped = 3;

struct Frame {
    uint8_t type = 0;
    uint64_t sequence = 0;
    Bytes payload;
};

// Empty when the payload is too large.
Bytes encode(const Key32& key, uint32_t direction, uint64_t sessionId, uint64_t sequence, uint8_t type,
             std::span<const uint8_t> payload);
// Authenticates one raw ANPC frame. nullopt on a bad length, identity (direction/session) or MAC.
std::optional<Frame> decodeRaw(std::span<const uint8_t> frame, const Key32& key, uint32_t direction,
                               uint64_t sessionId);

Bytes ackPayload(uint8_t commandType, uint64_t commandSequence, uint8_t result, uint8_t state, uint16_t udpPort,
                 std::span<const uint8_t> extra = {});
Bytes capabilitiesQueryPayload(uint16_t controlMajor, uint8_t audioMajor, uint32_t requiredCapabilities);
Bytes capabilitiesInfoPayload(uint64_t querySequence, uint16_t controlMajor, uint8_t audioMajor, uint8_t result,
                              uint32_t capabilities, uint32_t versionCode);

// Background authorization / trust ACK extras (empty on invalid input).
Bytes backgroundAuthorizationAckExtra(std::span<const uint8_t> requestId, uint8_t status);
Bytes backgroundTrustAckExtra(std::span<const uint8_t> commitment, uint8_t kind, uint8_t persistence,
                              int64_t remainingMs);
bool validBackgroundTrust(uint8_t kind, uint8_t persistence, int64_t remainingMs);

// Playback progress: base (70) -> v2 (78) -> schema2..6 extensions. "Unknown" is -1 / 0xff as in Java.
// Unknown is all ones on the wire (Java -1 / null), which the PC shows as unknown; a sink sets what it knows.
struct ProgressBase {
    uint32_t sampleRate = UINT32_MAX;
    uint8_t channels = 0xff;
    uint32_t jitterTargetFrames = UINT32_MAX;
    int64_t expiredFrames = -1, queuedFrames = -1, pendingFrames = -1;
    uint32_t underrunCount = UINT32_MAX;
    int64_t tcpDroppedFrames = -1;
    std::optional<bool> playbackAdvancing;  // nullopt -> 0xff
    uint32_t requestedFrames = UINT32_MAX, acceptedFrames = UINT32_MAX, effectiveFrames = UINT32_MAX,
             capacityFrames = UINT32_MAX, startThresholdFrames = UINT32_MAX, preRollFrames = UINT32_MAX;
};
struct LossWindow {
    int64_t lossFrames = 0;
    int64_t eligibleFrames = 0;
};

Bytes playbackProgressPayload(const ProgressBase& p);
Bytes playbackProgressPayloadV2(std::span<const uint8_t> legacy, uint8_t openedTrackFormat,
                                uint8_t openedTrackChannels, int32_t openedTrackSampleRate);
Bytes playbackProgressPayloadExtended(std::span<const uint8_t> base, int64_t monotonicNanos,
                                      int64_t receiverGeneration, int32_t outputEpoch, uint8_t outputEngine,
                                      const std::array<int64_t, 12>& counters, int32_t ringPendingFrames,
                                      int32_t streamPendingFrames);
Bytes playbackProgressPayloadSchema3(std::span<const uint8_t> schema2, int64_t targetFrames,
                                     int64_t observedFrames, int32_t errorFrames, int32_t correctionPpm,
                                     uint8_t resetReason);
Bytes playbackProgressPayloadSchema4(std::span<const uint8_t> schema3, int64_t appliedLedgerRevision);
Bytes playbackProgressPayloadSchema5(std::span<const uint8_t> schema4, const std::optional<LossWindow>& tenSeconds,
                                     const std::optional<LossWindow>& sixtySeconds,
                                     const std::optional<LossWindow>& cumulative);
Bytes playbackProgressPayloadSchema6(std::span<const uint8_t> schema5, int64_t pcDropFrames,
                                     int64_t receiverDropFrames, int64_t transportMissingFrames,
                                     int64_t outputDropFrames, int64_t recoveryCount, int64_t recoveryDurationMs);

}  // namespace anpod::p5c
