#include <jni.h>
#include <oboe/Oboe.h>
#include <oboe/OboeExtensions.h>
#include <android/log.h>
#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <ctime>
#include <iomanip>
#include <memory>
#include <mutex>
#include <sstream>
#include <string>
#include <vector>

namespace {
constexpr const char* TAG = "AnPodAudioProbe";
constexpr double kNanosPerSecond = 1000000000.0;

int64_t nowNs() {
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return static_cast<int64_t>(ts.tv_sec) * 1000000000LL + ts.tv_nsec;
}

// This callback performs no allocation, blocking calls, JNI, logging or network work.
class ClipCallback final : public oboe::AudioStreamDataCallback {
public:
    std::vector<int16_t> clip;
    int32_t channels = 2;
    size_t sampleOffset = 0; // audio callback thread only
    std::atomic<int64_t> callbacks{0};
    std::atomic<int64_t> lastAtNs{0};
    std::atomic<int64_t> latestGapNs{0};
    std::atomic<int64_t> maxGapNs{0};

    oboe::DataCallbackResult onAudioReady(oboe::AudioStream*, void* audioData,
                                           int32_t numFrames) override {
        int16_t* dst = static_cast<int16_t*>(audioData);
        const size_t count = static_cast<size_t>(numFrames) * channels;
        if (clip.empty()) {
            std::memset(dst, 0, count * sizeof(int16_t));
            return oboe::DataCallbackResult::Continue;
        }
        size_t written = 0;
        while (written < count) {
            const size_t n = std::min(count - written, clip.size() - sampleOffset);
            std::memcpy(dst + written, clip.data() + sampleOffset, n * sizeof(int16_t));
            written += n;
            sampleOffset = (sampleOffset + n == clip.size()) ? 0 : sampleOffset + n;
        }
        const int64_t current = nowNs();
        const int64_t previous = lastAtNs.exchange(current, std::memory_order_relaxed);
        if (previous != 0 && current > previous) {
            const int64_t gap = current - previous;
            latestGapNs.store(gap, std::memory_order_relaxed);
            int64_t mx = maxGapNs.load(std::memory_order_relaxed);
            while (gap > mx && !maxGapNs.compare_exchange_weak(mx, gap, std::memory_order_relaxed)) {}
        }
        callbacks.fetch_add(1, std::memory_order_relaxed);
        return oboe::DataCallbackResult::Continue;
    }
};

std::mutex gMutex; // control and stats threads only; never taken in callback
std::shared_ptr<oboe::AudioStream> gStream;
ClipCallback gCallback; // permanent until process exit, safe for stop/close
std::string gLastError;

void closeStreamLocked() {
    if (!gStream) return;
    // Closing waits for the callback to finish before the clip can be changed.
    gStream->requestStop();
    gStream->close();
    gStream.reset();
}

std::string jsonSnapshotLocked() {
    std::ostringstream o;
    if (!gStream) return "{\"running\":false,\"error\":\"not started\"}";
    auto stream = gStream;
    const int32_t sampleRate = stream->getSampleRate();
    const int32_t burst = stream->getFramesPerBurst();
    const int32_t bufferFrames = stream->getBufferSizeInFrames();
    const int32_t capacity = stream->getBufferCapacityInFrames();
    const auto xr = stream->getXRunCount();
    const int32_t xrun = xr ? xr.value() : -1;
    const int64_t framesWritten = stream->getFramesWritten();
    int64_t presented = 0, presentedAt = 0;
    const oboe::Result tsResult = stream->getTimestamp(CLOCK_MONOTONIC, &presented, &presentedAt);
    // This is an output-pipeline estimate, not acoustic speaker latency.
    double latency = -1;
    if (tsResult == oboe::Result::OK && sampleRate > 0 && framesWritten >= presented) {
        latency = ((presentedAt - nowNs()) + (framesWritten - presented) *
                   (kNanosPerSecond / sampleRate)) / 1000000.0;
        if (latency < 0 || latency > 1000) latency = -1;
    }
    o << std::fixed << std::setprecision(3)
      << "{\"running\":true,\"engine\":\"Oboe\",\"sampleRate\":" << sampleRate
      << ",\"channels\":" << stream->getChannelCount()
      << ",\"framesPerBurst\":" << burst
      << ",\"bufferFrames\":" << bufferFrames
      << ",\"capacityFrames\":" << capacity
      << ",\"xrun\":" << xrun
      << ",\"framesWritten\":" << framesWritten
      << ",\"latencyMs\":" << latency
      << ",\"callbackCount\":" << gCallback.callbacks.load()
      << ",\"callbackGapMs\":" << (gCallback.latestGapNs.load() / 1000000.0)
      << ",\"maxCallbackGapMs\":" << (gCallback.maxGapNs.load() / 1000000.0)
      << ",\"api\":\"" << (stream->getAudioApi() == oboe::AudioApi::AAudio ? "AAudio" : "OpenSLES")
      << "\",\"sharing\":\"" << (stream->getSharingMode() == oboe::SharingMode::Exclusive ? "Exclusive" : "Shared")
      << "\",\"perf\":\"" << (stream->getPerformanceMode() == oboe::PerformanceMode::LowLatency ? "LowLatency" : "Other")
      << "\",\"mmap\":\"" << (oboe::OboeExtensions::isMMapUsed(stream.get()) ? "YES" : "NO")
      << "\"}";
    return o.str();
}
} // namespace

extern "C" JNIEXPORT jstring JNICALL
Java_app_anpod_audioprobe_MainActivity_nativeStart(
        JNIEnv* env, jclass, jshortArray samples, jint sampleRate, jint channels, jint bursts) {
    std::lock_guard<std::mutex> guard(gMutex);
    closeStreamLocked();
    if (samples == nullptr || sampleRate <= 0 || channels != 2 || bursts < 1 || bursts > 8) {
        return env->NewStringUTF("Invalid PCM parameters");
    }
    const jsize n = env->GetArrayLength(samples);
    if (n < sampleRate || n % channels != 0) return env->NewStringUTF("Bad clip length");
    gCallback.clip.resize(static_cast<size_t>(n));
    env->GetShortArrayRegion(samples, 0, n, reinterpret_cast<jshort*>(gCallback.clip.data()));
    if (env->ExceptionCheck()) return nullptr;
    gCallback.sampleOffset = 0;
    gCallback.channels = channels;
    gCallback.callbacks = 0;
    gCallback.lastAtNs = 0;
    gCallback.latestGapNs = 0;
    gCallback.maxGapNs = 0;

    oboe::AudioStreamBuilder builder;
    builder.setDirection(oboe::Direction::Output);
    builder.setAudioApi(oboe::AudioApi::AAudio);
    builder.setPerformanceMode(oboe::PerformanceMode::LowLatency);
    builder.setSharingMode(oboe::SharingMode::Shared);
    builder.setUsage(oboe::Usage::Media);
    builder.setContentType(oboe::ContentType::Music);
    builder.setSampleRate(sampleRate);
    builder.setChannelCount(channels);
    builder.setFormat(oboe::AudioFormat::I16);
    builder.setDataCallback(&gCallback);

    auto openResult = builder.openStream(gStream);
    if (openResult != oboe::Result::OK || !gStream) {
        gLastError = std::string("Oboe openStream: ") + oboe::convertToText(openResult);
        gStream.reset();
        return env->NewStringUTF(gLastError.c_str());
    }
    const int burst = gStream->getFramesPerBurst();
    if (burst > 0) {
        // Requested 2/3/4 bursts; actual number is captured in telemetry.
        const auto accepted = gStream->setBufferSizeInFrames(burst * bursts);
        if (!accepted) __android_log_print(ANDROID_LOG_WARN, TAG, "buffer size request unsupported");
    }
    auto result = gStream->requestStart();
    if (result != oboe::Result::OK) {
        gLastError = std::string("Oboe requestStart: ") + oboe::convertToText(result);
        closeStreamLocked();
        return env->NewStringUTF(gLastError.c_str());
    }
    return env->NewStringUTF("");
}

extern "C" JNIEXPORT void JNICALL
Java_app_anpod_audioprobe_MainActivity_nativeStop(JNIEnv*, jclass) {
    std::lock_guard<std::mutex> guard(gMutex);
    closeStreamLocked();
}

extern "C" JNIEXPORT jstring JNICALL
Java_app_anpod_audioprobe_MainActivity_nativeSnapshot(JNIEnv* env, jclass) {
    std::lock_guard<std::mutex> guard(gMutex);
    const auto info = jsonSnapshotLocked();
    return env->NewStringUTF(info.c_str());
}
