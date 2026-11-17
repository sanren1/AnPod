#include "anpd_ingress.h"

#include <array>
#include <chrono>
#include <cstring>

namespace anpod {
namespace {

constexpr size_t kV2LedgerHeader = 36;
constexpr size_t kV3LedgerHeader = 52;
constexpr uint32_t kNonceDomain = 0x414e5048;  // "ANPH"

uint64_t getU64(std::span<const uint8_t> b, size_t at) {
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) v = (v << 8) | b[at + i];
    return v;
}
void putU64(Bytes& b, uint64_t v) {
    for (int i = 7; i >= 0; i--) b.push_back(static_cast<uint8_t>(v >> (i * 8)));
}

bool coversInterval(const std::vector<LossInterval>& snapshot, const LossInterval& wanted) {
    uint64_t wantedEnd = wanted.start + wanted.frameCount;
    for (const LossInterval& i : snapshot) {
        if (i.start <= wanted.start && i.start + i.frameCount >= wantedEnd) return true;
        if (i.start > wanted.start) return false;
    }
    return false;
}

bool sameIntervals(const std::vector<LossInterval>& a, const std::vector<LossInterval>& b) {
    if (a.size() != b.size()) return false;
    for (size_t i = 0; i < a.size(); i++)
        if (a[i].start != b[i].start || a[i].frameCount != b[i].frameCount) return false;
    return true;
}

}  // namespace

std::optional<LossLedger> LossLedger::decode(std::span<const uint8_t> p) {
    if (p.size() < kV2LedgerHeader) return std::nullopt;
    int version = p[0];
    int flags = p[1];
    size_t count = p[2];
    if (version != 2 && version != kVersion) return std::nullopt;
    size_t headerBytes = version == 2 ? kV2LedgerHeader : kV3LedgerHeader;
    if ((flags & ~1) != 0 || count > kMaxIntervals || p[3] != 0 || p.size() != headerBytes + count * 16)
        return std::nullopt;
    LossLedger l;
    l.periodEpoch = getU64(p, 4);
    l.revision = getU64(p, 12);
    l.captureTail = getU64(p, 20);
    l.settledThrough = getU64(p, 28);
    if (l.settledThrough > l.captureTail) return std::nullopt;
    if (version == kVersion) {
        l.sourceSegmentGeneration = getU64(p, 36);
        l.sourceSegmentStart = getU64(p, 44);
        l.hasSourceSegmentStart = true;
        if (l.sourceSegmentStart > l.settledThrough || l.sourceSegmentStart > l.captureTail) return std::nullopt;
    }
    l.complete = (flags & 1) != 0;
    uint64_t previousEnd = 0;
    for (size_t i = 0; i < count; i++) {
        uint64_t start = getU64(p, headerBytes + i * 16);
        uint64_t frames = getU64(p, headerBytes + i * 16 + 8);
        uint64_t end = start + frames;
        if (frames == 0 || end <= start || end > l.captureTail || (i > 0 && start <= previousEnd))
            return std::nullopt;
        l.intervals.push_back({start, frames});
        previousEnd = end;
    }
    return l;
}

Bytes LossLedger::encode() const {
    Bytes out = {static_cast<uint8_t>(kVersion), static_cast<uint8_t>(complete ? 1 : 0),
                 static_cast<uint8_t>(intervals.size()), 0};
    for (uint64_t v : {periodEpoch, revision, captureTail, settledThrough, sourceSegmentGeneration, sourceSegmentStart})
        putU64(out, v);
    for (const LossInterval& i : intervals) {
        putU64(out, i.start);
        putU64(out, i.frameCount);
    }
    return out;
}

void LossLedgerStore::take(const LossLedger& next) {
    current_ = next;
    lastSnapshotComplete_ = next.complete;
}

LedgerApply LossLedgerStore::apply(const LossLedger& next) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!hasRevision_) {
        take(next);
        complete_ = next.complete;
        hasRevision_ = true;
        return LedgerApply::Applied;
    }
    const LossLedger& cur = current_;
    if (next.periodEpoch < cur.periodEpoch) return LedgerApply::Ignored;
    if (next.periodEpoch > cur.periodEpoch) {
        if (next.revision <= cur.revision || next.captureTail < cur.captureTail ||
            next.settledThrough < cur.settledThrough)
            return LedgerApply::Invalid;
        take(next);
        complete_ = next.complete;
        return LedgerApply::EpochChanged;
    }
    if (next.revision < cur.revision) return LedgerApply::Ignored;
    if (next.revision == cur.revision) {
        if (next.captureTail != cur.captureTail || next.settledThrough != cur.settledThrough ||
            next.sourceSegmentGeneration != cur.sourceSegmentGeneration ||
            next.sourceSegmentStart != cur.sourceSegmentStart ||
            next.hasSourceSegmentStart != cur.hasSourceSegmentStart || next.complete != lastSnapshotComplete_ ||
            !sameIntervals(next.intervals, cur.intervals))
            return LedgerApply::Invalid;
        return LedgerApply::Ignored;
    }
    if (next.captureTail < cur.captureTail || next.settledThrough < cur.settledThrough) return LedgerApply::Invalid;
    if (cur.hasSourceSegmentStart && next.hasSourceSegmentStart) {
        if (next.sourceSegmentGeneration < cur.sourceSegmentGeneration ||
            (next.sourceSegmentGeneration == cur.sourceSegmentGeneration &&
             next.sourceSegmentStart != cur.sourceSegmentStart) ||
            (next.sourceSegmentGeneration > cur.sourceSegmentGeneration &&
             next.sourceSegmentStart < cur.settledThrough))
            return LedgerApply::Invalid;
    }
    for (const LossInterval& previous : cur.intervals)
        if (!coversInterval(next.intervals, previous)) return LedgerApply::Invalid;
    take(next);
    complete_ = complete_ && next.complete;
    return LedgerApply::Applied;
}

std::optional<uint64_t> LossLedgerStore::periodEpoch() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ ? std::optional(current_.periodEpoch) : std::nullopt;
}
std::optional<uint64_t> LossLedgerStore::appliedRevision() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ ? std::optional(current_.revision) : std::nullopt;
}
std::optional<uint64_t> LossLedgerStore::captureTail() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ ? std::optional(current_.captureTail) : std::nullopt;
}
std::optional<uint64_t> LossLedgerStore::settledThrough() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ ? std::optional(current_.settledThrough) : std::nullopt;
}
std::optional<uint64_t> LossLedgerStore::sourceSegmentGeneration() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ ? std::optional(current_.sourceSegmentGeneration) : std::nullopt;
}
std::optional<uint64_t> LossLedgerStore::sourceSegmentStart() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ && current_.hasSourceSegmentStart ? std::optional(current_.sourceSegmentStart) : std::nullopt;
}
bool LossLedgerStore::complete() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hasRevision_ && complete_;
}
std::vector<LossInterval> LossLedgerStore::intervals() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return current_.intervals;
}

AnpdIngress::AnpdIngress(uint64_t sessionId, const Key32& key, const AudioFormatV2& format, FloatSink sink)
    : sessionId_(sessionId), key_(key), format_(format), sink_(std::move(sink)) {}

AnpdIngress::~AnpdIngress() { secureZero(key_.data(), key_.size()); }

std::unique_ptr<AnpdIngress> AnpdIngress::create(uint64_t sessionId, const Key32& key, const AudioFormatV2& format,
                                                 FloatSink sink) {
    if (format.sampleFormat != 3 || !sink) return nullptr;
    return std::unique_ptr<AnpdIngress>(new AnpdIngress(sessionId, key, format, std::move(sink)));
}

bool AnpdIngress::wouldAccept(uint64_t sequence) const {
    if (!hasHighest_ || sequence > highest_) return true;
    uint64_t behind = highest_ - sequence;
    return behind < 64 && (replayBits_ & (uint64_t{1} << behind)) == 0;
}

void AnpdIngress::commit(uint64_t sequence) {
    if (!hasHighest_) {
        hasHighest_ = true;
        highest_ = sequence;
        replayBits_ = 1;
    } else if (sequence > highest_) {
        uint64_t advance = sequence - highest_;
        replayBits_ = advance >= 64 ? 1 : (replayBits_ << advance) | 1;
        highest_ = sequence;
    } else if (highest_ - sequence < 64) {
        replayBits_ |= uint64_t{1} << (highest_ - sequence);
    }
}

AnpdIngress::Result AnpdIngress::accept(std::span<const uint8_t> d) {
    if (d.size() < kAnpdHeaderBytes + kAnpdTagBytes || d.size() > kAnpdMaxDatagramBytes) return Result::HeaderRejected;
    if (d[0] != 'A' || d[1] != 'N' || d[2] != 'P' || d[3] != 'D' || d[4] != 2) return Result::HeaderRejected;
    uint8_t type = d[5];
    if (type != kAnpdFormat && type != kAnpdAudio && type != kAnpdLossLedger) return Result::HeaderRejected;
    if (d[6] != 0 || d[7] != 0) return Result::HeaderRejected;
    uint64_t sessionId = getU64(d, 8);
    uint64_t sequence = getU64(d, 16);
    uint64_t frameIndex = getU64(d, 24);
    size_t payloadLength = (static_cast<size_t>(d[32]) << 8) | d[33];
    if (payloadLength > kAnpdMaxPlaintextBytes || d.size() != kAnpdHeaderBytes + payloadLength + kAnpdTagBytes)
        return Result::HeaderRejected;
    if (sessionId != sessionId_) return Result::Identity;

    std::array<uint8_t, 12> nonce;
    for (int i = 0; i < 4; i++) nonce[i] = static_cast<uint8_t>(kNonceDomain >> (24 - i * 8));
    for (int i = 0; i < 8; i++) nonce[4 + i] = static_cast<uint8_t>(sequence >> (56 - i * 8));
    Bytes plain;
    if (!aes256GcmOpen(key_, nonce, d.first(kAnpdHeaderBytes), d.subspan(kAnpdHeaderBytes), plain))
        return Result::Authentication;

    std::optional<LossLedger> ledger;
    if (type == kAnpdLossLedger) {
        if (frameIndex != 0) return Result::FormatRejected;
        ledger = LossLedger::decode(plain);
        if (!ledger) return Result::FormatRejected;
    } else if (type == kAnpdFormat) {
        auto want = format_.encode();
        if (frameIndex != 0 || plain.size() != want.size() || std::memcmp(plain.data(), want.data(), want.size()) != 0)
            return Result::FormatRejected;
    } else if (plain.empty() || plain.size() % format_.frameBytes != 0) {
        return Result::FormatRejected;
    }
    if (!wouldAccept(sequence)) return Result::Replay;

    // A record stream must open with the format descriptor.
    if (type != kAnpdFormat && !formatSeen_) return Result::FormatRejected;
    if (ledger) {
        auto applied = ledger_.apply(*ledger);
        if (applied == LedgerApply::Invalid) return Result::FormatRejected;
        if (applied != LedgerApply::Ignored)
            sampler_.settlement(ledger->sourceSegmentStart, ledger->settledThrough, format_.sampleRate,
                                ledger->hasSourceSegmentStart && ledger_.complete());
        commit(sequence);
        return Result::LossLedger;
    }
    return acceptFloat(type, sequence, frameIndex, plain);
}

AnpdIngress::Result AnpdIngress::acceptFloat(uint8_t type, uint64_t sequence, uint64_t frameIndex,
                                             std::span<const uint8_t> payload) {
    if (type == kAnpdFormat) {
        if (sequence != 0) return Result::FormatRejected;
        commit(sequence);
        formatSeen_ = true;
        return Result::Format;
    }
    if (sequence == 0) return Result::FormatRejected;
    uint32_t frames = static_cast<uint32_t>(payload.size() / format_.frameBytes);
    if (frameIndex + frames <= frameIndex) return Result::FormatRejected;
    hasAuthenticatedAudio_ = true;
    lastAuthenticatedAudioAtNanos_ = std::chrono::duration_cast<std::chrono::nanoseconds>(
            std::chrono::steady_clock::now().time_since_epoch()).count();
    std::array<float, kAnpdMaxPlaintextBytes / sizeof(float)> samples;
    size_t count = static_cast<size_t>(frames) * format_.channels;
    for (size_t i = 0; i < count; i++) {
        uint32_t bits = static_cast<uint32_t>(payload[i * 4]) | (static_cast<uint32_t>(payload[i * 4 + 1]) << 8) |
                        (static_cast<uint32_t>(payload[i * 4 + 2]) << 16) |
                        (static_cast<uint32_t>(payload[i * 4 + 3]) << 24);
        if ((bits & 0x7f800000u) == 0x7f800000u) return Result::FormatRejected;  // Float32 PCM must be finite
        std::memcpy(&samples[i], &bits, sizeof bits);
    }
    commit(sequence);
    authenticatedAudioFrames_ += frames;
    authenticatedAudioPayloadBytes_ += payload.size();
    auto pcm = std::span<const float>(samples.data(), count);
    bool accepted = sink_(frameIndex, pcm, frames);
    if (!accepted) droppedFrames_ += frames;
    sampler_.audio(frameIndex, pcm, format_.channels, accepted, lastAuthenticatedAudioAtNanos_);
    return Result::Audio;
}

}  // namespace anpod
