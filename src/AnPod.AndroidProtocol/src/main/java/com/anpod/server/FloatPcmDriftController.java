package com.anpod.server;

/** Frame-count-only waterline controller for Float32 playout. */
public final class FloatPcmDriftController {
    private static final long FILTER_TIME_CONSTANT_NANOS = 1_000_000_000L;
    private static final long PERSISTENCE_NANOS = 2_000_000_000L;
    private static final long MAX_SAMPLE_GAP_NANOS = 100_000_000L;
    private static final long CREDIT_PER_FRAME = 1_000_000_000_000L;
    private static final int MAX_CORRECTION_PPM = 1_000;

    private final int sampleRate;
    private final int targetFrames;
    private long lastObservationNanos;
    private long outsideSinceNanos;
    private int outsideDirection;
    private long credit;
    private double filteredErrorFrames;
    private long resetCount;
    private long observedFrames = -1;
    private int correctionPpm;
    private int resetReason = 1;
    private boolean hasFilteredError;

    public FloatPcmDriftController(int sampleRate, int targetFrames) {
        if (sampleRate <= 0 || targetFrames <= 0)
            throw new IllegalArgumentException("invalid Float32 drift target");
        this.sampleRate = sampleRate;
        this.targetFrames = targetFrames;
    }

    public synchronized void reset(long nowNanos) {
        reset(nowNanos, 1);
    }

    public synchronized void reset(long nowNanos, int reason) {
        resetCount++;
        lastObservationNanos = nowNanos;
        observedFrames = -1;
        resetReason = reason;
        correctionPpm = 0;
        filteredErrorFrames = 0;
        hasFilteredError = false;
        clearDebt();
    }

    public synchronized long resetCount() { return resetCount; }
    public int targetFrames() { return targetFrames; }
    public synchronized long observedFrames() { return observedFrames; }
    public synchronized boolean hasFilteredError() { return hasFilteredError; }
    public synchronized long errorFrames() { return Math.round(filteredErrorFrames); }
    public synchronized int correctionPpm() { return correctionPpm; }
    public synchronized int resetReason() { return resetReason; }

    /** Observe source queue + current block. +1 deletes, -1 inserts, at no more than 1000ppm. */
    public synchronized int observe(long nowNanos, long bufferedFrames, boolean playing,
            boolean discontinuity) {
        if (bufferedFrames < 0) throw new IllegalArgumentException("negative Float32 waterline");
        observedFrames = bufferedFrames;
        if (!playing || discontinuity) {
            reset(nowNanos, discontinuity ? 4 : 2);
            observedFrames = bufferedFrames;
            return 0;
        }
        long elapsedNanos = nowNanos - lastObservationNanos;
        if (elapsedNanos <= 0 || elapsedNanos > MAX_SAMPLE_GAP_NANOS) {
            reset(nowNanos, 8);
            observedFrames = bufferedFrames;
            return 0;
        }

        double error = bufferedFrames - targetFrames;
        double alpha = elapsedNanos / (double) (FILTER_TIME_CONSTANT_NANOS + elapsedNanos);
        filteredErrorFrames += (error - filteredErrorFrames) * alpha;
        hasFilteredError = true;
        long deadband = deadbandFrames(targetFrames, sampleRate);
        int direction = filteredErrorFrames > deadband ? 1
                : filteredErrorFrames < -deadband ? -1 : 0;
        if (direction == 0) {
            correctionPpm = 0;
            clearDebt();
            lastObservationNanos = nowNanos;
            return 0;
        }
        if (direction != outsideDirection) {
            outsideDirection = direction;
            outsideSinceNanos = nowNanos;
            credit = 0;
            correctionPpm = 0;
        } else {
            long excessFrames = (long) Math.abs(filteredErrorFrames) - deadband;
            correctionPpm = (int) Math.min(MAX_CORRECTION_PPM,
                    excessFrames * MAX_CORRECTION_PPM / Math.max(1, deadband));
            if (direction < 0) correctionPpm = -correctionPpm;
            if (nowNanos - outsideSinceNanos >= PERSISTENCE_NANOS) {
                long earned = (long) sampleRate * elapsedNanos * Math.abs(correctionPpm) / 1_000L;
                credit += direction > 0 ? -earned : earned;
            }
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
