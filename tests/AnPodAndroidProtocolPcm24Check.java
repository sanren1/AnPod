package com.anpod.server;

import java.util.Arrays;

/** Offline fixed-vector check for packed PCM24 and frame-based Float32 processing. */
public final class AnPodAndroidProtocolPcm24Check {
    private AnPodAndroidProtocolPcm24Check() { }

    public static void main(String[] args) {
        checkPackedVectors();
        checkInvalidSamplesAndFragments();
        checkFrameQueueAndPlc();
        checkFrameDriftAndFades();
        checkFrameDriftController();
        System.out.println("PCM24 codec and Float32 frame checks passed.");
    }

    private static void checkPackedVectors() {
        float lsb = 1f / Pcm24PackedCodec.SCALE;
        float halfLsb = 0.5f / Pcm24PackedCodec.SCALE;
        float[] samples = {
                -1f, 1f, lsb, -lsb, halfLsb, -halfLsb,
                32769f / Pcm24PackedCodec.SCALE, -32769f / Pcm24PackedCodec.SCALE,
                0.25f, -0.25f, 0.75f, -0.75f
        };
        byte[] expected = {
                0, 0, (byte) 0x80, (byte) 0xff, (byte) 0xff, 0x7f,
                1, 0, 0, (byte) 0xff, (byte) 0xff, (byte) 0xff,
                1, 0, 0, (byte) 0xff, (byte) 0xff, (byte) 0xff,
                1, (byte) 0x80, 0, (byte) 0xff, 0x7f, (byte) 0xff,
                0, 0, 0x20, 0, 0, (byte) 0xe0, 0, 0, 0x60, 0, 0, (byte) 0xa0
        };
        byte[] encoded = new byte[expected.length + 4];
        Pcm24PackedCodec.encodeFromFloat(samples, 0, samples.length, encoded, 2);
        if (!Arrays.equals(expected, Arrays.copyOfRange(encoded, 2, encoded.length - 2)))
            throw new AssertionError("PCM24 signed, tie, stereo-order, or low-bit vector changed");

        float[] decoded = new float[samples.length + 2];
        Pcm24PackedCodec.decodeToFloat(encoded, 2, samples.length, decoded, 1);
        if (decoded[0] != 0f || decoded[decoded.length - 1] != 0f
                || decoded[1] != -1f || decoded[2] != 8388607f / 8388608f
                || decoded[3] != lsb || decoded[4] != -lsb
                || decoded[7] != 32769f / Pcm24PackedCodec.SCALE
                || decoded[8] != -32769f / Pcm24PackedCodec.SCALE) {
            throw new AssertionError("PCM24 sign extension or Float32 decode vector changed");
        }
    }

    private static void checkInvalidSamplesAndFragments() {
        for (float invalid : new float[] { Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY }) {
            try {
                Pcm24PackedCodec.encodeFromFloat(new float[] { invalid }, 0, 1, new byte[3], 0);
                throw new AssertionError("non-finite PCM24 input was accepted");
            } catch (IllegalArgumentException expected) { }
        }
        expectIllegal(() -> Pcm24PackedCodec.decodeToFloat(new byte[2], 0, 1, new float[1], 0));
        expectIllegal(() -> Pcm24PackedCodec.encodeFromFloat(new float[1], 0, 1, new byte[2], 0));
        expectIllegal(() -> Pcm24PackedCodec.decodeToFloat(new byte[3], 1, 1, new float[1], 0));
    }

    private static void checkFrameQueueAndPlc() {
        FloatPcmFrameQueue lowLatency = new FloatPcmFrameQueue(48_000, true);
        FloatPcmFrameQueue balanced = new FloatPcmFrameQueue(48_000, false);
        if (lowLatency.initialTargetFrames() != 960 || balanced.initialTargetFrames() != 1920
                || lowLatency.audioTrackBufferFrames(4_800, true) != 720
                || balanced.audioTrackBufferFrames(4_800, false) != 2_400) {
            throw new AssertionError("Float32 frame and AudioTrack budgets changed");
        }

        if (!lowLatency.offer(4, new float[] { 0.5f, -0.5f, 0.75f, -0.75f }, 2)
                || !lowLatency.offer(0, new float[] { 0.1f, -0.1f, 0.3f, -0.3f }, 2))
            throw new AssertionError("out-of-order Float32 frames were not queued");
        if (lowLatency.offer(1, new float[] { 1f, 1f }, 1))
            throw new AssertionError("overlapping frame range was accepted");

        float[] output = new float[16];
        if (lowLatency.poll(output, 1, 2) != 2 || output[2] != 0.1f || output[3] != -0.1f
                || lowLatency.playoutFrame() != 2) {
            throw new AssertionError("Float32 queue did not poll complete frames at its frame cursor");
        }
        if (lowLatency.concealGap(output, 3, 2) != 2 || lowLatency.concealedFrames() != 2
                || output[6] == 0f || lowLatency.playoutFrame() != 4) {
            throw new AssertionError("bounded PLC did not fill the frame-index gap");
        }
        if (lowLatency.poll(output, 5, 2) != 2 || output[10] != 0f || output[11] != 0f) {
            throw new AssertionError("post-gap Float32 fade did not start at zero");
        }

        FloatPcmFrameQueue recovery = new FloatPcmFrameQueue(48_000, true);
        if (!recovery.offer(0, new float[] { 1f, 1f }, 1)) throw new AssertionError("seed frame rejected");
        float[] oneFrame = new float[2];
        recovery.poll(oneFrame, 0, 1);
        if (!recovery.offer(2, filledStereoFrames(49, 1f), 49)
                || !recovery.offer(51, filledStereoFrames(191, 1f), 191))
            throw new AssertionError("5ms PCM24 tail/body packet fragments were rejected");
        if (recovery.concealGap(oneFrame, 0, 1) != 1) throw new AssertionError("single-frame gap not concealed");
        float[] fadeChunk = new float[49 * 2];
        if (recovery.poll(fadeChunk, 0, 49) != 49 || fadeChunk[0] != 0f
                || fadeChunk[96] != 48f / 144f) {
            throw new AssertionError("recovery fade did not start smoothly across the 49-frame tail");
        }
        if (recovery.poll(fadeChunk, 0, 49) != 49
                || fadeChunk[0] != 49f / 144f || fadeChunk[96] != 97f / 144f) {
            throw new AssertionError("recovery fade reset at the next PCM24 packet fragment");
        }

        FloatPcmFrameQueue bounded = new FloatPcmFrameQueue(48_000, false);
        int accepted = 0;
        long frameIndex = 0;
        long arrivalNanos = 1_000_000_000L;
        for (int packet = 0; packet < 150; packet++) {
            if (bounded.offer(frameIndex, new float[48 * 2], 48, arrivalNanos)) accepted++;
            frameIndex += 48;
        }
        boolean replacedOldest = bounded.offer(frameIndex, new float[2], 1, arrivalNanos + 1_000_000L);
        boolean acceptedOldPacket = bounded.offer(0, new float[48 * 2], 48, arrivalNanos + 2_000_000L);
        if (accepted != 150 || bounded.packetCount() != 150 || bounded.packetCapacity() != 7_200
                || bounded.queuedFrames() != 7_153 || bounded.expiredFrames() != 48
                || frameIndex != 150L * 48
                || !replacedOldest || acceptedOldPacket || bounded.packetCount() != 150
                || bounded.queuedFrames() != 7_153 || bounded.playoutFrame() != 48) {
            throw new AssertionError("balanced Float32 queue capacity check failed: accepted=" + accepted
                    + ", replaced=" + replacedOldest + ", oldAccepted=" + acceptedOldPacket
                    + ", packets=" + bounded.packetCount() + ", frames=" + bounded.queuedFrames()
                    + ", expired=" + bounded.expiredFrames() + ", capacity=" + bounded.packetCapacity()
                    + ", watermark=" + bounded.playoutFrame());
        }
        FloatPcmFrameQueue expired = new FloatPcmFrameQueue(48_000, false);
        if (!expired.offer(0, new float[48 * 2], 48, arrivalNanos)
                || !expired.offer(48, new float[48 * 2], 48, arrivalNanos + 150_000_001L)
                || expired.expiredFrames() != 48 || expired.droppedFrames() != 48
                || expired.queuedFrames() != 48 || expired.playoutFrame() != 48) {
            throw new AssertionError("Float32 queue did not expire packets at its 150ms age budget");
        }
        expectIllegal(() -> bounded.offer(20_000, new float[192 * 2], 192));
        checkFloatQueueWaitAndStop();
    }

    private static void checkFloatQueueWaitAndStop() {
        FloatPcmFrameQueue queue = new FloatPcmFrameQueue(48_000, true);
        float[] readBuffer = new float[191 * 2];
        java.util.concurrent.CountDownLatch entered = new java.util.concurrent.CountDownLatch(1);
        java.util.concurrent.CountDownLatch done = new java.util.concurrent.CountDownLatch(1);
        java.util.concurrent.atomic.AtomicInteger read = new java.util.concurrent.atomic.AtomicInteger(-2);
        Thread worker = new Thread(() -> {
            entered.countDown();
            try { read.set(queue.readFrames(readBuffer, 0, 191)); }
            catch (java.io.IOException error) { throw new AssertionError("Float32 reader failed", error); }
            catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
            finally { done.countDown(); }
        });
        worker.start();
        try {
            if (!entered.await(1, java.util.concurrent.TimeUnit.SECONDS))
                throw new AssertionError("Float32 reader did not start");
            long frameIndex = 0;
            for (int packet = 0; packet < 5; packet++) {
                if (!queue.offer(frameIndex, new float[191 * 2], 191))
                    throw new AssertionError("initial Float32 pre-roll packet rejected");
                frameIndex += 191;
            }
            if (done.await(20, java.util.concurrent.TimeUnit.MILLISECONDS))
                throw new AssertionError("Float32 reader started before its 20ms target");
            if (!queue.offer(frameIndex, new float[5 * 2], 5))
                throw new AssertionError("final initial Float32 pre-roll frames rejected");
            if (!done.await(1, java.util.concurrent.TimeUnit.SECONDS) || read.get() != 191)
                throw new AssertionError("Float32 reader did not release data at its initial target");
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new AssertionError("Float32 queue check interrupted", interrupted);
        } finally {
            queue.stop();
            worker.interrupt();
            try { worker.join(1_000); }
            catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        }

        FloatPcmFrameQueue stopped = new FloatPcmFrameQueue(48_000, true);
        stopped.setPaused(true);
        stopped.stop();
        try {
            if (stopped.readFrames(new float[2], 0, 1) != -1)
                throw new AssertionError("stopped Float32 queue released a frame");
        } catch (java.io.IOException | InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new AssertionError("stopped Float32 queue check interrupted", interrupted);
        }
    }

    private static void checkFrameDriftAndFades() {
        FloatPcmFrameQueue queue = new FloatPcmFrameQueue(48_000, true);
        float[] insert = { 0f, 0f, 1f, 1f, 3f, 3f, 0f, 0f };
        int inserted = queue.adjustDrift(insert, 3, 4, -1);
        if (inserted != 4 || insert[2] != 0.5f || insert[3] != 0.5f
                || queue.insertedDriftFrames() != 1) {
            throw new AssertionError("Float32 drift insertion did not stay frame based");
        }
        float[] delete = { 0f, 0f, 0.1f, 0.1f, 1f, 1f, 0f, 0f };
        int deleted = queue.adjustDrift(delete, 3, 4, 1);
        if (deleted != 2 || delete[2] != 1f || delete[3] != 1f
                || queue.deletedDriftFrames() != 1) {
            throw new AssertionError("Float32 drift deletion did not remove a whole frame");
        }
        float[] faded = { 1f, -1f, 1f, -1f, 1f, -1f };
        FloatPcmFrameQueue.fadeIn(faded, 0, 3, 2, 3);
        if (faded[0] != 0f || faded[2] <= 0f || faded[4] != 2f / 3f)
            throw new AssertionError("Float32 fade-in did not use frame positions");
        FloatPcmFrameQueue.fadeOut(faded, 0, 3, 2, 3);
        if (faded[4] != 0f) throw new AssertionError("Float32 fade-out did not reach digital silence");
        float[] tailFade = { 1f, 1f, 1f, 1f, 1f, 1f };
        FloatPcmFrameQueue.fadeOut(tailFade, 0, 3, 2, 2);
        if (tailFade[0] != 1f || tailFade[2] != 0.5f || tailFade[4] != 0f || tailFade[5] != 0f)
            throw new AssertionError("Float32 fade-out did not apply to the end of the frame range");
    }

    private static void checkFrameDriftController() {
        checkDriftCase(100, 960, 300, 600, 1_800);
        checkDriftCase(-100, 960, 300, 600, 1_800);
        checkDriftCase(1_000, 960, 120, 3_000, 7_000);
        checkDriftCase(-1_000, 960, 120, 3_000, 7_000);
        checkDriftCase(100, 1_920, 900, 2_000, 5_500);
        checkDriftCase(-100, 1_920, 900, 2_000, 5_500);
        checkDriftCase(1_000, 1_920, 180, 4_000, 9_000);
        checkDriftCase(-1_000, 1_920, 180, 4_000, 9_000);

        com.anpod.server.FloatPcmDriftController jitter =
                new com.anpod.server.FloatPcmDriftController(48_000, 240);
        long now = 0;
        jitter.reset(now);
        int edits = 0;
        for (int step = 0; step < 800; step++) {
            now += 5_000_000L;
            long waterline = 2_000 + (step >= 200 && step < 300 ? 40 : 0);
            int adjustment = jitter.observe(now, waterline, true, false);
            if (adjustment != 0) { edits++; jitter.commit(adjustment); }
        }
        if (edits != 0) throw new AssertionError("Float32 drift chased a short waterline jitter");
    }

    private static void checkDriftCase(int ppm, int targetFrames, int seconds,
            int minimumEdits, int maximumSpan) {
        DriftSimulation simulation = simulateDrift(ppm, targetFrames, seconds);
        int edits = ppm > 0 ? simulation.deleted : simulation.inserted;
        int wrongDirection = ppm > 0 ? simulation.inserted : simulation.deleted;
        long maximumEdits = (long) seconds * 48_000 * Math.abs(ppm) / 1_000_000 + 30;
        if (edits < minimumEdits || edits > maximumEdits
                || wrongDirection != 0 || simulation.span > maximumSpan) {
            throw new AssertionError("Float32 drift exceeded ppm/waterline budget for target "
                    + targetFrames + " and offset " + ppm + "ppm: edits=" + edits
                    + ", wrong=" + wrongDirection + ", span=" + simulation.span);
        }
    }

    private static DriftSimulation simulateDrift(int ppm, int targetFrames, int seconds) {
        com.anpod.server.FloatPcmDriftController drift =
                new com.anpod.server.FloatPcmDriftController(48_000, targetFrames);
        long now = 0;
        drift.reset(now);
        long pendingTrackFrames = 720;
        long queuedFrames = targetFrames;
        long waterline = pendingTrackFrames + queuedFrames;
        long min = waterline;
        long max = waterline;
        double incomingRemainder = 0;
        int inserted = 0;
        int deleted = 0;
        int steps = seconds * 200;
        for (int step = 0; step < steps; step++) {
            now += 5_000_000L;
            incomingRemainder += 240.0 * (1.0 + ppm / 1_000_000.0);
            long incomingFrames = (long) incomingRemainder;
            incomingRemainder -= incomingFrames;
            int adjustment = drift.observe(now, pendingTrackFrames + queuedFrames, true, false);
            if (adjustment != 0) {
                drift.commit(adjustment);
                if (adjustment > 0) deleted++;
                else inserted++;
            }
            queuedFrames += incomingFrames - 240 - adjustment;
            waterline = pendingTrackFrames + queuedFrames;
            min = Math.min(min, waterline);
            max = Math.max(max, waterline);
        }
        return new DriftSimulation(inserted, deleted, max - min);
    }

    private static final class DriftSimulation {
        final int inserted;
        final int deleted;
        final long span;
        DriftSimulation(int inserted, int deleted, long span) {
            this.inserted = inserted;
            this.deleted = deleted;
            this.span = span;
        }
    }

    private static float[] filledStereoFrames(int frames, float value) {
        float[] samples = new float[frames * 2];
        java.util.Arrays.fill(samples, value);
        return samples;
    }

    private static void expectIllegal(Runnable operation) {
        try {
            operation.run();
            throw new AssertionError("invalid PCM24/frame input was accepted");
        } catch (IllegalArgumentException expected) { }
    }
}
