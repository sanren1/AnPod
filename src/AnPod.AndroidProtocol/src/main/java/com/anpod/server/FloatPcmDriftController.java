package com.anpod.server;

/** Frame-count-only waterline controller for Float32 playout. */
public final class FloatPcmDriftController {
    private static final long BASELINE_NANOS = 1_000_000_000L;
    private static final long SAMPLE_NANOS = 10_000_000L;
    private static final long PERSISTENCE_NANOS = 2_000_000_000L;
    private static final long MAX_SAMPLE_GAP_NANOS = 100_000_000L;
    private static final long CREDIT_PER_FRAME = 1_000_000_000_000L;
    private static final int MAX_CORRECTION_PPM = 1_000;

    private final int sampleRate;
    private final int targetFrames;
    private final long[] baselineSamples = new long[101];
    private int baselineCount;
    private long baselineStartNanos;
    private long nextBaselineSampleNanos;
    private long baselineFrames;
    private long lastObservationNanos;
    private long outsideSinceNanos;
    private int outsideDirection;
    private long credit;
    private double filteredErrorFrames;
    private boolean baselineReady;
    private long resetCount;

    public FloatPcmDriftController(int sampleRate, int targetFrames) {
        if (sampleRate <= 0 || targetFrames <= 0)
            throw new IllegalArgumentException("invalid Float32 drift target");
        this.sampleRate = sampleRate;
        this.targetFrames = targetFrames;
    }

    public synchronized void reset(long nowNanos) {
        resetCount++;
        baselineCount = 0;
        baselineStartNanos = nowNanos;
        nextBaselineSampleNanos = nowNanos;
        baselineFrames = 0;
        lastObservationNanos = nowNanos;
        baselineReady = false;
        clearDebt();
    }

    public synchronized long resetCount() { return resetCount; }

    /** Observe queue + AudioTrack pending frames. +1 deletes, -1 inserts, and edits stay <=1000ppm. */
    public synchronized int observe(long nowNanos, long bufferedFrames, boolean playing,
            boolean discontinuity) {
        if (bufferedFrames < 0) throw new IllegalArgumentException("negative Float32 waterline");
        if (!playing || discontinuity) {
            reset(nowNanos);
            return 0;
        }
        long elapsedNanos = nowNanos - lastObservationNanos;
        if (elapsedNanos <= 0 || elapsedNanos > MAX_SAMPLE_GAP_NANOS) {
            reset(nowNanos);
            collectBaseline(nowNanos, bufferedFrames);
            lastObservationNanos = nowNanos;
            return 0;
        }
        if (!baselineReady) {
            collectBaseline(nowNanos, bufferedFrames);
            lastObservationNanos = nowNanos;
            return 0;
        }

        double error = bufferedFrames - baselineFrames;
        double alpha = elapsedNanos / (double) (BASELINE_NANOS + elapsedNanos);
        filteredErrorFrames += (error - filteredErrorFrames) * alpha;
        long deadband = deadbandFrames(targetFrames, sampleRate);
        int direction = filteredErrorFrames > deadband ? 1
                : filteredErrorFrames < -deadband ? -1 : 0;
        if (direction == 0) {
            clearDebt();
            lastObservationNanos = nowNanos;
            return 0;
        }
        if (direction != outsideDirection) {
            outsideDirection = direction;
            outsideSinceNanos = nowNanos;
            credit = 0;
        } else if (nowNanos - outsideSinceNanos >= PERSISTENCE_NANOS) {
            long excessFrames = (long) Math.abs(filteredErrorFrames) - deadband;
            int correctionPpm = (int) Math.min(MAX_CORRECTION_PPM,
                    excessFrames * MAX_CORRECTION_PPM / Math.max(1, deadband));
            long earned = (long) sampleRate * elapsedNanos * correctionPpm / 1_000L;
            credit += direction > 0 ? -earned : earned;
        }
        lastObservationNanos = nowNanos;

        if (nowNanos - outsideSinceNanos < PERSISTENCE_NANOS) return 0;
        if (credit <= -CREDIT_PER_FRAME) {
            return 1;
        }
        if (credit >= CREDIT_PER_FRAME) {
            return -1;
        }
        return 0;
    }

    /** Commits one edit only after the Float32 sample block accepted it. */
    public synchronized void commit(int adjustment) {
        if (adjustment == 1 && credit <= -CREDIT_PER_FRAME) credit += CREDIT_PER_FRAME;
        else if (adjustment == -1 && credit >= CREDIT_PER_FRAME) credit -= CREDIT_PER_FRAME;
        else if (adjustment != 0) throw new IllegalArgumentException("unearned Float32 drift edit");
    }

    private void collectBaseline(long nowNanos, long bufferedFrames) {
        if (nowNanos >= nextBaselineSampleNanos && baselineCount < baselineSamples.length) {
            baselineSamples[baselineCount++] = bufferedFrames;
            nextBaselineSampleNanos = nowNanos + SAMPLE_NANOS;
        }
        if (nowNanos - baselineStartNanos < BASELINE_NANOS || baselineCount == 0) return;
        for (int index = 1; index < baselineCount; index++) {
            long sample = baselineSamples[index];
            int insert = index;
            while (insert > 0 && baselineSamples[insert - 1] > sample) {
                baselineSamples[insert] = baselineSamples[insert - 1];
                insert--;
            }
            baselineSamples[insert] = sample;
        }
        baselineFrames = baselineSamples[baselineCount / 2];
        baselineReady = true;
        filteredErrorFrames = 0;
        clearDebt();
    }

    private static long deadbandFrames(long targetFrames, int sampleRate) {
        long minimum = Math.max(1, (sampleRate + 999L) / 1_000L);
        long maximum = Math.max(minimum, (sampleRate * 10L + 999L) / 1_000L);
        long quarter = Math.max(1, targetFrames / 4);
        return Math.max(minimum, Math.min(maximum, quarter));
    }

    private void clearDebt() {
        credit = 0;
        outsideSinceNanos = 0;
        outsideDirection = 0;
    }
}
