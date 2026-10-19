#ifndef ANPOD_AUDIO_CAPTURE_FLOAT_BLOCKS_H
#define ANPOD_AUDIO_CAPTURE_FLOAT_BLOCKS_H

#include <stdint.h>
#include <string.h>

#include "audio_capture.h"

#define ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES 4096u

/* Split an acquired engine packet into callback-sized chunks using caller-owned
   scratch storage. This function performs no allocation or synchronization. */
static inline int AnPodDispatchFloatFramesV2(
    const float* input,
    uint32_t frameCount,
    uint32_t sampleRate,
    uint16_t channels,
    int silent,
    float* scratch,
    uint64_t* nextFrameIndex,
    FloatPcmCallbackV2 callback,
    void* userData)
{
    if (sampleRate == 0 || channels != 2 || scratch == NULL || nextFrameIndex == NULL || callback == NULL)
    {
        return 0;
    }
    if (!silent && input == NULL)
    {
        return 0;
    }
    if ((uint64_t)frameCount > UINT64_MAX - *nextFrameIndex)
    {
        return 0;
    }

    uint32_t offset = 0;
    while (offset < frameCount)
    {
        const uint32_t remaining = frameCount - offset;
        const uint32_t frames = remaining > ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES
            ? ANPOD_AUDIO_CAPTURE_V2_MAX_CALLBACK_FRAMES : remaining;
        const size_t sampleCount = (size_t)frames * channels;
        if (silent)
        {
            memset(scratch, 0, sampleCount * sizeof(float));
        }
        else
        {
            memcpy(scratch, input + (size_t)offset * channels, sampleCount * sizeof(float));
        }
        callback(scratch, frames, sampleRate, channels, *nextFrameIndex, userData);
        offset += frames;
        *nextFrameIndex += frames;
    }
    return 1;
}

#endif
