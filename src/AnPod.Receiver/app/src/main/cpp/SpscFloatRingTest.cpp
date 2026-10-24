#ifdef NDEBUG
#undef NDEBUG
#endif

#include "SpscFloatRing.h"
#include "AudioBufferBudget.h"

#include <array>
#include <atomic>
#include <cassert>
#include <cstdint>
#include <iostream>
#include <thread>

static void testPartialFullWrapAndSilence() {
    SpscFloatRing ring;
    assert(ring.initialize(4, 2));
    const float first[] = {1, 2, 3, 4, 5, 6};
    assert(ring.write(first, 3) == 3);
    const float extra[] = {7, 8, 9, 10};
    assert(ring.write(extra, 2) == 1);
    assert(ring.pendingFrames() == 4);

    float output[10]{};
    assert(ring.render(output, 3, false, 1.0f) == 3);
    assert((std::array<float, 6>{output[0], output[1], output[2], output[3], output[4], output[5]}
            == std::array<float, 6>{1, 2, 3, 4, 5, 6}));
    const float wrapped[] = {11, 12, 13, 14};
    assert(ring.write(wrapped, 2) == 2);
    assert(ring.render(output, 5, false, 0.5f) == 3);
    assert(output[0] == 3.5f && output[1] == 4.0f);
    assert(output[2] == 5.5f && output[3] == 6.0f);
    assert(output[4] == 6.5f && output[5] == 7.0f);
    assert(output[6] == 0.0f && output[7] == 0.0f);
    assert(output[8] == 0.0f && output[9] == 0.0f);
    assert(ring.pendingFrames() == 0);

    assert(ring.write(first, 2) == 2);
    assert(ring.render(output, 2, true, 1.0f) == 0);
    assert(output[0] == 0.0f && output[1] == 0.0f
            && output[2] == 0.0f && output[3] == 0.0f);
    assert(ring.pendingFrames() == 2);
    ring.resetAfterQuiescence();
    assert(ring.pendingFrames() == 0);
}

static void testSequenceCounterWrapUsesIndependentPhysicalIndex() {
    SpscFloatRing ring;
    assert(ring.initialize(5, 1));
    ring.seedAfterQuiescenceForTest(UINT32_MAX - 1, 4);
    const float first[] = {1, 2, 3};
    assert(ring.write(first, 3) == 3); // sequence cursor wraps to 1
    float output[5]{};
    assert(ring.render(output, 2, false, 1.0f) == 2); // read cursor wraps to 0
    assert(output[0] == 1 && output[1] == 2);
    const float second[] = {4, 5, 6, 7};
    assert(ring.write(second, 4) == 4);
    assert(ring.render(output, 5, false, 1.0f) == 5);
    assert(output[0] == 3 && output[1] == 4 && output[2] == 5
            && output[3] == 6 && output[4] == 7);
    assert(ring.pendingFrames() == 0);
}

static void testSingleProducerConsumerOrder() {
    constexpr uint32_t frameCount = 250000;
    SpscFloatRing ring;
    assert(ring.initialize(127, 1));
    std::atomic<bool> producerDone{false};
    std::thread producer([&] {
        uint32_t next = 0;
        while (next < frameCount) {
            float batch[31];
            uint32_t count = frameCount - next < 31 ? frameCount - next : 31;
            for (uint32_t i = 0; i < count; ++i) batch[i] = static_cast<float>(next + i);
            uint32_t accepted = ring.write(batch, count);
            next += accepted;
            if (accepted == 0) std::this_thread::yield();
        }
        producerDone.store(true, std::memory_order_release);
    });

    uint32_t expected = 0;
    float batch[29];
    while (expected < frameCount) {
        const uint32_t requested = frameCount - expected < 29 ? frameCount - expected : 29;
        const uint32_t count = ring.render(batch, requested, false, 1.0f);
        for (uint32_t i = 0; i < count; ++i) {
            assert(batch[i] == static_cast<float>(expected + i));
        }
        expected += count;
        if (count == 0 && !producerDone.load(std::memory_order_acquire)) std::this_thread::yield();
    }
    producer.join();
    assert(ring.pendingFrames() == 0);
}

static void testReopenKeepsRingWithinFrameBudget() {
    constexpr int32_t budget = 720;
    constexpr int32_t requestedRing = 240;
    assert(requestedStreamBufferFrames(600, budget, requestedRing, 0) == 480);
    assert(actualBridgeRingFrames(budget, requestedRing, 0, 528, 128) == 192);

    // Reopen may get more stream-buffer budget, but must retain the old ring size.
    assert(requestedStreamBufferFrames(600, budget, requestedRing, 192) == 528);
    assert(actualBridgeRingFrames(budget, requestedRing, 192, 480, 128) == 192);
    assert(actualBridgeRingFrames(budget, requestedRing, 192, 480, 193) == -1);
    assert(actualBridgeRingFrames(budget, requestedRing, 192, 529, 128) == -1);
}

int main() {
    testPartialFullWrapAndSilence();
    testSequenceCounterWrapUsesIndependentPhysicalIndex();
    testSingleProducerConsumerOrder();
    testReopenKeepsRingWithinFrameBudget();
    std::cout << "SpscFloatRing tests passed\n";
}
