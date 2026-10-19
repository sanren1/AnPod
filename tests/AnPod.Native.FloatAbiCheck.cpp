#include <stddef.h>
#include <windows.h>

#include "audio_capture.h"
#include "audio_capture_float_blocks.h"

static_assert(sizeof(AudioCaptureCapabilitiesV2) == 16, "CapabilitiesV2 ABI size changed");
static_assert(offsetof(AudioCaptureCapabilitiesV2, maxCallbackFrames) == 12,
    "CapabilitiesV2 field order changed");
static_assert(sizeof(AudioCaptureFormatV2) == 48, "CaptureFormatV2 ABI size changed");
static_assert(offsetof(AudioCaptureFormatV2, mixSampleRate) == 8,
    "CaptureFormatV2 field order changed");
static_assert(offsetof(AudioCaptureFormatV2, captureSampleRate) == 32,
    "CaptureFormatV2 field order changed");

struct FloatDispatchCheck
{
    const float* expected;
    uint32_t totalFrames;
    uint32_t seenFrames;
    uint32_t calls;
    uint64_t firstFrameIndex;
    int expectSilence;
    int failed;
};

static void ANPOD_CALL CheckFloatBlock(const float* samples, uint32_t frames,
    uint32_t sampleRate, uint16_t channels, uint64_t frameIndex, void* userData)
{
    FloatDispatchCheck* check = (FloatDispatchCheck*)userData;
    const uint32_t remaining = check->totalFrames - check->seenFrames;
    const uint32_t expectedFrames = remaining > ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES
        ? ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES : remaining;
    if (frames == 0 || frames > ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES ||
        frames != expectedFrames ||
        sampleRate != 48000 || channels != 2 || frameIndex != check->firstFrameIndex + check->seenFrames)
    {
        check->failed = 1;
        return;
    }
    for (uint32_t frame = 0; frame < frames; ++frame)
    {
        const uint32_t absoluteFrame = check->seenFrames + frame;
        for (uint32_t channel = 0; channel < channels; ++channel)
        {
            const float value = samples[(size_t)frame * channels + channel];
            if (check->expectSilence)
            {
                if (value != 0.0f) check->failed = 1;
            }
            else if (value != check->expected[(size_t)absoluteFrame * channels + channel])
            {
                check->failed = 1;
            }
        }
    }
    check->seenFrames += frames;
    ++check->calls;
}

static int CheckFloatDispatch()
{
    const uint32_t totalFrames = 8193;
    float* input = new float[(size_t)totalFrames * 2];
    float scratch[ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES * 2];
    const float lowBitValue = 1.0f / 4194304.0f;
    for (uint32_t frame = 0; frame < totalFrames; ++frame)
    {
        input[(size_t)frame * 2] = frame == 0 ? lowBitValue : (float)frame / 100000.0f;
        input[(size_t)frame * 2 + 1] = -input[(size_t)frame * 2];
    }

    uint64_t nextFrameIndex = 37;
    FloatDispatchCheck normal = { input, totalFrames, 0, 0, nextFrameIndex, 0, 0 };
    const int normalOk = AnPodDispatchFloatFramesV2(input, totalFrames, 48000, 2, 0,
        scratch, &nextFrameIndex, CheckFloatBlock, &normal);
    if (!normalOk || normal.failed || normal.seenFrames != totalFrames || normal.calls != 3 ||
        nextFrameIndex != 37 + totalFrames || input[0] != lowBitValue)
    {
        delete[] input;
        return 20;
    }

    nextFrameIndex = 900;
    FloatDispatchCheck silent = { input, 4097, 0, 0, nextFrameIndex, 1, 0 };
    if (!AnPodDispatchFloatFramesV2(NULL, silent.totalFrames, 48000, 2, 1,
        scratch, &nextFrameIndex, CheckFloatBlock, &silent) || silent.failed ||
        silent.seenFrames != silent.totalFrames || silent.calls != 2 || nextFrameIndex != 4997)
    {
        delete[] input;
        return 21;
    }

    nextFrameIndex = 1200;
    FloatDispatchCheck paused = { input, 1, 0, 0, nextFrameIndex, 1, 0 };
    if (!AnPodDispatchFloatFramesV2(input, paused.totalFrames, 48000, 2, 1,
        scratch, &nextFrameIndex, CheckFloatBlock, &paused) || paused.failed ||
        paused.seenFrames != 1 || paused.calls != 1 || nextFrameIndex != 1201)
    {
        delete[] input;
        return 22;
    }

    delete[] input;
    return 0;
}

template <typename T>
static T GetExport(HMODULE module, const char* name)
{
    return reinterpret_cast<T>(GetProcAddress(module, name));
}

int main()
{
    const int floatDispatchResult = CheckFloatDispatch();
    if (floatDispatchResult != 0) return floatDispatchResult;

    HMODULE module = LoadLibraryW(L"audio_capture.dll");
    if (module == NULL) return 1;

    const char* oldExports[] = {
        "AudioCapture_Initialize", "AudioCapture_Listen", "AudioCapture_SetPcmCallback",
        "AudioCapture_GetPcmFormat", "AudioCapture_Start", "AudioCapture_SetStreamingEnabled",
        "AudioCapture_Stop", "AudioCapture_Cleanup", "AudioCapture_IsConnected",
        "AudioCapture_SetConnectToken", "AudioCapture_GetLastErrorCode",
        "AudioCapture_GetLastErrorMessage", "AudioCapture_ClearLastError",
        "AudioCapture_GetSentPcmBytes"
    };
    const char* newExports[] = {
        "AudioCapture_GetCapabilitiesV2", "AudioCapture_GetCaptureFormatV2",
        "AudioCapture_InitializeFloatV2", "AudioCapture_SetFloatPcmCallbackV2"
    };
    for (const char* name : oldExports)
    {
        if (GetProcAddress(module, name) == NULL) return 2;
    }
    for (const char* name : newExports)
    {
        if (GetProcAddress(module, name) == NULL) return 3;
    }

    typedef int (ANPOD_CALL *GetCapabilitiesFn)(AudioCaptureCapabilitiesV2*);
    typedef int (ANPOD_CALL *GetFormatFn)(AudioCaptureFormatV2*);
    GetCapabilitiesFn getCapabilities = GetExport<GetCapabilitiesFn>(module, newExports[0]);
    GetFormatFn getFormat = GetExport<GetFormatFn>(module, newExports[1]);

    AudioCaptureCapabilitiesV2 capabilities = { sizeof(capabilities), 2, 0, 0 };
    if (!getCapabilities(&capabilities) || capabilities.version != 2 ||
        capabilities.flags != ANPOD_AUDIO_CAPTURE_CAP_FLOAT32_V2 ||
        capabilities.maxCallbackFrames != 4096) return 4;
    capabilities.size = sizeof(capabilities) - sizeof(uint32_t);
    if (getCapabilities(&capabilities)) return 5;
    capabilities.size = sizeof(capabilities);
    capabilities.version = 3;
    if (getCapabilities(&capabilities)) return 6;
    struct ExtendedCapabilities
    {
        AudioCaptureCapabilitiesV2 value;
        uint32_t futureField;
    } extended = { { sizeof(ExtendedCapabilities), 2, 0, 0 }, 0x12345678 };
    if (!getCapabilities(&extended.value) || extended.futureField != 0x12345678) return 7;

    AudioCaptureFormatV2 format = {};
    format.size = sizeof(format);
    format.version = 2;
    if (getFormat(&format)) return 8; /* Format is unavailable before initialization. */

    FreeLibrary(module);
    return 0;
}
