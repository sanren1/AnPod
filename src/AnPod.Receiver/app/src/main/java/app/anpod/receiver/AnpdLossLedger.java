package app.anpod.receiver;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/** Authenticated ANPD source-drop snapshot payload. */
final class AnpdLossLedger {
    static final int VERSION = 3;
    static final int MAX_INTERVALS = 64;
    private static final int V2_HEADER_BYTES = 36;
    private static final int HEADER_BYTES = 52;

    final long periodEpoch;
    final long revision;
    final long captureTail;
    final long settledThrough;
    final long sourceSegmentGeneration;
    final long sourceSegmentStart;
    final boolean hasSourceSegmentStart;
    final boolean complete;
    final Interval[] intervals;

    private AnpdLossLedger(long periodEpoch, long revision, long captureTail, long settledThrough,
            long sourceSegmentGeneration, long sourceSegmentStart, boolean hasSourceSegmentStart,
            boolean complete, Interval[] intervals) {
        this.periodEpoch = periodEpoch;
        this.revision = revision;
        this.captureTail = captureTail;
        this.settledThrough = settledThrough;
        this.sourceSegmentGeneration = sourceSegmentGeneration;
        this.sourceSegmentStart = sourceSegmentStart;
        this.hasSourceSegmentStart = hasSourceSegmentStart;
        this.complete = complete;
        this.intervals = intervals;
    }

    static AnpdLossLedger decode(byte[] payload) throws UdpPacketDecoder.ProtocolException {
        if (payload == null || payload.length < V2_HEADER_BYTES) throw invalid();
        ByteBuffer bytes = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN);
        int version = bytes.get() & 0xff;
        int flags = bytes.get() & 0xff;
        int count = bytes.get() & 0xff;
        int reserved = bytes.get() & 0xff;
        int headerBytes = version == 2 ? V2_HEADER_BYTES : version == VERSION ? HEADER_BYTES : -1;
        if (headerBytes < 0 || (flags & ~1) != 0 || count > MAX_INTERVALS || reserved != 0
                || payload.length != headerBytes + count * 16) throw invalid();
        long periodEpoch = bytes.getLong();
        long revision = bytes.getLong();
        long captureTail = bytes.getLong();
        long settledThrough = bytes.getLong();
        if (Long.compareUnsigned(settledThrough, captureTail) > 0) throw invalid();
        long segmentGeneration = version == VERSION ? bytes.getLong() : 0L;
        long segmentStart = version == VERSION ? bytes.getLong() : 0L;
        boolean hasSegmentStart = version == VERSION;
        if (hasSegmentStart && (Long.compareUnsigned(segmentStart, settledThrough) > 0
                || Long.compareUnsigned(segmentStart, captureTail) > 0)) throw invalid();
        Interval[] intervals = new Interval[count];
        long previousEnd = 0;
        boolean hasPrevious = false;
        for (int i = 0; i < count; i++) {
            bytes.position(headerBytes + i * 16);
            long start = bytes.getLong();
            long frameCount = bytes.getLong();
            long end = start + frameCount;
            if (frameCount == 0 || Long.compareUnsigned(end, start) <= 0
                    || Long.compareUnsigned(end, captureTail) > 0
                    || (hasPrevious && Long.compareUnsigned(start, previousEnd) <= 0)) throw invalid();
            intervals[i] = new Interval(start, frameCount);
            previousEnd = end;
            hasPrevious = true;
        }
        return new AnpdLossLedger(periodEpoch, revision, captureTail, settledThrough,
                segmentGeneration, segmentStart, hasSegmentStart, (flags & 1) != 0, intervals);
    }

    byte[] encode() {
        ByteBuffer bytes = ByteBuffer.allocate(HEADER_BYTES + intervals.length * 16)
                .order(ByteOrder.BIG_ENDIAN);
        bytes.put((byte) VERSION).put((byte) (complete ? 1 : 0)).put((byte) intervals.length).put((byte) 0);
        bytes.putLong(periodEpoch).putLong(revision).putLong(captureTail).putLong(settledThrough);
        bytes.putLong(sourceSegmentGeneration).putLong(sourceSegmentStart);
        for (Interval interval : intervals) bytes.putLong(interval.start).putLong(interval.frameCount);
        return bytes.array();
    }

    static final class Store {
        private long periodEpoch;
        private long revision;
        private long captureTail;
        private long settledThrough;
        private long sourceSegmentGeneration;
        private long sourceSegmentStart;
        private boolean hasSourceSegmentStart;
        private boolean hasRevision;
        private boolean complete = true;
        private boolean lastSnapshotComplete;
        private Interval[] intervals = new Interval[0];

        synchronized boolean apply(AnpdLossLedger next) throws UdpPacketDecoder.ProtocolException {
            boolean epochChanged = false;
            if (hasRevision) {
                int epochOrder = Long.compareUnsigned(next.periodEpoch, periodEpoch);
                if (epochOrder < 0) return false;
                if (epochOrder > 0) {
                    if (Long.compareUnsigned(next.revision, revision) <= 0
                            || Long.compareUnsigned(next.captureTail, captureTail) < 0
                            || Long.compareUnsigned(next.settledThrough, settledThrough) < 0) throw invalid();
                    intervals = next.intervals.clone();
                    periodEpoch = next.periodEpoch;
                    revision = next.revision;
                    captureTail = next.captureTail;
                    settledThrough = next.settledThrough;
                    sourceSegmentGeneration = next.sourceSegmentGeneration;
                    sourceSegmentStart = next.sourceSegmentStart;
                    hasSourceSegmentStart = next.hasSourceSegmentStart;
                    complete = next.complete;
                    lastSnapshotComplete = next.complete;
                    epochChanged = true;
                } else {
                    int order = Long.compareUnsigned(next.revision, revision);
                    if (order < 0) return false;
                    if (order == 0) {
                        if (next.captureTail != captureTail || next.settledThrough != settledThrough
                                || next.sourceSegmentGeneration != sourceSegmentGeneration
                                || next.sourceSegmentStart != sourceSegmentStart
                                || next.hasSourceSegmentStart != hasSourceSegmentStart
                                || next.complete != lastSnapshotComplete || !sameIntervals(next.intervals, intervals))
                            throw invalid();
                        return false;
                    }
                    if (Long.compareUnsigned(next.captureTail, captureTail) < 0
                            || Long.compareUnsigned(next.settledThrough, settledThrough) < 0) throw invalid();
                    validateSegmentAdvance(next);
                    for (Interval previous : intervals)
                        if (!covers(next.intervals, previous)) throw invalid();
                    intervals = next.intervals.clone();
                    revision = next.revision;
                    captureTail = next.captureTail;
                    settledThrough = next.settledThrough;
                    sourceSegmentGeneration = next.sourceSegmentGeneration;
                    sourceSegmentStart = next.sourceSegmentStart;
                    hasSourceSegmentStart = next.hasSourceSegmentStart;
                    complete &= next.complete;
                    lastSnapshotComplete = next.complete;
                }
            } else {
                intervals = next.intervals.clone();
                periodEpoch = next.periodEpoch;
                revision = next.revision;
                captureTail = next.captureTail;
                settledThrough = next.settledThrough;
                sourceSegmentGeneration = next.sourceSegmentGeneration;
                sourceSegmentStart = next.sourceSegmentStart;
                hasSourceSegmentStart = next.hasSourceSegmentStart;
                complete = next.complete;
                lastSnapshotComplete = next.complete;
                hasRevision = true;
            }
            hasRevision = true;
            return epochChanged;
        }
        synchronized long periodEpoch() { return hasRevision ? periodEpoch : -1L; }
        synchronized long appliedRevision() { return hasRevision ? revision : -1L; }
        synchronized long captureTail() { return hasRevision ? captureTail : -1L; }
        synchronized long settledThrough() { return hasRevision ? settledThrough : -1L; }
        synchronized boolean complete() { return hasRevision && complete; }
        synchronized long sourceSegmentGeneration() { return hasRevision ? sourceSegmentGeneration : -1L; }
        synchronized long sourceSegmentStart() { return hasRevision && hasSourceSegmentStart ? sourceSegmentStart : -1L; }
        synchronized Interval[] intervals() { return intervals.clone(); }

        private void validateSegmentAdvance(AnpdLossLedger next) throws UdpPacketDecoder.ProtocolException {
            if (hasSourceSegmentStart && next.hasSourceSegmentStart) {
                int generationOrder = Long.compareUnsigned(next.sourceSegmentGeneration, sourceSegmentGeneration);
                if (generationOrder < 0 || (generationOrder == 0 && next.sourceSegmentStart != sourceSegmentStart)
                        || (generationOrder > 0 && Long.compareUnsigned(next.sourceSegmentStart, settledThrough) < 0))
                    throw invalid();
            }
        }

        private static boolean sameIntervals(Interval[] left, Interval[] right) {
            if (left.length != right.length) return false;
            for (int i = 0; i < left.length; i++)
                if (left[i].start != right[i].start || left[i].frameCount != right[i].frameCount) return false;
            return true;
        }

        private static boolean covers(Interval[] snapshot, Interval wanted) {
            long wantedEnd = wanted.start + wanted.frameCount;
            for (Interval interval : snapshot) {
                long end = interval.start + interval.frameCount;
                if (Long.compareUnsigned(interval.start, wanted.start) <= 0
                        && Long.compareUnsigned(end, wantedEnd) >= 0) return true;
                if (Long.compareUnsigned(interval.start, wanted.start) > 0) return false;
            }
            return false;
        }
    }
    private static UdpPacketDecoder.ProtocolException invalid() {
        return new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
    }

    static final class Interval {
        final long start;
        final long frameCount;
        Interval(long start, long frameCount) { this.start = start; this.frameCount = frameCount; }
    }
}
