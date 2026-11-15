#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <memory>
#include <new>

class SpscFloatRing final {
public:
    static_assert(std::atomic<uint32_t>::is_always_lock_free,
            "the audio callback requires lock-free 32-bit atomics");

    bool initialize(uint32_t capacityFrames, uint32_t channels) {
        if (capacityFrames == 0 || capacityFrames >= (uint32_t{1} << 31)
                || channels == 0
                || capacityFrames > std::numeric_limits<size_t>::max() / channels) {
            return false;
        }
        const size_t sampleCount = static_cast<size_t>(capacityFrames) * channels;
        if (sampleCount > std::numeric_limits<size_t>::max() / sizeof(float)) return false;
        std::unique_ptr<float[]> samples(new (std::nothrow) float[sampleCount]);
        if (!samples) return false;
        samples_ = std::move(samples);
        capacityFrames_ = capacityFrames;
        channels_ = channels;
        readIndex_ = 0;
        writeIndex_ = 0;
        resetAfterQuiescence();
        return true;
    }

    uint32_t write(const float* source, uint32_t requestedFrames) noexcept {
        if (source == nullptr || requestedFrames == 0 || samples_ == nullptr) return 0;
        const uint32_t write = writeCursor_.load(std::memory_order_relaxed);
        const uint32_t read = readCursor_.load(std::memory_order_acquire);
        const uint32_t pending = write - read;
        if (pending > capacityFrames_) return 0;
        const uint32_t accepted = requestedFrames < capacityFrames_ - pending
                ? requestedFrames : capacityFrames_ - pending;
        copyIntoRing(writeIndex_, source, accepted);
        writeIndex_ = advanceIndex(writeIndex_, accepted);
        writeCursor_.store(write + accepted, std::memory_order_release);
        return accepted;
    }

    uint32_t render(float* destination, uint32_t requestedFrames,
            bool muted, float gain) noexcept {
        if (destination == nullptr || requestedFrames == 0 || samples_ == nullptr) return 0;
        if (muted) {
            std::memset(destination, 0,
                    static_cast<size_t>(requestedFrames) * channels_ * sizeof(float));
            return 0;
        }

        const uint32_t read = readCursor_.load(std::memory_order_relaxed);
        const uint32_t write = writeCursor_.load(std::memory_order_acquire);
        const uint32_t pending = write - read;
        const uint32_t available = pending <= capacityFrames_ ? pending : 0;
        const uint32_t consumed = requestedFrames < available ? requestedFrames : available;
        copyFromRing(readIndex_, destination, consumed);
        readIndex_ = advanceIndex(readIndex_, consumed);
        if (consumed < requestedFrames) {
            std::memset(destination + static_cast<size_t>(consumed) * channels_, 0,
                    static_cast<size_t>(requestedFrames - consumed) * channels_ * sizeof(float));
        }
        if (gain != 1.0f) {
            const size_t samples = static_cast<size_t>(requestedFrames) * channels_;
            for (size_t i = 0; i < samples; ++i) destination[i] *= gain;
        }
        readCursor_.store(read + consumed, std::memory_order_release);
        return consumed;
    }

    uint32_t pendingFrames() const noexcept {
        const uint32_t write = writeCursor_.load(std::memory_order_acquire);
        const uint32_t read = readCursor_.load(std::memory_order_acquire);
        const uint32_t pending = write - read;
        return pending <= capacityFrames_ ? pending : 0;
    }

    uint32_t capacityFrames() const noexcept { return capacityFrames_; }
    uint32_t channels() const noexcept { return channels_; }

    // Only call after the producer and audio callback have both stopped.
    void resetAfterQuiescence() noexcept {
        readCursor_.store(0, std::memory_order_relaxed);
        writeCursor_.store(0, std::memory_order_relaxed);
        readIndex_ = 0;
        writeIndex_ = 0;
    }

#ifdef ANPOD_RING_TESTING
    // Seed sequence cursors near wrap without changing the production cursor arithmetic.
    void seedAfterQuiescenceForTest(uint32_t sequenceCursor, uint32_t physicalIndex) noexcept {
        if (capacityFrames_ == 0) return;
        readCursor_.store(sequenceCursor, std::memory_order_relaxed);
        writeCursor_.store(sequenceCursor, std::memory_order_relaxed);
        readIndex_ = physicalIndex % capacityFrames_;
        writeIndex_ = readIndex_;
    }
#endif

private:
    uint32_t advanceIndex(uint32_t index, uint32_t frames) const noexcept {
        const uint32_t advance = frames % capacityFrames_;
        return advance >= capacityFrames_ - index
                ? advance - (capacityFrames_ - index) : index + advance;
    }

    void copyIntoRing(uint32_t cursor, const float* source, uint32_t frames) noexcept {
        uint32_t index = cursor % capacityFrames_;
        uint32_t first = frames < capacityFrames_ - index ? frames : capacityFrames_ - index;
        size_t firstSamples = static_cast<size_t>(first) * channels_;
        std::memcpy(samples_.get() + static_cast<size_t>(index) * channels_, source,
                firstSamples * sizeof(float));
        const uint32_t remaining = frames - first;
        if (remaining != 0) {
            std::memcpy(samples_.get(), source + firstSamples,
                    static_cast<size_t>(remaining) * channels_ * sizeof(float));
        }
    }

    void copyFromRing(uint32_t cursor, float* destination, uint32_t frames) const noexcept {
        uint32_t index = cursor % capacityFrames_;
        uint32_t first = frames < capacityFrames_ - index ? frames : capacityFrames_ - index;
        size_t firstSamples = static_cast<size_t>(first) * channels_;
        std::memcpy(destination, samples_.get() + static_cast<size_t>(index) * channels_,
                firstSamples * sizeof(float));
        const uint32_t remaining = frames - first;
        if (remaining != 0) {
            std::memcpy(destination + firstSamples, samples_.get(),
                    static_cast<size_t>(remaining) * channels_ * sizeof(float));
        }
    }

    std::unique_ptr<float[]> samples_;
    uint32_t capacityFrames_ = 0;
    uint32_t channels_ = 0;
    uint32_t writeIndex_ = 0;
    uint32_t readIndex_ = 0;
    alignas(64) std::atomic<uint32_t> writeCursor_{0};
    alignas(64) std::atomic<uint32_t> readCursor_{0};
};
