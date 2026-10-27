package com.anpod.server;

/** Fixed-capacity normalized source-frame ranges. */
public final class SourceFrameRanges {
    public static final int CAPACITY = 64;
    private final long[] starts = new long[CAPACITY];
    private final long[] ends = new long[CAPACITY];
    private int size;
    private long revision;
    private long coveredFrames;
    private boolean complete = true;

    public boolean add(long start, long frameCount) {
        return addAndCountNewFrames(start, frameCount) >= 0;
    }

    /** Returns newly covered frames, or -1 when the bounded ledger overflowed. */
    public long addAndCountNewFrames(long start, long frameCount) {
        if (frameCount <= 0) throw new IllegalArgumentException("frameCount must be positive");
        long end = start + frameCount;
        if (Long.compareUnsigned(end, start) <= 0)
            throw new IllegalArgumentException("source frame range overflows");
        int first = 0;
        while (first < size && Long.compareUnsigned(ends[first], start) < 0) first++;
        long mergedStart = start;
        long mergedEnd = end;
        int after = first;
        long removedFrames = 0;
        while (after < size && Long.compareUnsigned(starts[after], mergedEnd) <= 0) {
            if (Long.compareUnsigned(starts[after], mergedStart) < 0) mergedStart = starts[after];
            if (Long.compareUnsigned(ends[after], mergedEnd) > 0) mergedEnd = ends[after];
            removedFrames += ends[after] - starts[after];
            after++;
        }
        if (first == after && size == CAPACITY) {
            complete = false;
            revision++;
            return -1;
        }
        int removed = after - first;
        boolean changed = removed == 0 || removed > 1
                || starts[first] != mergedStart || ends[first] != mergedEnd;
        long addedFrames = (mergedEnd - mergedStart) - removedFrames;
        if (removed == 0) {
            System.arraycopy(starts, first, starts, first + 1, size - first);
            System.arraycopy(ends, first, ends, first + 1, size - first);
            size++;
        } else if (removed > 1) {
            System.arraycopy(starts, after, starts, first + 1, size - after);
            System.arraycopy(ends, after, ends, first + 1, size - after);
            size -= removed - 1;
        }
        starts[first] = mergedStart;
        ends[first] = mergedEnd;
        coveredFrames += addedFrames;
        if (changed) revision++;
        return addedFrames;
    }

    /** Removes covered frames in [start, start + frameCount), returning removed coverage or -1 on overflow. */
    public long removeAndCountFrames(long start, long frameCount) {
        if (frameCount <= 0) throw new IllegalArgumentException("frameCount must be positive");
        long end = start + frameCount;
        if (Long.compareUnsigned(end, start) <= 0)
            throw new IllegalArgumentException("source frame range overflows");
        long[] nextStarts = new long[CAPACITY];
        long[] nextEnds = new long[CAPACITY];
        int nextSize = 0;
        long removedFrames = 0;
        for (int i = 0; i < size; i++) {
            long rangeStart = starts[i];
            long rangeEnd = ends[i];
            long overlapStart = Long.compareUnsigned(rangeStart, start) > 0 ? rangeStart : start;
            long overlapEnd = Long.compareUnsigned(rangeEnd, end) < 0 ? rangeEnd : end;
            if (Long.compareUnsigned(overlapStart, overlapEnd) >= 0) {
                if (nextSize == CAPACITY) {
                    complete = false;
                    revision++;
                    return -1;
                }
                nextStarts[nextSize] = rangeStart;
                nextEnds[nextSize++] = rangeEnd;
                continue;
            }
            removedFrames += overlapEnd - overlapStart;
            if (Long.compareUnsigned(rangeStart, overlapStart) < 0) {
                if (nextSize == CAPACITY) {
                    complete = false;
                    revision++;
                    return -1;
                }
                nextStarts[nextSize] = rangeStart;
                nextEnds[nextSize++] = overlapStart;
            }
            if (Long.compareUnsigned(overlapEnd, rangeEnd) < 0) {
                if (nextSize == CAPACITY) {
                    complete = false;
                    revision++;
                    return -1;
                }
                nextStarts[nextSize] = overlapEnd;
                nextEnds[nextSize++] = rangeEnd;
            }
        }
        if (removedFrames != 0) {
            System.arraycopy(nextStarts, 0, starts, 0, nextSize);
            System.arraycopy(nextEnds, 0, ends, 0, nextSize);
            size = nextSize;
            coveredFrames -= removedFrames;
            revision++;
        }
        return removedFrames;
    }
    public long revision() { return revision; }
    public int size() { return size; }
    public long start(int index) { return starts[index]; }
    public long end(int index) { return ends[index]; }
    public long coveredFrames() { return coveredFrames; }
    public boolean complete() { return complete; }

    public void clear() {
        if (size != 0 || !complete || coveredFrames != 0) revision++;
        size = 0;
        coveredFrames = 0;
        complete = true;
    }

    public Range[] snapshot() {
        Range[] result = new Range[size];
        for (int i = 0; i < size; i++) result[i] = new Range(starts[i], ends[i] - starts[i]);
        return result;
    }

    public static final class Range {
        public final long start;
        public final long frameCount;
        public Range(long start, long frameCount) { this.start = start; this.frameCount = frameCount; }
    }
}
