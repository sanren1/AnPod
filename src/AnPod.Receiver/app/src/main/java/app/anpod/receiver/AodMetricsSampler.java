package app.anpod.receiver;

/**
 * Pure JVM rate/latency arithmetic for the AOD metrics bus.
 *
 * <p>All methods take an explicit {@code nowNanos} so tests can drive the clock without Android.
 */
final class AodMetricsSampler {
    static final long MIN_WINDOW_NANOS = 500_000_000L;
    private static final long LOSS_WINDOW_NANOS = 60_000_000_000L;
    private static final long MAX_LOSS_SAMPLE_GAP_NANOS = 2_500_000_000L;
    private static final int LOSS_SAMPLE_CAPACITY = 122;

    private long windowStartNanos = -1;
    private long windowBytes;
    private long lastTotalBytes = -1;
    private long lastAudioNanos = -1;
    private Long lastBitsPerSecond;
    private final long[] lossStarts = new long[LOSS_SAMPLE_CAPACITY];
    private final long[] lossEnds = new long[LOSS_SAMPLE_CAPACITY];
    private final long[] lossFrames = new long[LOSS_SAMPLE_CAPACITY];
    private final long[] eligibleFrames = new long[LOSS_SAMPLE_CAPACITY];
    private int lossHead;
    private int lossSize;
    private long previousLossFrames = -1;
    private long previousEligibleFrames = -1;
    private long previousLossSampleNanos = -1;
    private Float lastLossPercent;
    private static final int SOURCE_LOSS_CAPACITY = 256;
    private final long[] sourceLossSegments = new long[SOURCE_LOSS_CAPACITY];
    private final long[] sourceLossStarts = new long[SOURCE_LOSS_CAPACITY];
    private final long[] sourceLossEnds = new long[SOURCE_LOSS_CAPACITY];
    private int sourceLossSize;
    private long sourceTimelineBaseline = -1;
    private long sourceTimelineThrough = -1;
    private final long[] unknownSourceStarts = new long[64];
    private final long[] unknownSourceEnds = new long[64];
    private int unknownSourceSize;
    private long unknownOverflowThrough = -1L;
    private long sourceLossOverflowThrough = -1L;

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

    synchronized boolean receivedAudioRecently(long nowNanos, long windowNanos) {
        return lastAudioNanos >= 0 && nowNanos - lastAudioNanos <= windowNanos;
    }

    /** Returns a weighted trailing 60-second loss rate only across continuously known samples. */
    synchronized Float recordCumulativeLoss(long loss, long eligible, boolean complete, long nowNanos) {
        if (!complete || loss < 0 || eligible < 0 || loss > eligible) {
            clearLossWindow();
            return null;
        }
        if (previousLossSampleNanos < 0) {
            setLossBaseline(loss, eligible, nowNanos);
            return null;
        }
        long elapsed = nowNanos - previousLossSampleNanos;
        if (elapsed < MIN_WINDOW_NANOS) return lastLossPercent;
        if (elapsed > MAX_LOSS_SAMPLE_GAP_NANOS || loss < previousLossFrames
                || eligible < previousEligibleFrames) {
            clearLossWindow();
            setLossBaseline(loss, eligible, nowNanos);
            return null;
        }
        long lossDelta = loss - previousLossFrames;
        long eligibleDelta = eligible - previousEligibleFrames;
        if (lossDelta > eligibleDelta) {
            clearLossWindow();
            setLossBaseline(loss, eligible, nowNanos);
            return null;
        }
        appendLossSample(previousLossSampleNanos, nowNanos, lossDelta, eligibleDelta);
        previousLossFrames = loss;
        previousEligibleFrames = eligible;
        previousLossSampleNanos = nowNanos;
        lastLossPercent = recentLossPercent(LOSS_WINDOW_NANOS, nowNanos);
        return lastLossPercent;
    }

    synchronized Float recentLossPercent(long windowNanos, long nowNanos) {
        LossWindow window = recentLossWindow(windowNanos, nowNanos);
        return window == null ? null : (float) (100.0 * window.lossFrames / window.eligibleFrames);
    }

    synchronized LossWindow recentLossWindow(long windowNanos, long nowNanos) {
        if (windowNanos <= 0 || lossSize == 0) return null;
        long cutoff = nowNanos - windowNanos;
        if (lossSize == 0 || lossStarts[lossHead] > cutoff) {
            return null;
        }
        double lossTotal = 0;
        double eligibleTotal = 0;
        for (int i = 0; i < lossSize; i++) {
            int index = (lossHead + i) % LOSS_SAMPLE_CAPACITY;
            long overlapStart = Math.max(lossStarts[index], cutoff);
            long overlapNanos = lossEnds[index] - overlapStart;
            if (overlapNanos <= 0) continue;
            double fraction = overlapNanos / (double) (lossEnds[index] - lossStarts[index]);
            lossTotal += lossFrames[index] * fraction;
            eligibleTotal += eligibleFrames[index] * fraction;
        }
        if (eligibleTotal <= 0) return null;
        return new LossWindow(Math.round(lossTotal), Math.round(eligibleTotal));
    }

    synchronized void recordSourceLossSnapshot(long segmentId, long logicalStart, long sourceOrigin,
            long settledThrough, int sampleRate, com.anpod.server.SourceFrameRanges.Range[] lossRanges,
            boolean complete) {
        recordSourceLossSnapshot(segmentId, logicalStart, sourceOrigin, settledThrough, sampleRate,
                lossRanges, complete, new com.anpod.server.SourceFrameRanges.Range[0]);
    }

    synchronized void recordSourceLossSnapshot(long segmentId, long logicalStart, long sourceOrigin,
            long settledThrough, int sampleRate, com.anpod.server.SourceFrameRanges.Range[] lossRanges,
            boolean complete, com.anpod.server.SourceFrameRanges.Range[] unknownRanges) {
        if (sourceOrigin == -1L || settledThrough == -1L || sampleRate <= 0
                || Long.compareUnsigned(settledThrough, sourceOrigin) < 0) return;
        long segmentEnd = logicalStart + (settledThrough - sourceOrigin);
        if (Long.compareUnsigned(segmentEnd, logicalStart) < 0) return;
        if (lossRanges == null) lossRanges = new com.anpod.server.SourceFrameRanges.Range[0];
        if (unknownRanges == null) unknownRanges = new com.anpod.server.SourceFrameRanges.Range[0];
        if (sourceTimelineBaseline < 0) sourceTimelineBaseline = logicalStart;
        if (sourceTimelineThrough < segmentEnd) sourceTimelineThrough = segmentEnd;
        int kept = 0;
        for (int i = 0; i < sourceLossSize; i++) {
            if (sourceLossSegments[i] != segmentId) {
                sourceLossSegments[kept] = sourceLossSegments[i];
                sourceLossStarts[kept] = sourceLossStarts[i];
                sourceLossEnds[kept++] = sourceLossEnds[i];
            }
        }
        sourceLossSize = kept;
        long cutoff = Math.max(sourceTimelineBaseline, sourceTimelineThrough - 60L * sampleRate);
        int retained = 0;
        for (int i = 0; i < sourceLossSize; i++) {
            if (Long.compareUnsigned(sourceLossEnds[i], cutoff) <= 0) continue;
            sourceLossSegments[retained] = sourceLossSegments[i];
            sourceLossStarts[retained] = Long.compareUnsigned(sourceLossStarts[i], cutoff) < 0
                    ? cutoff : sourceLossStarts[i];
            sourceLossEnds[retained++] = sourceLossEnds[i];
        }
        sourceLossSize = retained;
        if (sourceLossOverflowThrough != -1L
                && Long.compareUnsigned(sourceLossOverflowThrough, cutoff) <= 0)
            sourceLossOverflowThrough = -1L;
        if (unknownOverflowThrough != -1L
                && Long.compareUnsigned(unknownOverflowThrough, cutoff) <= 0)
            unknownOverflowThrough = -1L;        int retainedUnknown = 0;
        for (int i = 0; i < unknownSourceSize; i++) {
            if (Long.compareUnsigned(unknownSourceEnds[i], cutoff) <= 0) continue;
            unknownSourceStarts[retainedUnknown] = Long.compareUnsigned(unknownSourceStarts[i], cutoff) < 0
                    ? cutoff : unknownSourceStarts[i];
            unknownSourceEnds[retainedUnknown++] = unknownSourceEnds[i];
        }
        unknownSourceSize = retainedUnknown;
        for (com.anpod.server.SourceFrameRanges.Range range : lossRanges) {
            long start = Long.compareUnsigned(range.start, sourceOrigin) < 0 ? sourceOrigin : range.start;
            long end = range.start + range.frameCount;
            if (Long.compareUnsigned(end, settledThrough) > 0) end = settledThrough;
            if (Long.compareUnsigned(start, end) >= 0) continue;
            long logicalRangeStart = logicalStart + (start - sourceOrigin);
            long logicalRangeEnd = logicalStart + (end - sourceOrigin);
            if (Long.compareUnsigned(logicalRangeEnd, cutoff) <= 0) continue;
            if (Long.compareUnsigned(logicalRangeStart, cutoff) < 0) logicalRangeStart = cutoff;
            if (sourceLossSize == SOURCE_LOSS_CAPACITY) {
                if (sourceLossOverflowThrough == -1L
                        || Long.compareUnsigned(logicalRangeEnd, sourceLossOverflowThrough) > 0)
                    sourceLossOverflowThrough = logicalRangeEnd;
                return;
            }
            sourceLossSegments[sourceLossSize] = segmentId;
            sourceLossStarts[sourceLossSize] = logicalRangeStart;
            sourceLossEnds[sourceLossSize++] = logicalRangeEnd;
        }
        for (com.anpod.server.SourceFrameRanges.Range range : unknownRanges) {
            long start = Long.compareUnsigned(range.start, sourceOrigin) < 0 ? sourceOrigin : range.start;
            long end = range.start + range.frameCount;
            if (Long.compareUnsigned(end, settledThrough) > 0) end = settledThrough;
            if (Long.compareUnsigned(start, end) >= 0) continue;
            recordUnknownSourceRange(logicalStart + (start - sourceOrigin),
                    logicalStart + (end - sourceOrigin), sampleRate);
        }
        if (!complete && unknownRanges.length == 0 && segmentEnd > logicalStart)
            recordUnknownSourceRange(logicalStart, segmentEnd, sampleRate);
    }

    synchronized void recordUnknownSourceRange(long start, long end, int sampleRate) {
        if (end == -1L || sampleRate <= 0) return;
        if (Long.compareUnsigned(end, start) <= 0) return;
        if (sourceTimelineBaseline < 0 || Long.compareUnsigned(start, sourceTimelineBaseline) < 0)
            sourceTimelineBaseline = start;
        int index = 0;
        while (index < unknownSourceSize && Long.compareUnsigned(unknownSourceEnds[index], start) < 0) index++;
        long mergedStart = start;
        long mergedEnd = end;
        int after = index;
        while (after < unknownSourceSize
                && Long.compareUnsigned(unknownSourceStarts[after], mergedEnd) <= 0) {
            if (Long.compareUnsigned(unknownSourceStarts[after], mergedStart) < 0)
                mergedStart = unknownSourceStarts[after];
            if (Long.compareUnsigned(unknownSourceEnds[after], mergedEnd) > 0)
                mergedEnd = unknownSourceEnds[after];
            after++;
        }
        int removed = after - index;
        if (removed == 0 && unknownSourceSize == unknownSourceStarts.length) {
            if (unknownOverflowThrough == -1L
                    || Long.compareUnsigned(mergedEnd, unknownOverflowThrough) > 0)
                unknownOverflowThrough = mergedEnd;
            return;
        }        if (removed == 0) {
            System.arraycopy(unknownSourceStarts, index, unknownSourceStarts, index + 1, unknownSourceSize - index);
            System.arraycopy(unknownSourceEnds, index, unknownSourceEnds, index + 1, unknownSourceSize - index);
            unknownSourceSize++;
        } else if (removed > 1) {
            System.arraycopy(unknownSourceStarts, after, unknownSourceStarts, index + 1, unknownSourceSize - after);
            System.arraycopy(unknownSourceEnds, after, unknownSourceEnds, index + 1, unknownSourceSize - after);
            unknownSourceSize -= removed - 1;
        }
        unknownSourceStarts[index] = mergedStart;
        unknownSourceEnds[index] = mergedEnd;
    }
    synchronized LossWindow recentSourceLossWindow(long windowNanos, int sampleRate) {
        if (windowNanos <= 0 || sampleRate <= 0 || sourceTimelineBaseline < 0
                || sourceTimelineThrough < sourceTimelineBaseline) return null;
        long windowFrames = Math.max(1, Math.round(windowNanos * (sampleRate / 1_000_000_000.0)));
        long cutoff = Math.max(sourceTimelineBaseline, sourceTimelineThrough - windowFrames);
        long eligible = sourceTimelineThrough - cutoff;
        if (eligible <= 0) return null;
        if (sourceLossOverflowThrough != -1L
                && Long.compareUnsigned(sourceLossOverflowThrough, cutoff) > 0) return null;
        if (unknownOverflowThrough != -1L
                && Long.compareUnsigned(unknownOverflowThrough, cutoff) > 0) return null;
        for (int i = 0; i < unknownSourceSize; i++)
            if (Long.compareUnsigned(unknownSourceStarts[i], sourceTimelineThrough) < 0
                    && Long.compareUnsigned(unknownSourceEnds[i], cutoff) > 0) return null;
        long loss = 0;
        for (int i = 0; i < sourceLossSize; i++) {
            long start = Math.max(sourceLossStarts[i], cutoff);
            long end = Math.min(sourceLossEnds[i], sourceTimelineThrough);
            if (end > start) loss += end - start;
        }
        return new LossWindow(loss, eligible);
    }
    synchronized void resetLossWindow() {
        clearLossWindow();
        sourceLossSize = 0;
        sourceTimelineBaseline = -1;
        sourceTimelineThrough = -1;
            unknownSourceSize = 0;
            unknownOverflowThrough = -1L;
            sourceLossOverflowThrough = -1L;
    }

    synchronized void excludeLossTime(long pausedNanos) {
        if (pausedNanos <= 0) return;
        if (previousLossSampleNanos >= 0) previousLossSampleNanos += pausedNanos;
        for (int i = 0; i < lossSize; i++) {
            int index = (lossHead + i) % LOSS_SAMPLE_CAPACITY;
            lossStarts[index] += pausedNanos;
            lossEnds[index] += pausedNanos;
        }
    }
    private void appendLossSample(long start, long end, long loss, long eligible) {
        if (lossSize == LOSS_SAMPLE_CAPACITY) {
            lossHead = (lossHead + 1) % LOSS_SAMPLE_CAPACITY;
            lossSize--;
        }
        int index = (lossHead + lossSize++) % LOSS_SAMPLE_CAPACITY;
        lossStarts[index] = start;
        lossEnds[index] = end;
        lossFrames[index] = loss;
        eligibleFrames[index] = eligible;
    }

    private void setLossBaseline(long loss, long eligible, long nowNanos) {
        previousLossFrames = loss;
        previousEligibleFrames = eligible;
        previousLossSampleNanos = nowNanos;
    }

    private void clearLossWindow() {
        lossHead = 0;
        lossSize = 0;
        previousLossFrames = -1;
        previousEligibleFrames = -1;
        previousLossSampleNanos = -1;
        lastLossPercent = null;
    }

    static final class LossWindow {
        final long lossFrames;
        final long eligibleFrames;
        LossWindow(long lossFrames, long eligibleFrames) {
            this.lossFrames = lossFrames;
            this.eligibleFrames = eligibleFrames;
        }
    }

    synchronized void reset(long nowNanos) {
        resetPayloadRate(nowNanos);
        clearLossWindow();
    }

    synchronized void resetPayloadRate(long nowNanos) {
        windowStartNanos = nowNanos;
        windowBytes = 0;
        lastTotalBytes = -1;
        lastAudioNanos = -1;
        lastBitsPerSecond = null;
    }

    /** Buffer latency for the currently observable receive-side frames; null when not measurable. */
    static Float bufferMs(long queuedOrBufferedFrames, long pendingTrackFrames, int sampleRate) {
        if (sampleRate <= 0 || queuedOrBufferedFrames < 0 || pendingTrackFrames < 0) return null;
        return (float) (1000.0 * (queuedOrBufferedFrames + pendingTrackFrames) / sampleRate);
    }
}
