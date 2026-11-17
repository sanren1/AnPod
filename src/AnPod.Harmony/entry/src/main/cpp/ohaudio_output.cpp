#include "ohaudio_output.h"

#include <chrono>
#include <cstring>
#include <hilog/log.h>
#include <ohaudio/native_audio_session_manager.h>
#include <ohaudio/native_audiorenderer.h>
#include <ohaudio/native_audiostreambuilder.h>

namespace anpod {
namespace {

// Same targets as the Android Float32 path, indexed by the descriptor mode ([0] Balanced, [1] LowLatency):
// FloatPcmFrameQueue start 40 / 20 ms (also the re-prime target after an underrun) and queue max 150 / 60 ms
// plus the output budget 50 / 15 ms (Pcm16FrameStream.*_AUDIO_TRACK_BUFFER_MS) as the ring capacity.
constexpr uint32_t kStartMs[2] = {40, 20};
constexpr uint32_t kCapacityMs[2] = {200, 75};
constexpr int64_t kAdvancingWindowNanos = 200'000'000;

// AudioSession belongs to the process. A replaced renderer may close after its successor starts.
std::mutex mixMutex;
OH_AudioSessionManager* mixManager = nullptr;
unsigned mixUsers = 0;

int activateMix(bool& held) {
    std::lock_guard lock(mixMutex);
    if (!mixManager) {
        auto result = OH_AudioManager_GetAudioSessionManager(&mixManager);
        if (result != AUDIOCOMMON_RESULT_SUCCESS) return result;
    }
    OH_AudioSession_Strategy strategy{CONCURRENCY_MIX_WITH_OTHERS};
    auto result = OH_AudioSessionManager_IsAudioSessionActivated(mixManager) && mixUsers
        ? AUDIOCOMMON_RESULT_SUCCESS : OH_AudioSessionManager_ActivateAudioSession(mixManager, &strategy);
    if (result == AUDIOCOMMON_RESULT_SUCCESS && !held) {
        held = true;
        ++mixUsers;
    }
    return result;
}

int64_t nowNanos() {
    return std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now().time_since_epoch())
        .count();
}

OH_AudioData_Callback_Result onWrite(OH_AudioRenderer*, void* self, void* data, int32_t bytes) {
    static_cast<OhAudioOutput*>(self)->render(data, bytes);
    return AUDIO_DATA_CALLBACK_RESULT_VALID;
}

void onInterrupt(OH_AudioRenderer*, void* self, OH_AudioInterrupt_ForceType type, OH_AudioInterrupt_Hint hint) {
    static_cast<OhAudioOutput*>(self)->interrupt(type, hint);
}

void logFailure(const char* step, int result, uint32_t rate, int mode) {
    OH_LOG_Print(LOG_APP, LOG_ERROR, 0xA0D0, "AnPod", "OHAudio %{public}s failed (%{public}d) for %{public}u Hz F32 %{public}s",
                 step, result, rate, mode == 1 ? "fast" : "normal");
}

}  // namespace

OhAudioOutput::OhAudioOutput(InterruptListener onInterrupt) : onInterrupt_(std::move(onInterrupt)) {}

OhAudioOutput::~OhAudioOutput() { close(); }

bool OhAudioOutput::startMixedRenderer() {
    // The system can deactivate the process session while a renderer remains paused.
    int result = activateMix(mixHeld_);
    if (result != AUDIOCOMMON_RESULT_SUCCESS) {
        logFailure("activate concurrent playback", result, sampleRate_, fast_.value_or(false));
        return false;
    }
    result = OH_AudioRenderer_Start(renderer_);
    if (result != AUDIOSTREAM_SUCCESS) logFailure("start", result, sampleRate_, fast_.value_or(false));
    return result == AUDIOSTREAM_SUCCESS;
}

bool OhAudioOutput::open(const AudioFormatV2& format) {
    if (renderer_ || format.sampleFormat != 3 || format.channels == 0 || format.mode > 1) return false;
    const int mode = format.mode;
    if (!ring_.initialize(format.sampleRate * kCapacityMs[mode] / 1000, format.channels)) return false;
    sampleRate_ = format.sampleRate;
    channels_ = format.channels;
    startThresholdFrames_ = format.sampleRate * kStartMs[mode] / 1000;
    paused_ = interrupted_ = primed_ = closing_ = resumeFailed_ = false;
    failure_.clear();
    underruns_ = 0;
    lastAdvanceNanos_ = 0;

    OH_AudioStreamBuilder* builder = nullptr;
    int result = OH_AudioStreamBuilder_Create(&builder, AUDIOSTREAM_TYPE_RENDERER);
    if (result != AUDIOSTREAM_SUCCESS) {
        logFailure("builder", result, format.sampleRate, mode);
        return false;
    }
    const char* step = "configure";
    if ((result = OH_AudioStreamBuilder_SetSamplingRate(builder, static_cast<int32_t>(format.sampleRate))) ==
            AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetChannelCount(builder, format.channels)) == AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetSampleFormat(builder, AUDIOSTREAM_SAMPLE_F32LE)) == AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetEncodingType(builder, AUDIOSTREAM_ENCODING_TYPE_RAW)) ==
            AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetRendererInfo(builder, AUDIOSTREAM_USAGE_MUSIC)) == AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetLatencyMode(
             builder, mode == 1 ? AUDIOSTREAM_LATENCY_MODE_FAST : AUDIOSTREAM_LATENCY_MODE_NORMAL)) ==
            AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetRendererWriteDataCallback(builder, onWrite, this)) ==
            AUDIOSTREAM_SUCCESS &&
        (result = OH_AudioStreamBuilder_SetRendererInterruptCallback(builder, onInterrupt, this)) ==
            AUDIOSTREAM_SUCCESS) {
        step = "generate";
        OH_AudioRenderer* renderer = nullptr;
        result = OH_AudioStreamBuilder_GenerateRenderer(builder, &renderer);
        renderer_ = renderer;
    }
    OH_AudioStreamBuilder_Destroy(builder);
    if (!renderer_) {
        logFailure(step, result, format.sampleRate, mode);
        return false;
    }
    // Never play a stream the system silently changed: the opened format must be the negotiated one.
    int32_t rate = 0, channels = 0;
    OH_AudioStream_SampleFormat sampleFormat = AUDIOSTREAM_SAMPLE_U8;
    if (OH_AudioRenderer_GetSamplingRate(renderer_, &rate) != AUDIOSTREAM_SUCCESS ||
        OH_AudioRenderer_GetChannelCount(renderer_, &channels) != AUDIOSTREAM_SUCCESS ||
        OH_AudioRenderer_GetSampleFormat(renderer_, &sampleFormat) != AUDIOSTREAM_SUCCESS ||
        rate != static_cast<int32_t>(format.sampleRate) || channels != format.channels ||
        sampleFormat != AUDIOSTREAM_SAMPLE_F32LE) {
        logFailure("format check", rate, format.sampleRate, mode);
        close();
        return false;
    }
    if (!startMixedRenderer()) {
        close();
        return false;
    }
    OH_AudioStream_FastStatus status = AUDIOSTREAM_FASTSTATUS_NORMAL;
    fast_.reset();
    if (OH_AudioRenderer_GetFastStatus(renderer_, &status) == AUDIOSTREAM_SUCCESS)
        fast_ = status == AUDIOSTREAM_FASTSTATUS_FAST;
    // LowLatency is the FAST path or nothing: never continue on the normal path in its name (ruling R10).
    if (mode == 1 && !fast_.value_or(false)) {
        logFailure("fast path", static_cast<int>(status), format.sampleRate, mode);
        close();
        failure_ = "low latency unsupported";
        return false;
    }
    return true;
}

bool OhAudioOutput::write(const float* interleaved, size_t frames) {
    if (!renderer_ || paused_ || interrupted_) return false;
    // All or nothing, so a dropped packet is counted whole; free space only grows while we are the producer.
    if (frames > ring_.capacityFrames() - ring_.pendingFrames()) return false;
    return ring_.write(interleaved, static_cast<uint32_t>(frames)) == frames;
}

int32_t OhAudioOutput::render(void* data, int32_t bytes) {
    float* out = static_cast<float*>(data);
    uint32_t frames = static_cast<uint32_t>(bytes) / (channels_ * sizeof(float));
    if (!primed_) {
        if (ring_.pendingFrames() < startThresholdFrames_) {
            std::memset(data, 0, static_cast<size_t>(bytes));
            return 0;
        }
        primed_ = true;
    }
    uint32_t consumed = ring_.render(out, frames, false, 1.0f);
    if (consumed > 0) lastAdvanceNanos_ = nowNanos();
    if (consumed < frames) {
        if (!closing_ && !paused_) underruns_++;  // the drained read while stopping is not an underrun
        primed_ = false;  // refill to the same start target before playing on
    }
    return static_cast<int32_t>(consumed);
}

void OhAudioOutput::interrupt(int32_t forceType, int32_t hint) {
    // Held across the Start below so close() cannot release the renderer under it.
    std::lock_guard<std::mutex> lock(interruptMutex_);
    if (closing_ || !renderer_) return;
    if (forceType == AUDIOSTREAM_INTERRUPT_FORCE &&
        (hint == AUDIOSTREAM_INTERRUPT_HINT_PAUSE || hint == AUDIOSTREAM_INTERRUPT_HINT_STOP)) {
        interrupted_ = true;
        if (onInterrupt_) onInterrupt_(true);
    } else if (hint == AUDIOSTREAM_INTERRUPT_HINT_RESUME && interrupted_) {
        primed_ = false;
        if (!paused_ && !startMixedRenderer()) {
            resumeFailed_ = true;  // the session ends with kOutputResumeError at the next HEARTBEAT / progress query
            return;
        }
        interrupted_ = false;
        if (onInterrupt_) onInterrupt_(false);
    }
}

void OhAudioOutput::pause() {
    paused_ = true;
    if (renderer_) OH_AudioRenderer_Pause(renderer_);
}

bool OhAudioOutput::resume() {
    primed_ = false;
    if (!renderer_ || (!interrupted_ && !startMixedRenderer())) return false;
    paused_ = false;
    return true;
}

void OhAudioOutput::close() {
    if (!renderer_) return;
    {
        std::lock_guard<std::mutex> lock(interruptMutex_);
        closing_ = true;
    }
    OH_AudioRenderer_Stop(renderer_);
    OH_AudioRenderer_Release(renderer_);
    renderer_ = nullptr;
    {
        std::lock_guard lock(mixMutex);
        if (mixHeld_ && --mixUsers == 0) {
            auto result = OH_AudioSessionManager_DeactivateAudioSession(mixManager);
            if (result != AUDIOCOMMON_RESULT_SUCCESS) logFailure("release audio session", result, sampleRate_, fast_.value_or(false));
        }
        mixHeld_ = false;
    }
    ring_.resetAfterQuiescence();
}

PlaybackStats OhAudioOutput::stats() const {
    PlaybackStats s;
    if (!renderer_) return s;
    s.sampleRate = sampleRate_;
    s.channels = static_cast<uint8_t>(channels_);
    s.pendingFrames = ring_.pendingFrames();
    s.capacityFrames = ring_.capacityFrames();
    s.startThresholdFrames = startThresholdFrames_;
    s.underrunCount = underruns_;
    s.playbackAdvancing = !paused_ && !interrupted_ && nowNanos() - lastAdvanceNanos_ < kAdvancingWindowNanos;
    return s;
}

}  // namespace anpod
