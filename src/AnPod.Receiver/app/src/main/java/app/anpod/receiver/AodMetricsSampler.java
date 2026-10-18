package app.anpod.receiver;

/**
 * Pure JVM rate/loss/latency arithmetic for the AOD metrics bus.
 *
 * <p>All methods take an explicit {@code nowNanos} so tests can drive the clock without Android.
 */
final class AodMetricsSampler {
    static final long MIN_WINDOW_NANOS = 500_000_000L;

    private long windowStartNanos = -1;
    private long windowBytes;
    private long lastTotalBytes = -1;
    private long acceptedFrames;
    private long concealedFrames;
    private long lastAudioNanos = -1;
    private Long lastBitsPerSecond;

    /** Feeds a monotonically growing cumulative byte counter (UDP payload or USB PCM input). */
    synchronized void recordCumulativeBytes(long totalBytes, long nowNanos) {
        if (totalBytes < 0) return;
        if (lastTotalBytes < 0) {
            lastTotalBytes = totalBytes;
            if (windowStartNanos < 0) windowStartNanos = nowNanos;
            return;
        }
        long delta = totalBytes - lastTotalBytes;
        lastTotalBytes = totalBytes;
        if (delta <= 0) return;
        if (windowStartNanos < 0) windowStartNanos = nowNanos;
        windowBytes += delta;
        lastAudioNanos = nowNanos;
    }

    /** Cumulative counters from the UDP queue; replaces previous totals rather than adding. */
    synchronized void updateFrameTotals(long accepted, long concealed) {
        acceptedFrames = Math.max(0, accepted);
        concealedFrames = Math.max(0, concealed);
    }

    /** Returns null until the first full window; afterwards 0 is a meaningful "no new bytes". */
    synchronized Long bitsPerSecond(long nowNanos) {
        if (windowStartNanos < 0 || nowNanos - windowStartNanos < MIN_WINDOW_NANOS) return lastBitsPerSecond;
        double seconds = (nowNanos - windowStartNanos) / 1_000_000_000.0;
        long bitsPerSecond = Math.round(windowBytes * 8 / seconds);
        windowStartNanos = nowNanos;
        windowBytes = 0;
        lastBitsPerSecond = bitsPerSecond;
        return bitsPerSecond;
    }

    /** UDP only; returns null when no audio frame has been accounted for yet. */
    synchronized Float lossPercent() {
        long expected = acceptedFrames + concealedFrames;
        if (expected <= 0) return null;
        return (float) (100.0 * concealedFrames / expected);
    }

    synchronized boolean receivedAudioRecently(long nowNanos, long windowNanos) {
        return lastAudioNanos >= 0 && nowNanos - lastAudioNanos <= windowNanos;
    }

    synchronized void reset(long nowNanos) {
        windowStartNanos = nowNanos;
        windowBytes = 0;
        lastTotalBytes = -1;
        acceptedFrames = 0;
        concealedFrames = 0;
        lastAudioNanos = -1;
        lastBitsPerSecond = null;
    }

    /** Buffer latency for the currently observable receive-side frames; null when not measurable. */
    static Float bufferMs(long queuedOrBufferedFrames, long pendingTrackFrames, int sampleRate) {
        if (sampleRate <= 0 || queuedOrBufferedFrames < 0 || pendingTrackFrames < 0) return null;
        return (float) (1000.0 * (queuedOrBufferedFrames + pendingTrackFrames) / sampleRate);
    }
}
