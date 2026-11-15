#include "p5c_control.h"

#include <cstring>

namespace anpod::p5c {
namespace {

void putInt(Bytes& b, size_t at, uint32_t v) {
    for (int i = 3; i >= 0; i--) {
        b[at + i] = static_cast<uint8_t>(v);
        v >>= 8;
    }
}
void putLong(Bytes& b, size_t at, uint64_t v) {
    for (int i = 7; i >= 0; i--) {
        b[at + i] = static_cast<uint8_t>(v);
        v >>= 8;
    }
}
uint32_t getInt(std::span<const uint8_t> b, size_t at) {
    return (uint32_t{b[at]} << 24) | (uint32_t{b[at + 1]} << 16) | (uint32_t{b[at + 2]} << 8) | b[at + 3];
}
uint64_t getLong(std::span<const uint8_t> b, size_t at) {
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) v = (v << 8) | b[at + i];
    return v;
}
Bytes grow(std::span<const uint8_t> src, size_t extra) {
    Bytes out(src.begin(), src.end());
    out.resize(src.size() + extra, 0);
    return out;
}

// Java writes (int) -1 for an unknown 32-bit value.
uint32_t u32OrUnknown(int64_t v) { return v < 0 ? 0xffffffffu : static_cast<uint32_t>(v); }

void putLossWindow(Bytes& b, size_t at, const std::optional<LossWindow>& w) {
    putLong(b, at, static_cast<uint64_t>(w ? w->lossFrames : -1));
    putLong(b, at + 8, static_cast<uint64_t>(w ? w->eligibleFrames : -1));
}

}  // namespace

Bytes encode(const Key32& key, uint32_t direction, uint64_t sessionId, uint64_t sequence, uint8_t type,
             std::span<const uint8_t> payload) {
    if (payload.size() > kMaxFrameBytes - kHeaderBytes - kTagBytes) return {};
    Bytes frame(kHeaderBytes + payload.size() + kTagBytes, 0);
    frame[0] = 'A';
    frame[1] = 'N';
    frame[2] = 'P';
    frame[3] = 'C';
    frame[4] = 1;
    frame[5] = type;
    putInt(frame, 8, direction);
    putLong(frame, 12, sessionId);
    putLong(frame, 20, sequence);
    frame[28] = static_cast<uint8_t>(payload.size() >> 8);
    frame[29] = static_cast<uint8_t>(payload.size());
    if (!payload.empty()) std::memcpy(frame.data() + kHeaderBytes, payload.data(), payload.size());
    Digest tag = hmacSha256(key, std::span<const uint8_t>(frame.data(), kHeaderBytes + payload.size()));
    std::memcpy(frame.data() + kHeaderBytes + payload.size(), tag.data(), kTagBytes);
    secureZero(tag.data(), tag.size());
    return frame;
}

std::optional<Frame> decodeRaw(std::span<const uint8_t> frame, const Key32& key, uint32_t direction,
                               uint64_t sessionId) {
    if (frame.size() < kHeaderBytes + kTagBytes || frame.size() > kMaxFrameBytes) return std::nullopt;
    size_t payloadLength = (static_cast<size_t>(frame[28]) << 8) | frame[29];
    if (frame[0] != 'A' || frame[1] != 'N' || frame[2] != 'P' || frame[3] != 'C' || frame[4] != 1 ||
        frame[6] != 0 || frame[7] != 0 || getInt(frame, 8) != direction || getLong(frame, 12) != sessionId ||
        frame.size() != kHeaderBytes + payloadLength + kTagBytes)
        return std::nullopt;
    Digest expected = hmacSha256(key, frame.first(kHeaderBytes + payloadLength));
    bool ok = constantTimeEqual(expected, frame.subspan(kHeaderBytes + payloadLength, kTagBytes));
    secureZero(expected.data(), expected.size());
    if (!ok) return std::nullopt;
    return Frame{frame[5], getLong(frame, 20),
                 Bytes(frame.begin() + kHeaderBytes, frame.begin() + kHeaderBytes + payloadLength)};
}

Bytes ackPayload(uint8_t commandType, uint64_t commandSequence, uint8_t result, uint8_t state, uint16_t udpPort,
                 std::span<const uint8_t> extra) {
    Bytes p(13, 0);
    p[0] = commandType;
    putLong(p, 1, commandSequence);
    p[9] = result;
    p[10] = state;
    p[11] = static_cast<uint8_t>(udpPort >> 8);
    p[12] = static_cast<uint8_t>(udpPort);
    p.insert(p.end(), extra.begin(), extra.end());
    return p;
}

Bytes backgroundAuthorizationAckExtra(std::span<const uint8_t> requestId, uint8_t status) {
    if (requestId.size() != 16 || status > 6) return {};  // AUTH_PENDING..AUTH_CANCELLED
    Bytes extra(requestId.begin(), requestId.end());
    extra.push_back(status);
    return extra;
}

bool validBackgroundTrust(uint8_t kind, uint8_t persistence, int64_t remainingMs) {
    switch (kind) {
        case 0: return persistence == 0 && remainingMs == 0;
        case 1: return persistence == 1 && ((remainingMs >= 0 && remainingMs <= 30000) || remainingMs == -1);
        case 2: return persistence == 1 && remainingMs >= 0 && remainingMs <= 8LL * 3600 * 1000;
        case 3: return persistence == 2 && remainingMs == -1;
        default: return false;
    }
}

Bytes backgroundTrustAckExtra(std::span<const uint8_t> commitment, uint8_t kind, uint8_t persistence,
                              int64_t remainingMs) {
    if (commitment.size() != 32 || !validBackgroundTrust(kind, persistence, remainingMs)) return {};
    Bytes extra(commitment.begin(), commitment.end());
    extra.resize(42, 0);
    extra[32] = kind;
    extra[33] = persistence;
    putLong(extra, 34, static_cast<uint64_t>(remainingMs));
    return extra;
}

Bytes playbackProgressPayload(const ProgressBase& p) {
    Bytes b(70, 0);
    putInt(b, 0, p.sampleRate);
    b[4] = p.channels;
    putInt(b, 5, p.jitterTargetFrames);
    putLong(b, 9, static_cast<uint64_t>(p.expiredFrames));
    putLong(b, 17, static_cast<uint64_t>(p.queuedFrames));
    putLong(b, 25, static_cast<uint64_t>(p.pendingFrames));
    putInt(b, 33, p.underrunCount);
    putLong(b, 37, static_cast<uint64_t>(p.tcpDroppedFrames));
    b[45] = p.playbackAdvancing ? (*p.playbackAdvancing ? 1 : 0) : 0xff;
    putInt(b, 46, p.requestedFrames);
    putInt(b, 50, p.acceptedFrames);
    putInt(b, 54, p.effectiveFrames);
    putInt(b, 58, p.capacityFrames);
    putInt(b, 62, p.startThresholdFrames);
    putInt(b, 66, p.preRollFrames);
    return b;
}

Bytes playbackProgressPayloadV2(std::span<const uint8_t> legacy, uint8_t openedTrackFormat,
                                uint8_t openedTrackChannels, int32_t openedTrackSampleRate) {
    if (legacy.size() != 70 || openedTrackFormat > 3 || openedTrackChannels > 2 || openedTrackSampleRate < 0)
        return {};
    Bytes b = grow(legacy, 8);
    b[70] = 1;  // schema
    b[71] = openedTrackFormat;
    b[72] = openedTrackChannels;
    b[73] = 0;  // reserved
    putInt(b, 74, static_cast<uint32_t>(openedTrackSampleRate));
    return b;
}

Bytes playbackProgressPayloadExtended(std::span<const uint8_t> base, int64_t monotonicNanos,
                                      int64_t receiverGeneration, int32_t outputEpoch, uint8_t outputEngine,
                                      const std::array<int64_t, 12>& counters, int32_t ringPendingFrames,
                                      int32_t streamPendingFrames) {
    if ((base.size() != 70 && base.size() != 78) || outputEngine > 2) return {};
    Bytes b = grow(base, 126);
    size_t o = base.size();
    b[o] = 2;
    putLong(b, o + 1, static_cast<uint64_t>(monotonicNanos));
    putLong(b, o + 9, static_cast<uint64_t>(receiverGeneration));
    putInt(b, o + 17, static_cast<uint32_t>(outputEpoch));
    b[o + 21] = outputEngine;
    for (size_t i = 0; i < counters.size(); i++) putLong(b, o + 22 + i * 8, static_cast<uint64_t>(counters[i]));
    putInt(b, o + 118, static_cast<uint32_t>(ringPendingFrames));
    putInt(b, o + 122, static_cast<uint32_t>(streamPendingFrames));
    return b;
}

Bytes playbackProgressPayloadSchema3(std::span<const uint8_t> schema2, int64_t targetFrames,
                                     int64_t observedFrames, int32_t errorFrames, int32_t correctionPpm,
                                     uint8_t resetReason) {
    if ((schema2.size() != 196 && schema2.size() != 204) || targetFrames < -1 || targetFrames > 0xffffffffLL ||
        observedFrames < -1 || observedFrames > 0xffffffffLL)
        return {};
    Bytes b = grow(schema2, 17);
    b[schema2.size() - 126] = 3;
    size_t o = schema2.size();
    putInt(b, o, u32OrUnknown(targetFrames));
    putInt(b, o + 4, u32OrUnknown(observedFrames));
    putInt(b, o + 8, static_cast<uint32_t>(errorFrames));
    putInt(b, o + 12, static_cast<uint32_t>(correctionPpm));
    b[o + 16] = resetReason;
    return b;
}

Bytes playbackProgressPayloadSchema4(std::span<const uint8_t> schema3, int64_t appliedLedgerRevision) {
    if (schema3.size() != 213 && schema3.size() != 221) return {};
    size_t ext = schema3.size() - 143;
    if (schema3[ext] != 3) return {};
    Bytes b = grow(schema3, 8);
    b[ext] = 4;
    putLong(b, schema3.size(), static_cast<uint64_t>(appliedLedgerRevision));
    return b;
}

Bytes playbackProgressPayloadSchema5(std::span<const uint8_t> schema4, const std::optional<LossWindow>& tenSeconds,
                                     const std::optional<LossWindow>& sixtySeconds,
                                     const std::optional<LossWindow>& cumulative) {
    if (schema4.size() != 221 && schema4.size() != 229) return {};
    size_t ext = schema4.size() - 151;
    if (schema4[ext] != 4) return {};
    Bytes b = grow(schema4, 48);
    b[ext] = 5;
    putLossWindow(b, schema4.size(), tenSeconds);
    putLossWindow(b, schema4.size() + 16, sixtySeconds);
    putLossWindow(b, schema4.size() + 32, cumulative);
    return b;
}

Bytes playbackProgressPayloadSchema6(std::span<const uint8_t> schema5, int64_t pcDropFrames,
                                     int64_t receiverDropFrames, int64_t transportMissingFrames,
                                     int64_t outputDropFrames, int64_t recoveryCount, int64_t recoveryDurationMs) {
    if (schema5.size() != 269 && schema5.size() != 277) return {};
    size_t ext = schema5.size() - 199;
    if (schema5[ext] != 5) return {};
    Bytes b = grow(schema5, 48);
    b[ext] = 6;
    const int64_t v[6] = {pcDropFrames, receiverDropFrames, transportMissingFrames,
                          outputDropFrames, recoveryCount, recoveryDurationMs};
    for (size_t i = 0; i < 6; i++) putLong(b, schema5.size() + i * 8, static_cast<uint64_t>(v[i]));
    return b;
}

Bytes capabilitiesQueryPayload(uint16_t controlMajor, uint8_t audioMajor, uint32_t requiredCapabilities) {
    Bytes p(8, 0);
    p[0] = static_cast<uint8_t>(controlMajor >> 8);
    p[1] = static_cast<uint8_t>(controlMajor);
    p[2] = audioMajor;
    putInt(p, 4, requiredCapabilities);
    return p;
}

Bytes capabilitiesInfoPayload(uint64_t querySequence, uint16_t controlMajor, uint8_t audioMajor, uint8_t result,
                              uint32_t capabilities, uint32_t versionCode) {
    Bytes p(20, 0);
    putLong(p, 0, querySequence);
    p[8] = static_cast<uint8_t>(controlMajor >> 8);
    p[9] = static_cast<uint8_t>(controlMajor);
    p[10] = audioMajor;
    p[11] = result;
    putInt(p, 12, capabilities);
    putInt(p, 16, versionCode);
    return p;
}

}  // namespace anpod::p5c
