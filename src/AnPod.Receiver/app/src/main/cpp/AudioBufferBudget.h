#pragma once

#include <algorithm>
#include <cstdint>

inline int32_t requestedStreamBufferFrames(int32_t streamCapacityFrames,
        int32_t totalBudgetFrames, int32_t requestedRingFrames,
        int32_t retainedRingFrames) {
    if (streamCapacityFrames <= 0 || totalBudgetFrames <= 0 || requestedRingFrames <= 0
            || retainedRingFrames < 0) {
        return -1;
    }
    const int32_t ringFrames = retainedRingFrames > 0
            ? retainedRingFrames : std::min(requestedRingFrames, totalBudgetFrames);
    if (ringFrames >= totalBudgetFrames) return -1;
    return std::min(streamCapacityFrames, totalBudgetFrames - ringFrames);
}

inline int32_t actualBridgeRingFrames(int32_t totalBudgetFrames, int32_t requestedRingFrames,
        int32_t retainedRingFrames, int32_t actualStreamBufferFrames, int32_t burstFrames) {
    if (totalBudgetFrames <= 0 || requestedRingFrames <= 0 || retainedRingFrames < 0
            || actualStreamBufferFrames <= 0 || burstFrames <= 0
            || actualStreamBufferFrames >= totalBudgetFrames) {
        return -1;
    }
    const int32_t budgetRoom = totalBudgetFrames - actualStreamBufferFrames;
    const int32_t ringFrames = retainedRingFrames > 0
            ? retainedRingFrames : std::min(requestedRingFrames, budgetRoom);
    if (ringFrames < burstFrames || ringFrames <= 0 || ringFrames > budgetRoom) return -1;
    return ringFrames;
}
