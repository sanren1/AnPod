#pragma once

#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <mutex>
#include <optional>
#include <span>

namespace anpod {

// Spec section 17 (waveform B): 120 points x 10 ms = 1.2 s; Android PcmWaveformTap buckets are also 10 ms.
inline constexpr size_t kAodPoints = 120;
inline constexpr int64_t kAodBucketNanos = 10000000;

struct AodSnapshot {
    std::array<float, kAodPoints> left{}, right{};
    bool receiving = false;
    std::optional<bool> fast;
    std::optional<double> bitsPerSecond, bufferMs, lossPercent;
    uint32_t sampleRate = 0, channels = 0;
    std::optional<uint32_t> underruns;
    std::optional<int64_t> newestAgeNanos;  // now - time the newest bucket was published; absent with no live wave
};

// One ANPD reader writes. The UI copies only the published peak history; the writer never waits for it.
class AodSampler {
public:
    void audio(uint64_t start, std::span<const float> pcm, unsigned channels, bool accepted, int64_t now) {
        size_t frames = pcm.size() / channels;
        lastAudio_ = now;
        tail_ = std::max(tail_, start + frames);
        if (accepted) cover(start, start + frames);
        if (bucket_ < 0 || now - bucket_ > 400000000) {
            peaks_ = {};
            leftPeak_ = rightPeak_ = 0;
            bucket_ = now;
        }
        for (size_t i = 0; i < frames; i += std::max<size_t>(1, (frames + 127) / 128)) {
            leftPeak_ = std::max(leftPeak_, std::abs(pcm[i * channels]));
            rightPeak_ = std::max(rightPeak_, std::abs(pcm[i * channels + (channels > 1)]));
        }
        if (now - bucket_ < kAodBucketNanos) return;
        for (auto& side : peaks_) std::move_backward(side.begin(), side.end() - 1, side.end());
        peaks_[0][0] = std::min(1.f, leftPeak_);
        peaks_[1][0] = std::min(1.f, rightPeak_);
        leftPeak_ = rightPeak_ = 0;
        bucket_ = now;
        std::unique_lock lock(waveMutex_, std::try_to_lock);
        if (lock) {
            published_ = peaks_;
            publishedAt_ = now;
        }
    }

    // Count source frames absent from accepted PCM in the settled portion of the last minute.
    void settlement(uint64_t origin, uint64_t settled, unsigned rate, bool complete) {
        uint64_t end = std::min(settled, tail_);
        uint64_t window = uint64_t{60} * rate;
        uint64_t start = std::max(origin, end > window ? end - window : 0);
        if (!complete || start >= end || start < forgottenThrough_) {
            loss_ = -1;
            return;
        }
        uint64_t good = 0;
        for (size_t i = 0; i < count_; ++i) {
            uint64_t lo = std::max(start, ranges_[i].start), hi = std::min(end, ranges_[i].end);
            if (hi > lo) good += hi - lo;
        }
        loss_ = 100.0 * (end - start - good) / (end - start);
    }

    AodSnapshot snapshot(int64_t now, uint64_t bytes) {
        AodSnapshot out;
        auto last = lastAudio_.load();
        out.receiving = last >= 0 && now - last <= 2000000000;
        if (last >= 0 && now - last <= 400000000) {
            std::lock_guard lock(waveMutex_);
            if (now - publishedAt_ <= 400000000) {
                out.left = published_[0];
                out.right = published_[1];
                out.newestAgeNanos = now - publishedAt_;
            }
        }
        if (windowStart_ < 0) {
            windowStart_ = now;
            baseline_ = bytes;
        } else if (now - windowStart_ >= 500000000) {
            rate_ = std::round((bytes - baseline_) * 8e9 / (now - windowStart_));
            windowStart_ = now;
            baseline_ = bytes;
        }
        out.bitsPerSecond = rate_;
        double loss = loss_.load();
        if (loss >= 0) out.lossPercent = loss;
        return out;
    }

private:
    struct Range { uint64_t start, end; };
    void cover(uint64_t start, uint64_t end) {
        if (count_ && ranges_[count_ - 1].end == start) {
            ranges_[count_ - 1].end = end;
            return;
        }
        auto first = ranges_.begin(), limit = first + count_;
        auto at = std::lower_bound(first, limit, start, [](Range r, uint64_t n) { return r.end < n; });
        auto after = at;
        while (after != limit && after->start <= end) {
            start = std::min(start, after->start);
            end = std::max(end, after->end);
            ++after;
        }
        if (after != at) {
            count_ -= after - at - 1;
            if (after != at + 1) std::move(after, limit, at + 1);
        } else {
            if (count_ == ranges_.size()) {
                if (at == first) { forgottenThrough_ = std::max(forgottenThrough_, end); return; }
                forgottenThrough_ = std::max(forgottenThrough_, first->end);
                std::move(first + 1, at, first);
                --at;
            } else {
                std::move_backward(at, limit, limit + 1);
                ++count_;
            }
        }
        *at = {start, end};
    }
    std::array<Range, 256> ranges_{};
    size_t count_ = 0;
    uint64_t tail_ = 0, forgottenThrough_ = 0;
    std::atomic<double> loss_{-1};
    std::atomic<int64_t> lastAudio_{-1};
    int64_t bucket_ = -1, windowStart_ = -1, publishedAt_ = -1;
    uint64_t baseline_ = 0;
    std::optional<double> rate_;
    float leftPeak_ = 0, rightPeak_ = 0;
    std::array<std::array<float, kAodPoints>, 2> peaks_{}, published_{};
    std::mutex waveMutex_;
};

}  // namespace anpod
