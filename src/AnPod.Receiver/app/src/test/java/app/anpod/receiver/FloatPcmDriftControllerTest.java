package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import com.anpod.server.FloatPcmDriftController;
import com.anpod.server.FloatPcmFrameQueue;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;
import org.junit.Test;

public final class FloatPcmDriftControllerTest {
    @Test public void fixedTargetsConvergeFromBothSidesAndModeSwitchKeepsNewTarget() {
        int lowTarget = new FloatPcmFrameQueue(48_000, true).initialTargetFrames();
        int balancedTarget = new FloatPcmFrameQueue(48_000, false).initialTargetFrames();
        for (int target : new int[] {lowTarget, balancedTarget}) {
            int deadband = target / 4;
            for (int offset : new int[] {-2 * deadband, 2 * deadband}) {
                long[] result = simulateWaterline(target, target + offset, 0, 120);
                assertEquals(target, result[4]);
                assertTrue(Math.abs(result[0] - target) <= deadband + 100);
                assertTrue(result[6] > result[5]);
            }
        }

        long[] switched = simulateWaterline(balancedTarget, lowTarget, 0, 120);
        assertEquals(balancedTarget, switched[4]);
        assertTrue(Math.abs(switched[0] - balancedTarget) <= balancedTarget / 4 + 100);

        long[] noOutput = simulateWaterline(lowTarget, lowTarget, 100, 120, 0);
        long[] largeOutput = simulateWaterline(lowTarget, lowTarget, 100, 120, 4);
        for (int i = 0; i <= 4; i++) assertEquals("output pending must not change source control", noOutput[i], largeOutput[i]);
        assertTrue(largeOutput[6] > noOutput[6]);
    }

    @Test public void virtualClockDriftOf100And300PpmHasCorrectBoundedFeedback() {
        int[] targets = {new FloatPcmFrameQueue(48_000, true).initialTargetFrames(),
                new FloatPcmFrameQueue(48_000, false).initialTargetFrames()};
        for (int target : targets) {
            for (int ppm : new int[] {-300, -100, 100, 300}) {
                long[] result = simulateWaterline(target, target, ppm, 600);
                assertEquals(target, result[4]);
                assertTrue(Math.abs(result[0] - target) <= target / 2 + 300);
                assertTrue(result[6] > result[5]);
                assertTrue(result[3] <= 1_000);
                if (ppm > 0) assertTrue("fast source must be corrected by late deletions", result[2] > result[1]);
                else assertTrue("slow source must be corrected by late insertions", result[1] > result[2]);
            }
        }
    }

    @Test public void discontinuityResetsControllerDebtAndShortPlcDoesNotRebuffer() throws Exception {
        FloatPcmDriftController drift = new FloatPcmDriftController(1_000, 40);
        long now = 1_000_000_000L;
        drift.reset(now, 1);
        for (int i = 0; i < 250; i++) {
            now += 10_000_000L;
            drift.observe(now, 60, true, false);
        }
        now += 10_000_000L;
        assertEquals(0, drift.observe(now, 45, true, true));
        assertEquals(4, drift.resetReason());
        assertEquals(45, drift.observedFrames());
        assertTrue(!drift.hasFilteredError());

        FloatPcmFrameQueue queue = new FloatPcmFrameQueue(1_000, true, 95);
        assertTrue(queue.offer(0, stereoFrames(20), 20));
        assertEquals(20, queue.readFrames(new float[40], 0, 20));
        assertTrue(queue.offer(25, stereoFrames(10), 10));
        assertEquals(5, queue.readFrames(new float[40], 0, 20));
        assertEquals(10, queue.readFrames(new float[40], 0, 20));
        assertEquals(5, queue.concealedFrames());
        assertEquals(20, queue.initialTargetFrames());
    }

    @Test public void sustainedPositiveAndNegativeWaterlineErrorsCorrectInTheRightDirection() {
        assertCorrectionDirection(60, 1);
        assertCorrectionDirection(20, -1);
    }

    @Test public void longSourceGapAndOversizedFrameGapBothRebufferAtInitialTarget() throws Exception {
        FloatPcmFrameQueue idleQueue = new FloatPcmFrameQueue(1_000, true, 95);
        assertTrue(idleQueue.offer(0, stereoFrames(20), 20));
        assertEquals(20, idleQueue.readFrames(new float[40], 0, 20));
        FutureTask<Integer> idleRead = blockedRead(idleQueue);
        try {
            Thread.sleep(130);
            assertTrue(idleQueue.offer(20, stereoFrames(10), 10));
            Thread.sleep(20);
            assertTrue(!idleRead.isDone());
            assertTrue(idleQueue.offer(30, stereoFrames(10), 10));
            assertEquals(10, idleRead.get(1, TimeUnit.SECONDS).intValue());
        } finally {
            idleQueue.stop();
        }

        FloatPcmFrameQueue gapQueue = new FloatPcmFrameQueue(1_000, true, 95);
        assertTrue(gapQueue.offer(0, stereoFrames(20), 20));
        assertEquals(20, gapQueue.readFrames(new float[40], 0, 20));
        FutureTask<Integer> gapRead = blockedRead(gapQueue);
        try {
            assertTrue(gapQueue.offer(100, stereoFrames(10), 10));
            Thread.sleep(30);
            assertTrue(!gapRead.isDone());
            assertTrue(gapQueue.offer(110, stereoFrames(10), 10));
            assertEquals(10, gapRead.get(1, TimeUnit.SECONDS).intValue());
        } finally {
            gapQueue.stop();
        }
    }

    private static void assertCorrectionDirection(int observedFrames, int expectedAdjustment) {
        FloatPcmDriftController controller = new FloatPcmDriftController(1_000, 40);
        long now = 1_000_000_000L;
        controller.reset(now, 1);
        boolean corrected = false;
        for (int i = 0; i < 1_000; i++) {
            now += 10_000_000L;
            int adjustment = controller.observe(now, observedFrames, true, false);
            if (adjustment != 0) {
                assertEquals(expectedAdjustment, adjustment);
                controller.commit(adjustment);
                corrected = true;
                break;
            }
        }
        assertTrue(corrected);
        assertTrue(Math.abs(controller.correctionPpm()) <= 1_000);
        assertEquals(expectedAdjustment > 0 ? 1 : -1, Integer.signum(controller.correctionPpm()));
    }

    /** Virtual source clock plus controller edits; output pending varies but is deliberately not an input. */
    private static long[] simulateWaterline(int targetFrames, int initialFrames, int sourceClockPpm,
            int durationSeconds) {
        return simulateWaterline(targetFrames, initialFrames, sourceClockPpm, durationSeconds, 1);
    }

    private static long[] simulateWaterline(int targetFrames, int initialFrames, int sourceClockPpm,
            int durationSeconds, int outputPendingMultiplier) {
        final int sampleRate = 48_000;
        final long stepNanos = 10_000_000L;
        final int steps = durationSeconds * 100;
        FloatPcmDriftController controller = new FloatPcmDriftController(sampleRate, targetFrames);
        long now = 1_000_000_000L;
        controller.reset(now, 1);
        long waterline = initialFrames;
        double fractionalSourceFrames = 0;
        long lateInserts = 0, lateDeletes = 0;
        long minTotalBuffer = Long.MAX_VALUE, maxTotalBuffer = Long.MIN_VALUE;
        int maxPpm = 0;
        for (int step = 0; step < steps; step++) {
            now += stepNanos;
            fractionalSourceFrames += sampleRate * (double) sourceClockPpm * stepNanos / 1_000_000_000_000_000.0;
            while (fractionalSourceFrames >= 1) { waterline++; fractionalSourceFrames--; }
            while (fractionalSourceFrames <= -1) { waterline--; fractionalSourceFrames++; }
            // Platform pending can change independently; only source queue+current block is observed.
            long outputPendingFrames = (step / 50 % 4) * targetFrames * outputPendingMultiplier;
            long totalBufferFrames = waterline + outputPendingFrames;
            minTotalBuffer = Math.min(minTotalBuffer, totalBufferFrames);
            maxTotalBuffer = Math.max(maxTotalBuffer, totalBufferFrames);
            int adjustment = controller.observe(now, waterline, true, false);
            maxPpm = Math.max(maxPpm, Math.abs(controller.correctionPpm()));
            if (adjustment != 0) {
                waterline -= adjustment;
                controller.commit(adjustment);
                if (step >= steps / 2) {
                    if (adjustment > 0) lateDeletes++;
                    else lateInserts++;
                }
            }
        }
        return new long[] {waterline, lateInserts, lateDeletes, maxPpm,
                controller.targetFrames(), minTotalBuffer, maxTotalBuffer};
    }

    private static FutureTask<Integer> blockedRead(FloatPcmFrameQueue queue) {
        FutureTask<Integer> read = new FutureTask<>(() -> queue.readFrames(new float[40], 0, 20));
        Thread thread = new Thread(read, "Float32 rebuffer check");
        thread.setDaemon(true);
        thread.start();
        return read;
    }

    private static float[] stereoFrames(int frames) { return new float[frames * 2]; }
}
