// OHAudio Float32 renderer for the wired session: opens exactly the negotiated rate / channels as F32LE,
// LowLatency (mode 1) as AUDIOSTREAM_LATENCY_MODE_FAST and Balanced as NORMAL. A format or mode the device
// cannot open fails; there is no fallback to another format or mode. The session reader thread fills an SPSC
// ring that the OHAudio write callback drains.
#pragma once

#include "float_ring.h"
#include "stream_session.h"

#include <atomic>
#include <functional>

struct OH_AudioRendererStruct;

namespace anpod {

class OhAudioOutput final : public PlaybackSink {
public:
    // Called on an OHAudio thread: true when an audio interrupt paused the stream, false when it resumed.
    using InterruptListener = std::function<void(bool interrupted)>;

    explicit OhAudioOutput(InterruptListener onInterrupt = {});
    ~OhAudioOutput() override;
    OhAudioOutput(const OhAudioOutput&) = delete;
    OhAudioOutput& operator=(const OhAudioOutput&) = delete;

    bool open(const AudioFormatV2& format) override;
    bool write(const float* interleaved, size_t frames) override;
    void pause() override;
    void resume() override;
    void close() override;
    PlaybackStats stats() const override;

    bool isOpen() const { return renderer_ != nullptr; }
    // The latency path the system actually granted (OH_AudioRenderer_GetFastStatus after start).
    bool fast() const { return fast_; }

    // OHAudio C callbacks (public so the .cpp can take their address).
    int32_t render(void* data, int32_t bytes);
    void interrupt(int32_t forceType, int32_t hint);

private:
    InterruptListener onInterrupt_;
    OH_AudioRendererStruct* renderer_ = nullptr;
    SpscFloatRing ring_;
    uint32_t sampleRate_ = 0, channels_ = 0, startThresholdFrames_ = 0;
    bool fast_ = false;
    std::atomic<bool> paused_{false}, interrupted_{false}, primed_{false};
    std::atomic<uint32_t> underruns_{0};
    std::atomic<int64_t> lastAdvanceNanos_{0};
};

}  // namespace anpod
