#include <jni.h>
#include <oboe/Oboe.h>

#include "AudioBufferBudget.h"
#include "SpscFloatRing.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <memory>
#include <mutex>
#include <new>
#include <string>
#include <time.h>
#include <unordered_map>

namespace {

constexpr jint kErrInvalidArgument = -1;
constexpr jint kErrInvalidHandle = -2;
constexpr jint kErrInvalidState = -3;
constexpr jint kErrUnsupportedFormat = -4;
constexpr jint kErrOpenFailed = -5;
constexpr jint kErrBudget = -6;
constexpr jint kErrStreamFailed = -7;
constexpr jint kErrJniArray = -8;

constexpr jint kInfoCount = 15;
constexpr jint kMetricCount = 11;
constexpr jint kTimestampCount = 2;

// info[3] and info[14] use this stable bridge enum, not Oboe's enum values.
constexpr jint kFormatUnknown = 0;
constexpr jint kFormatPcm16 = 1;
constexpr jint kFormatPackedPcm24 = 2;
constexpr jint kFormatFloat32 = 1;
constexpr jint kFormatPcm32 = 4;
constexpr jint kFormatOther = 5;
constexpr jint kHardwareFormatFloat32 = 3;

constexpr jint kApiUnknown = 0;
constexpr jint kApiOpenSles = 1;
constexpr jint kApiAAudio = 2;
constexpr jint kSharingUnknown = 0;
constexpr jint kSharingShared = 1;
constexpr jint kSharingExclusive = 2;
constexpr jint kPerformanceUnknown = 0;
constexpr jint kPerformanceNone = 1;
constexpr jint kPerformancePowerSaving = 2;
constexpr jint kPerformanceLowLatency = 3;
constexpr jint kPerformanceOffloaded = 4;

static_assert(std::atomic<uint32_t>::is_always_lock_free,
        "32-bit callback atomics must be lock-free on all packaged ABIs");

thread_local std::string gLastError;
std::mutex gSessionsMutex;
std::unordered_map<jlong, std::shared_ptr<struct AudioSession>> gSessions;
jlong gNextHandle = 1;
std::array<std::shared_ptr<struct AudioSession>, 8> gRetainedCloseFailures;

jfloat bitsToFloat(uint32_t bits) noexcept {
    jfloat value;
    std::memcpy(&value, &bits, sizeof(value));
    return value;
}

uint32_t floatToBits(jfloat value) noexcept {
    uint32_t bits;
    std::memcpy(&bits, &value, sizeof(bits));
    return bits;
}

jint mapAudioApi(oboe::AudioApi api) noexcept {
    switch (api) {
        case oboe::AudioApi::OpenSLES: return kApiOpenSles;
        case oboe::AudioApi::AAudio: return kApiAAudio;
        default: return kApiUnknown;
    }
}

jint mapSharing(oboe::SharingMode mode) noexcept {
    switch (mode) {
        case oboe::SharingMode::Shared: return kSharingShared;
        case oboe::SharingMode::Exclusive: return kSharingExclusive;
        default: return kSharingUnknown;
    }
}

jint mapPerformance(oboe::PerformanceMode mode) noexcept {
    switch (mode) {
        case oboe::PerformanceMode::None: return kPerformanceNone;
        case oboe::PerformanceMode::PowerSaving: return kPerformancePowerSaving;
        case oboe::PerformanceMode::LowLatency: return kPerformanceLowLatency;
        case oboe::PerformanceMode::PowerSavingOffloaded: return kPerformanceOffloaded;
        default: return kPerformanceUnknown;
    }
}

jint mapHardwareFormat(oboe::AudioFormat format) noexcept {
    switch (format) {
        case oboe::AudioFormat::I16: return kFormatPcm16;
        case oboe::AudioFormat::I24: return kFormatPackedPcm24;
        case oboe::AudioFormat::Float: return kHardwareFormatFloat32;
        case oboe::AudioFormat::I32: return kFormatPcm32;
        case oboe::AudioFormat::Unspecified: return kFormatUnknown;
        default: return kFormatOther;
    }
}

void setError(jint code, const std::string& message) {
    gLastError = message;
    if (gLastError.empty()) gLastError = "Native Oboe output error " + std::to_string(code);
}

struct CallbackState final : oboe::AudioStreamDataCallback, oboe::AudioStreamErrorCallback {
    explicit CallbackState(std::shared_ptr<SpscFloatRing> ringValue)
            : ring(std::move(ringValue)) {}

    oboe::DataCallbackResult onAudioReady(oboe::AudioStream*, void* audioData,
            int32_t numFrames) override {
        if (numFrames <= 0 || audioData == nullptr) {
            lastCallbackError.store(static_cast<uint32_t>(oboe::Result::ErrorIllegalArgument),
                    std::memory_order_relaxed);
            return oboe::DataCallbackResult::Stop;
        }
        const uint32_t frames = static_cast<uint32_t>(numFrames);
        const uint32_t channels = ring->channels();
        if (channels == 0 || static_cast<size_t>(frames)
                > std::numeric_limits<size_t>::max() / channels) {
            lastCallbackError.store(static_cast<uint32_t>(oboe::Result::ErrorOutOfRange),
                    std::memory_order_relaxed);
            return oboe::DataCallbackResult::Stop;
        }
        const bool isMuted = mute.load(std::memory_order_relaxed) != 0;
        const float gain = bitsToFloat(volumeBits.load(std::memory_order_relaxed));
        const uint32_t consumed = ring->render(static_cast<float*>(audioData), frames, isMuted, gain);
        callbackRequestedFrames.fetch_add(frames, std::memory_order_relaxed);
        ringConsumedFrames.fetch_add(consumed, std::memory_order_relaxed);
        zeroFilledFrames.fetch_add(isMuted ? frames : frames - consumed, std::memory_order_relaxed);
        return oboe::DataCallbackResult::Continue;
    }

    void onErrorBeforeClose(oboe::AudioStream*, oboe::Result error) override {
        lastCallbackError.store(static_cast<uint32_t>(error), std::memory_order_relaxed);
        mute.store(1, std::memory_order_relaxed);
    }

    void onErrorAfterClose(oboe::AudioStream*, oboe::Result error) override {
        lastCallbackError.store(static_cast<uint32_t>(error), std::memory_order_relaxed);
        mute.store(1, std::memory_order_relaxed);
    }

    void resetMetricsAfterQuiescence() noexcept {
        ringConsumedFrames.store(0, std::memory_order_relaxed);
        callbackRequestedFrames.store(0, std::memory_order_relaxed);
        zeroFilledFrames.store(0, std::memory_order_relaxed);
        lastCallbackError.store(0, std::memory_order_relaxed);
    }

    std::shared_ptr<SpscFloatRing> ring;
    std::atomic<uint32_t> mute{1};
    std::atomic<uint32_t> volumeBits{floatToBits(1.0f)};
    std::atomic<uint32_t> ringConsumedFrames{0};
    std::atomic<uint32_t> callbackRequestedFrames{0};
    std::atomic<uint32_t> zeroFilledFrames{0};
    std::atomic<uint32_t> lastCallbackError{0};
};

struct AudioSession final {
    AudioSession(int32_t rate, int32_t channelCount, int32_t budget, int32_t requestedRing)
            : sampleRate(rate), channels(channelCount), totalBudgetFrames(budget),
              requestedRingFrames(requestedRing), ring(std::make_shared<SpscFloatRing>()),
              callback(std::make_shared<CallbackState>(ring)) {}

    jint openStreamAndFillInfo(jint* info) {
        if (stream) {
            return fillInfo(info);
        }

        oboe::AudioStreamBuilder builder;
        builder.setDirection(oboe::Direction::Output)
                ->setPerformanceMode(oboe::PerformanceMode::LowLatency)
                ->setSharingMode(oboe::SharingMode::Shared)
                ->setUsage(oboe::Usage::Media)
                ->setContentType(oboe::ContentType::Music)
                ->setSampleRate(sampleRate)
                ->setChannelCount(channels)
                ->setFormat(oboe::AudioFormat::Float)
                ->setSampleRateConversionQuality(oboe::SampleRateConversionQuality::None)
                ->setFormatConversionAllowed(false)
                ->setChannelConversionAllowed(false)
                ->setDataCallback(callback)
                ->setErrorCallback(callback);

        oboe::Result result = builder.openStream(stream);
        if (result != oboe::Result::OK || !stream) {
            setError(kErrOpenFailed, std::string("Oboe openStream: ") + oboe::convertToText(result));
            return kErrOpenFailed;
        }
        const jint status = validateAndSetBuffer();
        if (status != 0) {
            const jint closeResult = closeStream();
            return closeResult == 0 ? status : closeResult;
        }
        return fillInfo(info);
    }

    jint validateAndSetBuffer() {
        if (!stream || stream->getSampleRate() != sampleRate
                || stream->getChannelCount() != channels
                || stream->getFormat() != oboe::AudioFormat::Float) {
            setError(kErrUnsupportedFormat, "Oboe actual client format does not match requested Float32 format");
            return kErrUnsupportedFormat;
        }
        const int32_t capacity = stream->getBufferCapacityInFrames();
        const int32_t burst = stream->getFramesPerBurst();
        if (capacity <= 0 || burst <= 0 || totalBudgetFrames <= 0 || requestedRingFrames <= 0) {
            setError(kErrBudget, "Oboe did not report a usable buffer capacity, burst, or frame budget");
            return kErrBudget;
        }
        const uint32_t retainedRingCapacity = ring->capacityFrames();
        if (retainedRingCapacity > static_cast<uint32_t>(std::numeric_limits<int32_t>::max())) {
            setError(kErrBudget, "Existing bridge ring capacity exceeds supported frame range");
            return kErrBudget;
        }
        const int32_t retainedRingFrames = static_cast<int32_t>(retainedRingCapacity);
        const int32_t requestedStreamFrames = requestedStreamBufferFrames(capacity, totalBudgetFrames,
                requestedRingFrames, retainedRingFrames);
        if (requestedStreamFrames <= 0) {
            setError(kErrBudget, "Output frame budget cannot hold both stream buffer and bridge ring");
            return kErrBudget;
        }
        oboe::ResultWithValue<int32_t> setSize = stream->setBufferSizeInFrames(requestedStreamFrames);
        actualStreamBufferFrames = setSize ? setSize.value() : stream->getBufferSizeInFrames();
        if (actualStreamBufferFrames <= 0) {
            setError(kErrBudget, std::string("Oboe actual stream buffer size unavailable: ")
                    + oboe::convertToText(setSize.error()));
            return kErrBudget;
        }
        const int32_t actualRing = actualBridgeRingFrames(totalBudgetFrames, requestedRingFrames,
                retainedRingFrames, actualStreamBufferFrames, burst);
        if (actualRing <= 0) {
            setError(kErrBudget, "Output frame budget cannot hold one callback burst plus the actual stream buffer");
            return kErrBudget;
        }
        if (ring->capacityFrames() == 0) {
            if (!ring->initialize(static_cast<uint32_t>(actualRing), static_cast<uint32_t>(channels))) {
                setError(kErrOpenFailed, "Could not allocate the Float32 bridge ring");
                return kErrOpenFailed;
            }
        }
        return 0;
    }

    jint fillInfo(jint* info) const {
        if (info == nullptr || !stream) return kErrInvalidState;
        const int32_t hardwareRate = stream->getHardwareSampleRate();
        const int32_t hardwareChannels = stream->getHardwareChannelCount();
        const oboe::AudioFormat hardwareFormat = stream->getHardwareFormat();
        info[0] = 0;
        info[1] = stream->getSampleRate();
        info[2] = stream->getChannelCount();
        info[3] = kFormatFloat32;
        info[4] = stream->getFramesPerBurst() > 0 ? stream->getFramesPerBurst() : 0;
        info[5] = stream->getBufferSizeInFrames();
        info[6] = stream->getBufferCapacityInFrames();
        info[7] = static_cast<jint>(ring->capacityFrames());
        info[8] = mapAudioApi(stream->getAudioApi());
        info[9] = mapSharing(stream->getSharingMode());
        info[10] = mapPerformance(stream->getPerformanceMode());
        info[11] = totalBudgetFrames;
        info[12] = hardwareRate > 0 ? hardwareRate : 0;
        info[13] = hardwareChannels > 0 ? hardwareChannels : 0;
        info[14] = mapHardwareFormat(hardwareFormat);
        return 0;
    }

    jint start(jint* info) {
        if (stream == nullptr) {
            const jint openResult = openStreamAndFillInfo(info);
            if (openResult != 0) return openResult;
        } else if (fillInfo(info) != 0) {
            setError(kErrInvalidState, "Could not refresh Oboe stream information");
            return kErrInvalidState;
        }
        callback->mute.store(0, std::memory_order_relaxed);
        const oboe::Result result = stream->requestStart();
        if (result != oboe::Result::OK) {
            callback->mute.store(1, std::memory_order_relaxed);
            setError(kErrStreamFailed, std::string("Oboe requestStart: ") + oboe::convertToText(result));
            closeStream();
            return kErrStreamFailed;
        }
        started = true;
        return 0;
    }

    jint closeStream() {
        if (!stream) return 0;
        callback->mute.store(1, std::memory_order_relaxed);
        const oboe::StreamState state = stream->getState();
        if (state != oboe::StreamState::Closed && started) stream->requestStop();
        oboe::Result result = oboe::Result::OK;
        if (state != oboe::StreamState::Closed) result = stream->close();
        if (result != oboe::Result::OK && stream->getState() != oboe::StreamState::Closed) {
            setError(kErrStreamFailed, std::string("Oboe close: ") + oboe::convertToText(result));
            return kErrStreamFailed;
        }
        stream.reset();
        started = false;
        actualStreamBufferFrames = 0;
        return 0;
    }

    int32_t sampleRate;
    int32_t channels;
    int32_t totalBudgetFrames;
    int32_t requestedRingFrames;
    int32_t actualStreamBufferFrames = 0;
    bool started = false;
    std::mutex controlMutex;
    std::shared_ptr<SpscFloatRing> ring;
    std::shared_ptr<CallbackState> callback;
    std::shared_ptr<oboe::AudioStream> stream;
    std::atomic<uint32_t> acceptedFrames{0};
    std::atomic<uint32_t> producerFullRetries{0};
};

std::shared_ptr<AudioSession> lookup(jlong handle) {
    std::lock_guard<std::mutex> lock(gSessionsMutex);
    auto it = gSessions.find(handle);
    return it == gSessions.end() ? nullptr : it->second;
}

jlong registerSession(const std::shared_ptr<AudioSession>& session) {
    std::lock_guard<std::mutex> lock(gSessionsMutex);
    jlong handle = gNextHandle++;
    if (handle == 0) handle = gNextHandle++;
    gSessions.emplace(handle, session);
    return handle;
}

void retainCloseFailure(const std::shared_ptr<AudioSession>& session) noexcept {
    std::lock_guard<std::mutex> lock(gSessionsMutex);
    for (auto& retained : gRetainedCloseFailures) {
        if (!retained) {
            retained = session;
            return;
        }
    }
    // A failed native close must never free callback state. This only runs on
    // a control/error path; deliberately retain process-lifetime ownership.
    auto* retained = new (std::nothrow) std::shared_ptr<AudioSession>(session);
    (void) retained;
}

jint copyInfoToJava(JNIEnv* env, jintArray infoArray, jint* info) {
    if (infoArray == nullptr) {
        setError(kErrJniArray, "Info array is null");
        return kErrJniArray;
    }
    const jsize length = env->GetArrayLength(infoArray);
    if (length < kInfoCount) {
        if (length > 0) env->SetIntArrayRegion(infoArray, 0, 1, info);
        setError(kErrJniArray, "Info array must contain at least 15 integers");
        return kErrJniArray;
    }
    env->SetIntArrayRegion(infoArray, 0, kInfoCount, info);
    if (env->ExceptionCheck()) {
        setError(kErrJniArray, "Could not write stream info array");
        return kErrJniArray;
    }
    return 0;
}

jint fillInfoArrayError(JNIEnv* env, jintArray infoArray, jint status) {
    if (infoArray != nullptr && env->GetArrayLength(infoArray) > 0) {
        env->SetIntArrayRegion(infoArray, 0, 1, &status);
    }
    return status;
}

} // namespace

extern "C" JNIEXPORT jlong JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeOpen(JNIEnv* env, jclass, jint sampleRate,
        jint channels, jint totalBudgetFrames, jint requestedBridgeCapacityFrames, jintArray infoArray) {
    if (infoArray == nullptr) {
        setError(kErrJniArray, "Info array is null");
        return 0;
    }
    if (env->GetArrayLength(infoArray) < kInfoCount) {
        fillInfoArrayError(env, infoArray, kErrJniArray);
        setError(kErrJniArray, "Info array must contain at least 15 integers");
        return 0;
    }
    if (sampleRate <= 0 || channels <= 0 || channels > 8 || totalBudgetFrames <= 0
            || requestedBridgeCapacityFrames <= 0) {
        fillInfoArrayError(env, infoArray, kErrInvalidArgument);
        setError(kErrInvalidArgument, "Invalid output rate, channel count, or frame budget");
        return 0;
    }
    std::shared_ptr<AudioSession> session;
    try {
        session = std::make_shared<AudioSession>(sampleRate, channels,
                totalBudgetFrames, requestedBridgeCapacityFrames);
    } catch (const std::bad_alloc&) {
        fillInfoArrayError(env, infoArray, kErrOpenFailed);
        setError(kErrOpenFailed, "Could not allocate Oboe output session");
        return 0;
    }
    jint info[kInfoCount]{};
    jint status = session->openStreamAndFillInfo(info);
    if (status != 0) {
        fillInfoArrayError(env, infoArray, status);
        if (session->stream) return registerSession(session);
        return 0;
    }
    status = copyInfoToJava(env, infoArray, info);
    if (status != 0) {
        if (session->closeStream() != 0) retainCloseFailure(session);
        return 0;
    }
    const jlong handle = registerSession(session);
    gLastError.clear();
    return handle;
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeWriteFloat(JNIEnv* env, jclass, jlong handle,
        jfloatArray samplesArray, jint offsetFrames, jint frameCount) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    const uint32_t callbackError = session->callback->lastCallbackError.load(std::memory_order_acquire);
    if (callbackError != 0) {
        setError(kErrStreamFailed, "Oboe callback reported a stream error");
        return kErrStreamFailed;
    }
    if (samplesArray == nullptr || offsetFrames < 0 || frameCount < 0) {
        setError(kErrInvalidArgument, "Invalid Float32 array or frame range");
        return kErrInvalidArgument;
    }
    if (frameCount == 0) return 0;
    const uint64_t channels = static_cast<uint32_t>(session->channels);
    const uint64_t firstSample = static_cast<uint64_t>(static_cast<uint32_t>(offsetFrames)) * channels;
    const uint64_t requestedSamples = static_cast<uint64_t>(static_cast<uint32_t>(frameCount)) * channels;
    const jsize arrayLength = env->GetArrayLength(samplesArray);
    if (firstSample > static_cast<uint64_t>(arrayLength)
            || requestedSamples > static_cast<uint64_t>(arrayLength) - firstSample) {
        setError(kErrJniArray, "Float32 frame range exceeds the source array");
        return kErrJniArray;
    }
    const uint32_t available = session->ring->capacityFrames() - session->ring->pendingFrames();
    const uint32_t request = std::min(static_cast<uint32_t>(frameCount), available);
    if (request == 0) {
        session->producerFullRetries.fetch_add(1, std::memory_order_relaxed);
        return 0;
    }
    jfloat* elements = env->GetFloatArrayElements(samplesArray, nullptr);
    if (elements == nullptr) {
        setError(kErrJniArray, "Could not access Float32 source array");
        return kErrJniArray;
    }
    const float* source = elements + firstSample;
    const uint32_t accepted = session->ring->write(source, request);
    env->ReleaseFloatArrayElements(samplesArray, elements, JNI_ABORT);
    session->acceptedFrames.fetch_add(accepted, std::memory_order_relaxed);
    if (accepted < static_cast<uint32_t>(frameCount)) {
        session->producerFullRetries.fetch_add(1, std::memory_order_relaxed);
    }
    gLastError.clear();
    return static_cast<jint>(accepted);
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeRequestPause(JNIEnv*, jclass, jlong handle) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    session->callback->mute.store(1, std::memory_order_relaxed);
    gLastError.clear();
    return 0;
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativePauseAndFlush(JNIEnv*, jclass, jlong handle) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    std::lock_guard<std::mutex> lock(session->controlMutex);
    const jint closeResult = session->closeStream();
    if (closeResult != 0) return closeResult;
    session->ring->resetAfterQuiescence();
    session->callback->resetMetricsAfterQuiescence();
    session->acceptedFrames.store(0, std::memory_order_relaxed);
    session->producerFullRetries.store(0, std::memory_order_relaxed);
    gLastError.clear();
    return 0;
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeStart(JNIEnv* env, jclass, jlong handle, jintArray infoArray) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    if (infoArray == nullptr || env->GetArrayLength(infoArray) < kInfoCount) {
        setError(kErrJniArray, "Info array must contain at least 15 integers");
        return kErrJniArray;
    }
    std::lock_guard<std::mutex> lock(session->controlMutex);
    jint info[kInfoCount]{};
    jint status = session->start(info);
    if (status == 0) status = copyInfoToJava(env, infoArray, info);
    else fillInfoArrayError(env, infoArray, status);
    if (status != 0) return status;
    gLastError.clear();
    return 0;
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeSetVolume(JNIEnv*, jclass, jlong handle, jfloat volume) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    if (!std::isfinite(volume) || volume < 0.0f || volume > 1.0f) {
        setError(kErrInvalidArgument, "Volume must be finite and in [0, 1]");
        return kErrInvalidArgument;
    }
    session->callback->volumeBits.store(floatToBits(volume), std::memory_order_relaxed);
    gLastError.clear();
    return 0;
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeGetMetrics(JNIEnv* env, jclass, jlong handle,
        jintArray metricsArray, jlongArray timestampArray) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    if (metricsArray == nullptr || timestampArray == nullptr
            || env->GetArrayLength(metricsArray) < kMetricCount
            || env->GetArrayLength(timestampArray) < kTimestampCount) {
        setError(kErrJniArray, "Metrics and timestamp arrays must have at least 11 and 2 elements");
        return kErrJniArray;
    }
    std::lock_guard<std::mutex> controlLock(session->controlMutex);
    jint metrics[kMetricCount]{};
    metrics[0] = static_cast<jint>(session->acceptedFrames.load(std::memory_order_relaxed));
    metrics[1] = static_cast<jint>(session->callback->ringConsumedFrames.load(std::memory_order_relaxed));
    metrics[2] = static_cast<jint>(session->callback->callbackRequestedFrames.load(std::memory_order_relaxed));
    metrics[3] = static_cast<jint>(session->callback->zeroFilledFrames.load(std::memory_order_relaxed));
    metrics[4] = static_cast<jint>(session->ring->pendingFrames());
    metrics[5] = -1;
    metrics[6] = 0;
    metrics[7] = 0;
    int64_t framesWritten = -1;
    int64_t framesRead = -1;
    auto xrun = session->stream ? session->stream->getXRunCount()
                                : oboe::ResultWithValue<int32_t>(oboe::Result::ErrorInvalidState);
    metrics[8] = xrun ? xrun.value() : -1;
    metrics[9] = static_cast<jint>(session->producerFullRetries.load(std::memory_order_relaxed));
    metrics[10] = static_cast<jint>(session->callback->lastCallbackError.load(std::memory_order_relaxed));
    jlong timestamp[kTimestampCount] = {-1, -1};
    if (session->stream) {
        framesWritten = session->stream->getFramesWritten();
        framesRead = session->stream->getFramesRead();
        metrics[6] = framesWritten >= 0 ? static_cast<jint>(static_cast<uint32_t>(framesWritten)) : -1;
        metrics[7] = framesRead >= 0 ? static_cast<jint>(static_cast<uint32_t>(framesRead)) : -1;
        int64_t framePosition = -1;
        int64_t timeNanos = -1;
        if (session->stream->getTimestamp(CLOCK_MONOTONIC, &framePosition, &timeNanos) == oboe::Result::OK) {
            timestamp[0] = framePosition;
            timestamp[1] = timeNanos;
        }
    }
    if (framesWritten >= 0 && framesRead >= 0) {
        const uint32_t streamPending = static_cast<uint32_t>(framesWritten)
                - static_cast<uint32_t>(framesRead);
        metrics[5] = streamPending <= static_cast<uint32_t>(session->totalBudgetFrames)
                ? static_cast<jint>(streamPending) : -1;
    }
    env->SetIntArrayRegion(metricsArray, 0, kMetricCount, metrics);
    env->SetLongArrayRegion(timestampArray, 0, kTimestampCount, timestamp);
    if (env->ExceptionCheck()) {
        setError(kErrJniArray, "Could not write Oboe metrics arrays");
        return kErrJniArray;
    }
    gLastError.clear();
    return 0;
}

extern "C" JNIEXPORT jint JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeClose(JNIEnv*, jclass, jlong handle) {
    auto session = lookup(handle);
    if (!session) {
        setError(kErrInvalidHandle, "Invalid Oboe output handle");
        return kErrInvalidHandle;
    }
    {
        std::lock_guard<std::mutex> lock(session->controlMutex);
        const jint closeResult = session->closeStream();
        if (closeResult != 0) return closeResult;
    }
    {
        std::lock_guard<std::mutex> lock(gSessionsMutex);
        auto it = gSessions.find(handle);
        if (it != gSessions.end() && it->second == session) {
            gSessions.erase(it);
        }
    }
    gLastError.clear();
    return 0;
}

extern "C" JNIEXPORT jstring JNICALL
Java_app_anpod_receiver_OboeNativeBridge_nativeGetLastErrorMessage(JNIEnv* env, jclass) {
    return env->NewStringUTF(gLastError.c_str());
}
