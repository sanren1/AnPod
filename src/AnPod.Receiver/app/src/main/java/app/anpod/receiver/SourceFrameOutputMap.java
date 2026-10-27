package app.anpod.receiver;

import com.anpod.server.SourceFrameRanges;

/** Bounded mapping from successful output writes back to source-frame ranges. */
final class SourceFrameOutputMap {
    static final int CAPACITY = 64;
    private final long[] outputStarts = new long[CAPACITY];
    private final long[] sourceStarts = new long[CAPACITY];
    private final int[] frames = new int[CAPACITY];
    private final boolean[] synthetic = new boolean[CAPACITY];
    private final boolean[] unknownMapping = new boolean[CAPACITY];
    private final long[] gapStarts = new long[CAPACITY];
    private final long[] gapEnds = new long[CAPACITY];

    private int gapCount;
    private boolean gapTrackingComplete = true;
    private boolean hasSettledSourceThrough;
    private int head;
    private int size;
    private boolean complete = true;
    private boolean rollingSegmentInvalidated;
    private boolean rollingSourceTrackingComplete;
    private long writtenOutputFrames;
    private long lastPlayedOutputFrames;
    private long settledSourceThrough = -1;
    private long settledSourceFrames;
    private long settledGoodSourceFrames;
    private long settledSyntheticFrames;
    private long settledPcDropFrames;
    private final SourceFrameRanges accountedPcDrops = new SourceFrameRanges();
    private final SourceFrameRanges accountedReceiverDrops = new SourceFrameRanges();
    private final SourceFrameRanges accountedTransportMissing = new SourceFrameRanges();
    private final SourceFrameRanges unknownWriteRanges = new SourceFrameRanges();
    private final SourceFrameRanges settledUnknownRanges = new SourceFrameRanges();
    private final SourceFrameRanges settledGoodRanges = new SourceFrameRanges();
    private boolean eligibleOriginKnown;
    private boolean sourceSettlementKnown;
    private boolean sourceSegmentSettlementKnown;
    private boolean sourceSegmentActive;
    private boolean sourceSegmentFloorKnown;
    private boolean sourceSegmentFloorUncertaintyIsProvisional;
    private boolean sourceSegmentCompleteBeforeProvisionalFloor;
    private long sourceSegmentFloor;
    private long sourceSegmentId;
    private long sourceSegmentLogicalStart;
    private long sourceSegmentTimelineBase;
    private long sourceSegmentCaptureTail = -1L;
    private long sourceSegmentUnknownStart = -1L;
    private long sourceSegmentUnknownEnd = -1L;
    private long sourceSegmentSettledThrough = -1L;
    private long eligibleOrigin;
    private long settledReceiverDropFrames;
    private long settledTransportMissingFrames;
    private long settledOutputDropFrames;
    private long recoveryCount;
    private long recoveryDurationMs;
    private final SourceFrameRanges outputDropRanges = new SourceFrameRanges();
    private final SourceFrameRanges pendingOutputDropRanges = new SourceFrameRanges();
    private boolean pendingOutputDropsSettled = true;

    synchronized void recordWrite(UdpPcmQueue.QueueInputStream input, int inputFrameOffset, int writtenFrames) {
        if (writtenFrames <= 0) return;
        if (input == null || !input.sourceRangeBatchComplete()) {
            markSegmentUnknown();
            writtenOutputFrames += writtenFrames;
            return;
        }
        int skipped = 0;
        int remaining = writtenFrames;
        int count = input.sourceRangeCount();
        for (int i = 0; i < count && remaining > 0; i++) {
            int rangeFrames = input.sourceRangeFrames(i);
            long rangeStart = input.sourceRangeStart(i);
            if (inputFrameOffset >= skipped + rangeFrames) {
                skipped += rangeFrames;
                continue;
            }
            int offset = Math.max(0, inputFrameOffset - skipped);
            int mapped = Math.min(remaining, rangeFrames - offset);
            if (mapped > 0) {
                append(writtenOutputFrames + writtenFrames - remaining, rangeStart + offset, mapped,
                        input.sourceRangeSynthetic(i));
                remaining -= mapped;
            }
            skipped += rangeFrames;
        }
        if (remaining != 0) markSegmentUnknown();
        writtenOutputFrames += writtenFrames;
    }

    synchronized void recordWrite(long sourceStart, int count, boolean isSynthetic) {
        if (count <= 0) return;
        append(writtenOutputFrames, sourceStart, count, isSynthetic);
        writtenOutputFrames += count;
    }

    private void append(long outputStart, long sourceStart, int count, boolean isSynthetic) {
        observeEligibleSource(sourceStart);
        if (size == CAPACITY) {
            markSegmentUnknown();
            return;
        }
        if (size > 0) {
            int last = (head + size - 1) % CAPACITY;
            long outputEnd = outputStarts[last] + frames[last];
            long sourceEnd = sourceStarts[last] + frames[last];
            if (Long.compareUnsigned(sourceStart, sourceEnd) < 0) markSegmentUnknown();
            if (outputEnd != outputStart) markSegmentUnknown();
            if (outputEnd == outputStart && sourceEnd == sourceStart && synthetic[last] == isSynthetic
                    && !unknownMapping[last]) {
                frames[last] += count;
                return;
            }
        }
        int at = (head + size++) % CAPACITY;
        outputStarts[at] = outputStart;
        sourceStarts[at] = sourceStart;
        frames[at] = count;
        synthetic[at] = isSynthetic;
        unknownMapping[at] = false;
    }

    synchronized void advancePlayed(long playedOutputFrames) {
        if (playedOutputFrames < lastPlayedOutputFrames || playedOutputFrames > writtenOutputFrames) {
            markSegmentUnknown();
            return;
        }
        lastPlayedOutputFrames = playedOutputFrames;
        while (size > 0 && Long.compareUnsigned(playedOutputFrames, outputStarts[head]) > 0) {
            int consumed = (int) Math.min(frames[head], playedOutputFrames - outputStarts[head]);
            if (unknownMapping[head]) {
                outputStarts[head] += consumed;
                frames[head] -= consumed;
                if (frames[head] == 0) {
                    head = (head + 1) % CAPACITY;
                    size--;
                }
                continue;
            }
            long sourceStart = sourceStarts[head];
            long sourceEnd = sourceStart + consumed;
            if (hasSettledSourceThrough) {
                int order = Long.compareUnsigned(sourceStart, settledSourceThrough);
                if (order > 0) recordGap(settledSourceThrough, sourceStart);
                else if (order < 0) markSegmentUnknown();
            }
            settledSourceThrough = sourceEnd;
            hasSettledSourceThrough = true;
            settledSourceFrames += consumed;
            if (synthetic[head]) {
                settledSyntheticFrames += consumed;
                recordGap(sourceStart, sourceEnd);
            } else {
                settledGoodSourceFrames += consumed;
                if (!settledGoodRanges.add(sourceStart, consumed)) markSegmentUnknown();
                reclassifyLowerPriority(sourceStart, consumed, 0);
            }
            outputStarts[head] += consumed;
            sourceStarts[head] += consumed;
            frames[head] -= consumed;
            if (frames[head] == 0) {
                head = (head + 1) % CAPACITY;
                size--;
            }
        }
    }

    private void recordGap(long start, long end) {
        if (Long.compareUnsigned(end, start) <= 0) return;
        if (gapCount > 0 && gapEnds[gapCount - 1] == start) {
            gapEnds[gapCount - 1] = end;
            return;
        }
        if (gapCount == CAPACITY) {
            gapTrackingComplete = false;
            return;
        }
        gapStarts[gapCount] = start;
        gapEnds[gapCount] = end;
        gapCount++;
    }

    synchronized void accountSourceGaps(AnpdLossLedger.Interval[] pcDrops,
            SourceFrameRanges.Range[] receiverDrops, SourceFrameRanges.Range[] startupOutputDrops,
            long settledThrough, boolean pcTrackingComplete, boolean receiverTrackingComplete,
            boolean outputDropTrackingComplete) {
        if (pcDrops == null || !sourceSegmentActive) return;
        for (AnpdLossLedger.Interval interval : pcDrops) observeEligibleSource(interval.start);
        if (receiverDrops != null)
            for (SourceFrameRanges.Range range : receiverDrops) observeEligibleSource(range.start);
        if (startupOutputDrops != null)
            for (SourceFrameRanges.Range range : startupOutputDrops) observeEligibleSource(range.start);
        for (int i = 0; i < unknownWriteRanges.size(); i++)
            observeEligibleSource(unknownWriteRanges.start(i));
        if (settledThrough == -1L || !eligibleOriginKnown) return;
        sourceSettlementKnown = true;
        sourceSegmentSettlementKnown = true;
        if (sourceSegmentSettledThrough == -1L
                || Long.compareUnsigned(settledThrough, sourceSegmentSettledThrough) > 0)
            sourceSegmentSettledThrough = settledThrough;
        accountPcDropIntervals(pcDrops, settledThrough);
        accountReceiverDropIntervals(receiverDrops, pcDrops, settledThrough);
        accountOutputDropIntervals(startupOutputDrops, receiverDrops, pcDrops, settledThrough);
        for (int i = 0; i < pendingOutputDropRanges.size(); i++)
            accountOutputDropInterval(pendingOutputDropRanges.start(i),
                    pendingOutputDropRanges.end(i) - pendingOutputDropRanges.start(i),
                    receiverDrops, pcDrops, settledThrough);
        accountPendingOutputSettlement(settledThrough);
        if (!pcTrackingComplete || !receiverTrackingComplete || !outputDropTrackingComplete)
            gapTrackingComplete = false;
        rollingSourceTrackingComplete = pcTrackingComplete && receiverTrackingComplete
                && outputDropTrackingComplete && gapTrackingComplete;
        for (int gap = 0; gap < gapCount; gap++) {
            long start = gapStarts[gap];
            long limit = minUnsigned(gapEnds[gap], settledThrough);
            if (Long.compareUnsigned(start, limit) >= 0) continue;
            long cursor = start;
            while (Long.compareUnsigned(cursor, limit) < 0) {
                AnpdLossLedger.Interval pc = containingOrNextPc(pcDrops, cursor);
                SourceFrameRanges.Range receiver = containingOrNextReceiver(receiverDrops, cursor);
                SourceFrameRanges.Range output = containingOrNextReceiver(startupOutputDrops, cursor);
                boolean inUnknown = contains(unknownWriteRanges, cursor);
                boolean inPc = pc != null && Long.compareUnsigned(pc.start, cursor) <= 0;
                boolean inReceiver = receiver != null && Long.compareUnsigned(receiver.start, cursor) <= 0;
                boolean inOutput = output != null && Long.compareUnsigned(output.start, cursor) <= 0;
                long next;
                if (inPc) {
                    next = minUnsigned(limit, pc.start + pc.frameCount);
                } else if (inReceiver) {
                    next = minUnsigned(limit, receiver.start + receiver.frameCount);
                    if (pc != null) next = minUnsigned(next, pc.start);
                    if (output != null && Long.compareUnsigned(output.start, cursor) > 0)
                        next = minUnsigned(next, output.start);
                    long added = accountedReceiverDrops.addAndCountNewFrames(cursor, next - cursor);
                    if (added < 0) gapTrackingComplete = false;
                    else settledReceiverDropFrames += added;
                    reclassifyLowerPriority(cursor, next - cursor, 1);
                } else if (inOutput) {
                    next = minUnsigned(limit, output.start + output.frameCount);
                    if (pc != null) next = minUnsigned(next, pc.start);
                    if (receiver != null) next = minUnsigned(next, receiver.start);
                } else if (inUnknown) {
                    next = minUnsigned(limit, nextBoundary(unknownWriteRanges, cursor, true));
                    long added = settledUnknownRanges.addAndCountNewFrames(cursor, next - cursor);
                    if (added < 0) markSegmentUnknown();
                } else {
                    long nextPc = pc == null ? limit : pc.start;
                    long nextReceiver = receiver == null ? limit : receiver.start;
                    long nextOutput = output == null ? limit : output.start;
                    long nextUnknown = nextBoundary(unknownWriteRanges, cursor, false);
                    next = minUnsigned(limit, minUnsigned(nextPc,
                            minUnsigned(nextReceiver, minUnsigned(nextOutput, nextUnknown))));
                    if (next == cursor) {
                        gapTrackingComplete = false;
                        break;
                    }
                    if (pcTrackingComplete && receiverTrackingComplete && outputDropTrackingComplete) {
                        long added = accountedTransportMissing.addAndCountNewFrames(cursor, next - cursor);
                        if (added < 0) gapTrackingComplete = false;
                        else settledTransportMissingFrames += added;
                    } else {
                        gapTrackingComplete = false;
                    }
                }
                cursor = next;
            }
        }
    }

    private void accountPcDropIntervals(AnpdLossLedger.Interval[] pcDrops, long settledThrough) {
        for (AnpdLossLedger.Interval interval : pcDrops) {
            long start = maxUnsigned(interval.start, eligibleOrigin);
            long limit = minUnsigned(interval.start + interval.frameCount, settledThrough);
            long cursor = start;
            while (Long.compareUnsigned(cursor, limit) < 0) {
                boolean inGood = contains(settledGoodRanges, cursor);
                long next = minUnsigned(limit, nextBoundary(settledGoodRanges, cursor, inGood));
                if (next == cursor) {
                    gapTrackingComplete = false;
                    break;
                }
                if (inGood) {
                    gapTrackingComplete = false;
                } else {
                    long added = accountedPcDrops.addAndCountNewFrames(cursor, next - cursor);
                    if (added < 0) gapTrackingComplete = false;
                    else settledPcDropFrames += added;
                    reclassifyLowerPriority(cursor, next - cursor, 0);
                }
                cursor = next;
            }
        }
    }

    private void accountOutputDropIntervals(SourceFrameRanges.Range[] ranges,
            SourceFrameRanges.Range[] receiverDrops, AnpdLossLedger.Interval[] pcDrops,
            long settledThrough) {
        if (ranges == null) return;
        for (SourceFrameRanges.Range range : ranges)
            accountOutputDropInterval(range.start, range.frameCount, receiverDrops, pcDrops, settledThrough);
    }

    private void accountOutputDropInterval(long rangeStart, long frameCount,
            SourceFrameRanges.Range[] receiverDrops, AnpdLossLedger.Interval[] pcDrops,
            long settledThrough) {
        long start = maxUnsigned(rangeStart, eligibleOrigin);
        long limit = minUnsigned(rangeStart + frameCount, settledThrough);
        long cursor = start;
        while (Long.compareUnsigned(cursor, limit) < 0) {
            boolean inGood = contains(settledGoodRanges, cursor);
            AnpdLossLedger.Interval pc = containingOrNextPc(pcDrops, cursor);
            boolean inPc = pc != null && Long.compareUnsigned(pc.start, cursor) <= 0;
            SourceFrameRanges.Range receiver = containingOrNextReceiver(receiverDrops, cursor);
            boolean inReceiver = receiver != null
                    && Long.compareUnsigned(receiver.start, cursor) <= 0;
            long next = minUnsigned(limit, nextBoundary(settledGoodRanges, cursor, inGood));
            if (pc != null) next = minUnsigned(next,
                    inPc ? pc.start + pc.frameCount : pc.start);
            if (receiver != null) next = minUnsigned(next,
                    inReceiver ? receiver.start + receiver.frameCount : receiver.start);
            if (next == cursor) {
                gapTrackingComplete = false;
                break;
            }
            if (!inGood && !inPc && !inReceiver) {
                long added = outputDropRanges.addAndCountNewFrames(cursor, next - cursor);
                if (added < 0) markSegmentUnknown();
                else settledOutputDropFrames += added;
                reclassifyLowerPriority(cursor, next - cursor, 2);
            }
            cursor = next;
        }
    }

    private void reclassifyLowerPriority(long start, long frameCount, int priority) {
        if (priority == 0) subtractReclassified(accountedReceiverDrops, start, frameCount, 1);
        if (priority <= 1) subtractReclassified(outputDropRanges, start, frameCount, 2);
        subtractReclassified(accountedTransportMissing, start, frameCount, 3);
    }

    private void subtractReclassified(SourceFrameRanges ranges, long start, long frameCount, int category) {
        long removed = ranges.removeAndCountFrames(start, frameCount);
        if (removed < 0) {
            markSegmentUnknown();
            return;
        }
        switch (category) {
            case 1: settledReceiverDropFrames -= removed; break;
            case 2: settledOutputDropFrames -= removed; break;
            case 3: settledTransportMissingFrames -= removed; break;
            default: throw new IllegalArgumentException("unknown loss category");
        }
        if (settledReceiverDropFrames < 0 || settledOutputDropFrames < 0
                || settledTransportMissingFrames < 0) markSegmentUnknown();
    }
    private void accountPendingOutputSettlement(long settledThrough) {
        pendingOutputDropsSettled = true;
        for (int i = 0; i < pendingOutputDropRanges.size(); i++) {
            if (Long.compareUnsigned(pendingOutputDropRanges.end(i), settledThrough) > 0) {
                pendingOutputDropsSettled = false;
                return;
            }
        }
    }

    private void accountReceiverDropIntervals(SourceFrameRanges.Range[] ranges,
            AnpdLossLedger.Interval[] pcDrops, long settledThrough) {
        if (ranges == null) return;
        for (SourceFrameRanges.Range range : ranges) {
            long start = maxUnsigned(range.start, eligibleOrigin);
            long limit = minUnsigned(range.start + range.frameCount, settledThrough);
            long cursor = start;
            while (Long.compareUnsigned(cursor, limit) < 0) {
                boolean inGood = contains(settledGoodRanges, cursor);
                boolean inPendingOutput = containsPendingOutput(cursor);
                AnpdLossLedger.Interval pc = containingOrNextPc(pcDrops, cursor);
                boolean inPc = pc != null && Long.compareUnsigned(pc.start, cursor) <= 0;
                long next = limit;
                next = minUnsigned(next, nextBoundary(settledGoodRanges, cursor, inGood));
                next = minUnsigned(next, nextPendingOutputBoundary(cursor, inPendingOutput));
                if (pc != null) next = minUnsigned(next, inPc ? pc.start + pc.frameCount : pc.start);
                if (next == cursor) {
                    gapTrackingComplete = false;
                    break;
                }
                if (!inGood && !inPendingOutput && !inPc) {
                    long added = accountedReceiverDrops.addAndCountNewFrames(cursor, next - cursor);
                    if (added < 0) gapTrackingComplete = false;
                    else settledReceiverDropFrames += added;
                    reclassifyLowerPriority(cursor, next - cursor, 1);
                }
                cursor = next;
            }
        }
    }

    private static boolean contains(SourceFrameRanges ranges, long frame) {
        for (int i = 0; i < ranges.size(); i++)
            if (Long.compareUnsigned(ranges.start(i), frame) <= 0
                    && Long.compareUnsigned(ranges.end(i), frame) > 0) return true;
        return false;
    }

    private boolean containsPendingOutput(long sourceFrame) {
        for (int i = 0; i < size; i++) {
            int at = (head + i) % CAPACITY;
            long end = sourceStarts[at] + frames[at];
            if (Long.compareUnsigned(sourceStarts[at], sourceFrame) <= 0
                    && Long.compareUnsigned(end, sourceFrame) > 0) return true;
        }
        return false;
    }

    private long nextPendingOutputBoundary(long sourceFrame, boolean inside) {
        long boundary = -1L;
        for (int i = 0; i < size; i++) {
            int at = (head + i) % CAPACITY;
            long candidate = inside ? sourceStarts[at] + frames[at] : sourceStarts[at];
            if (Long.compareUnsigned(candidate, sourceFrame) > 0
                    && (boundary == -1L || Long.compareUnsigned(candidate, boundary) < 0))
                boundary = candidate;
        }
        return boundary;
    }

    private static long nextBoundary(SourceFrameRanges ranges, long frame, boolean inside) {
        long boundary = -1L;
        for (int i = 0; i < ranges.size(); i++) {
            long candidate = inside ? ranges.end(i) : ranges.start(i);
            if (Long.compareUnsigned(candidate, frame) > 0
                    && (boundary == -1L || Long.compareUnsigned(candidate, boundary) < 0))
                boundary = candidate;
        }
        return boundary;
    }
    private static AnpdLossLedger.Interval containingOrNextPc(AnpdLossLedger.Interval[] ranges, long cursor) {
        for (AnpdLossLedger.Interval range : ranges) {
            long end = range.start + range.frameCount;
            if (Long.compareUnsigned(end, cursor) > 0) return range;
        }
        return null;
    }

    private static SourceFrameRanges.Range containingOrNextReceiver(SourceFrameRanges.Range[] ranges, long cursor) {
        for (SourceFrameRanges.Range range : ranges) {
            long end = range.start + range.frameCount;
            if (Long.compareUnsigned(end, cursor) > 0) return range;
        }
        return null;
    }

    private static long minUnsigned(long left, long right) {
        return Long.compareUnsigned(left, right) <= 0 ? left : right;
    }

    private static long maxUnsigned(long left, long right) {
        return Long.compareUnsigned(left, right) >= 0 ? left : right;
    }

    private void observeEligibleSource(long start) {
        if (sourceSegmentFloorKnown && Long.compareUnsigned(start, sourceSegmentFloor) < 0) return;
        if (!eligibleOriginKnown || Long.compareUnsigned(start, eligibleOrigin) < 0) {
            eligibleOrigin = start;
            eligibleOriginKnown = true;
        }
    }

    private void markSegmentUnknown() {
        complete = false;
        rollingSegmentInvalidated = true;
    }

    synchronized void beginSourceSegment() {
        beginSourceSegment(-1L);
    }

    synchronized void beginSourceSegment(long sourceFloor, long settledThrough) {
        beginSourceSegment(sourceFloor, settledThrough, 48_000);
    }

    synchronized void beginSourceSegment(long sourceFloor, long settledThrough, int sampleRate) {
        boolean previousSettlement = sourceSettlementKnown || eligibleOriginKnown || settledSourceFrames > 0;
        long logicalBase = settledEligibleSourceFrames();
        long eligibleThrough = settledEligibleSourceThrough();
        long tail = sourceSegmentCaptureTail;
        long unknownFrames = tail != -1L && eligibleThrough != -1L
                && Long.compareUnsigned(tail, eligibleThrough) > 0 ? tail - eligibleThrough : 0;
        boolean unknownUnmapped = previousSettlement && (sourceFloor == -1L || settledThrough == -1L);
        if (unknownFrames == 0 && previousSettlement && sourceFloor != -1L && settledThrough != -1L
                && Long.compareUnsigned(sourceFloor, settledThrough) > 0)
            unknownFrames = sourceFloor - settledThrough;
        beginSourceSegment(sourceFloor);
        sourceSegmentTimelineBase = logicalBase;
        if (unknownFrames > 0) {
            complete = false;
            sourceSegmentUnknownStart = logicalBase;
            sourceSegmentUnknownEnd = logicalBase + unknownFrames;
            sourceSegmentLogicalStart = sourceSegmentUnknownEnd;
        } else if (unknownUnmapped) {
            sourceSegmentFloorUncertaintyIsProvisional = true;
            sourceSegmentCompleteBeforeProvisionalFloor = complete;
            complete = false;
            sourceSegmentUnknownStart = Math.max(0L, logicalBase - 60L * Math.max(1, sampleRate));
            sourceSegmentUnknownEnd = logicalBase;
        }
    }

    synchronized void recordSourceCaptureTail(long captureTail) {
        if (captureTail != -1L && (sourceSegmentCaptureTail == -1L
                || Long.compareUnsigned(captureTail, sourceSegmentCaptureTail) > 0))
            sourceSegmentCaptureTail = captureTail;
    }

    synchronized boolean setSourceSegmentFloor(long sourceFloor) {
        if (size != 0 || writtenOutputFrames != 0) return false;
        sourceSegmentFloorKnown = true;
        sourceSegmentFloor = sourceFloor;
        eligibleOriginKnown = true;
        eligibleOrigin = sourceFloor;
        settledSourceThrough = sourceFloor;
        hasSettledSourceThrough = true;
        if (sourceSegmentFloorUncertaintyIsProvisional) {
            sourceSegmentUnknownStart = -1L;
            sourceSegmentUnknownEnd = -1L;
            sourceSegmentLogicalStart = sourceSegmentTimelineBase;
            complete = sourceSegmentCompleteBeforeProvisionalFloor;
            sourceSegmentFloorUncertaintyIsProvisional = false;
        }
        return true;
    }
    synchronized void beginSourceSegment(long sourceFloor) {
        if (size != 0 || writtenOutputFrames != lastPlayedOutputFrames || !pendingOutputDropsSettled)
            markSegmentUnknown();
        sourceSegmentLogicalStart = settledEligibleSourceFrames();
        sourceSegmentTimelineBase = sourceSegmentLogicalStart;
        sourceSegmentCaptureTail = -1L;
        sourceSegmentUnknownStart = -1L;
        sourceSegmentUnknownEnd = -1L;
        sourceSegmentId++;
        head = 0;
        size = 0;
        writtenOutputFrames = 0;
        lastPlayedOutputFrames = 0;
        hasSettledSourceThrough = sourceFloor != -1L;
        settledSourceThrough = sourceFloor;
        sourceSegmentSettlementKnown = false;
        sourceSegmentSettledThrough = -1L;
        sourceSegmentActive = true;
        sourceSegmentFloorKnown = sourceFloor != -1L;
        sourceSegmentFloor = sourceFloor;
        sourceSegmentFloorUncertaintyIsProvisional = false;
        eligibleOriginKnown = sourceSegmentFloorKnown;
        eligibleOrigin = sourceSegmentFloorKnown ? sourceFloor : 0;
        gapCount = 0;
        gapTrackingComplete = true;
        accountedPcDrops.clear();
        accountedReceiverDrops.clear();
        accountedTransportMissing.clear();
        unknownWriteRanges.clear();
        settledUnknownRanges.clear();
        settledGoodRanges.clear();
        outputDropRanges.clear();
        pendingOutputDropRanges.clear();
        pendingOutputDropsSettled = true;
        rollingSourceTrackingComplete = false;
        rollingSegmentInvalidated = false;
    }

    synchronized void resetLossPeriod() {
        head = 0;
        size = 0;
        gapCount = 0;
        gapTrackingComplete = true;
        hasSettledSourceThrough = false;
        settledSourceThrough = -1;
        settledSourceFrames = 0;
        settledGoodSourceFrames = 0;
        settledSyntheticFrames = 0;
        settledPcDropFrames = 0;
        settledReceiverDropFrames = 0;
        settledTransportMissingFrames = 0;
        settledOutputDropFrames = 0;
        recoveryCount = 0;
        recoveryDurationMs = 0;
        writtenOutputFrames = 0;
        lastPlayedOutputFrames = 0;
        complete = true;
        rollingSegmentInvalidated = false;
        rollingSourceTrackingComplete = false;
        sourceSettlementKnown = false;
        sourceSegmentSettlementKnown = false;
        sourceSegmentSettledThrough = -1L;
        sourceSegmentActive = false;
        sourceSegmentFloorKnown = false;
        sourceSegmentFloor = 0;
        sourceSegmentFloorUncertaintyIsProvisional = false;
        sourceSegmentTimelineBase = 0;
        sourceSegmentUnknownStart = -1L;
        sourceSegmentUnknownEnd = -1L;
        eligibleOriginKnown = false;
        eligibleOrigin = 0;
        pendingOutputDropsSettled = true;
        accountedPcDrops.clear();
        accountedReceiverDrops.clear();
        accountedTransportMissing.clear();
        unknownWriteRanges.clear();
        settledUnknownRanges.clear();
        settledGoodRanges.clear();
        outputDropRanges.clear();
        pendingOutputDropRanges.clear();
    }
    synchronized void resetOutputEpoch(boolean playbackPositionKnown) {
        if (size != 0 || writtenOutputFrames != lastPlayedOutputFrames) {
            if (playbackPositionKnown) {
                long mappedPendingFrames = 0;
                for (int i = 0; i < size; i++) {
                    int at = (head + i) % CAPACITY;
                    mappedPendingFrames += frames[at];
                    if (!synthetic[at] && !unknownMapping[at]) {
                        if (!pendingOutputDropRanges.add(sourceStarts[at], frames[at])) markSegmentUnknown();
                    }

                }
                if (mappedPendingFrames != writtenOutputFrames - lastPlayedOutputFrames)
                    markSegmentUnknown();
                if (mappedPendingFrames > 0) pendingOutputDropsSettled = false;
            } else {
                markSegmentUnknown();
            }
        }
        if (!outputDropRanges.complete() || !pendingOutputDropRanges.complete()) markSegmentUnknown();
        head = 0;
        size = 0;
        writtenOutputFrames = 0;
        lastPlayedOutputFrames = 0;
    }

    synchronized void recordUnknownWrite(int count) {
        if (count <= 0) return;
        markSegmentUnknown();
        appendUnknownOutput(count);
    }
    synchronized void recordUnknownOutput(int outputFrames) {
        if (outputFrames <= 0) return;
        markSegmentUnknown();
        appendUnknownOutput(outputFrames);
    }
    synchronized void recordUnknownWrite(long sourceStart, int sourceFrames, int outputFrames) {
        if (outputFrames <= 0) return;
        if (sourceStart < 0 || sourceFrames <= 0 || !unknownWriteRanges.add(sourceStart, sourceFrames))
            markSegmentUnknown();
        else {
            complete = false;
            recordGap(sourceStart, sourceStart + sourceFrames);
        }
        appendUnknownOutput(outputFrames);
    }
    synchronized void recordUnknownWrite(UdpPcmQueue.QueueInputStream input, int outputFrames) {
        if (outputFrames <= 0) return;
        if (input == null || !input.sourceRangeBatchComplete()) {
            markSegmentUnknown();
            appendUnknownOutput(outputFrames);
            return;
        }
        boolean known = false;
        for (int i = 0; i < input.sourceRangeCount(); i++) {
            if (input.sourceRangeSynthetic(i)) continue;
            if (!unknownWriteRanges.add(input.sourceRangeStart(i), input.sourceRangeFrames(i))) {
                markSegmentUnknown();
                appendUnknownOutput(outputFrames);
                return;
            }
            known = true;
        }
        if (!known) markSegmentUnknown();
        else complete = false;
        appendUnknownOutput(outputFrames);
    }
    private void appendUnknownOutput(int outputFrames) {
        if (size == CAPACITY) {
            markSegmentUnknown();
            writtenOutputFrames += outputFrames;
            return;
        }
        int at = (head + size++) % CAPACITY;
        outputStarts[at] = writtenOutputFrames;
        sourceStarts[at] = 0;
        frames[at] = outputFrames;
        synthetic[at] = false;
        unknownMapping[at] = true;
        writtenOutputFrames += outputFrames;
    }
    synchronized void markIncomplete() { markSegmentUnknown(); }
    synchronized boolean rollingComplete() {
        return rollingSourceTrackingComplete && !rollingSegmentInvalidated && sourceSettlementKnown
                && sourceSegmentSettlementKnown
                && gapTrackingComplete && outputDropRanges.complete()
                && pendingOutputDropRanges.complete() && pendingOutputDropsSettled;
    }
    synchronized boolean hasCurrentSegmentSettlement() { return sourceSegmentSettlementKnown; }
    synchronized boolean complete() {
        return complete && eligibleOriginKnown && gapTrackingComplete && outputDropRanges.complete()
                && pendingOutputDropRanges.complete() && pendingOutputDropsSettled
                && sourceSettlementKnown && sourceSegmentSettlementKnown;
    }
    synchronized long sourceSegmentId() { return sourceSegmentId; }
    synchronized long sourceSegmentLogicalStart() { return sourceSegmentLogicalStart; }
    synchronized long sourceSegmentTimelineBase() { return sourceSegmentTimelineBase; }
    synchronized long sourceSegmentUnknownStart() { return sourceSegmentUnknownStart; }
    synchronized long sourceSegmentUnknownEnd() { return sourceSegmentUnknownEnd; }
    synchronized long sourceSegmentOrigin() { return eligibleOriginKnown ? eligibleOrigin : -1L; }
    synchronized long sourceSegmentSettledThrough() { return sourceSegmentSettledThrough; }    synchronized long settledEligibleSourceThrough() {
        if (!eligibleOriginKnown || sourceSegmentSettledThrough == -1L
                || Long.compareUnsigned(eligibleOrigin, sourceSegmentSettledThrough) > 0) return -1L;
        SourceFrameRanges combined = new SourceFrameRanges();
        addRanges(combined, settledGoodRanges.snapshot());
        addRanges(combined, accountedPcDrops.snapshot());
        addRanges(combined, accountedReceiverDrops.snapshot());
        addRanges(combined, accountedTransportMissing.snapshot());
        addRanges(combined, outputDropRanges.snapshot());
        addRanges(combined, settledUnknownRanges.snapshot());
        if (!combined.complete()) return -1L;
        long cursor = eligibleOrigin;
        SourceFrameRanges.Range[] ranges = combined.snapshot();
        for (SourceFrameRanges.Range range : ranges) {
            long end = range.start + range.frameCount;
            if (Long.compareUnsigned(end, cursor) <= 0) continue;
            if (Long.compareUnsigned(range.start, cursor) > 0) break;
            cursor = minUnsigned(end, sourceSegmentSettledThrough);
            if (Long.compareUnsigned(cursor, sourceSegmentSettledThrough) >= 0) break;
        }
        return cursor;
    }
    synchronized SourceFrameRanges.Range[] settledLossRanges() {
        SourceFrameRanges combined = new SourceFrameRanges();
        addRanges(combined, accountedPcDrops.snapshot());
        addRanges(combined, accountedReceiverDrops.snapshot());
        addRanges(combined, accountedTransportMissing.snapshot());
        addRanges(combined, outputDropRanges.snapshot());
        return combined.complete() ? combined.snapshot() : null;
    }
    synchronized SourceFrameRanges.Range[] settledUnknownRanges() {
        return settledUnknownRanges.complete() ? settledUnknownRanges.snapshot() : null;
    }
    private static void addRanges(SourceFrameRanges target, SourceFrameRanges.Range[] ranges) {
        for (SourceFrameRanges.Range range : ranges)
            if (!target.add(range.start, range.frameCount)) return;
    }    synchronized long settledSourceThrough() { return settledSourceThrough; }
    synchronized long settledSourceFrames() { return settledSourceFrames; }
    synchronized long settledGoodSourceFrames() { return settledGoodSourceFrames; }
    synchronized long settledSyntheticFrames() { return settledSyntheticFrames; }
    synchronized long settledPcDropFrames() { return settledPcDropFrames; }
    synchronized long settledReceiverDropFrames() { return settledReceiverDropFrames; }
    synchronized long settledTransportMissingFrames() { return settledTransportMissingFrames; }
    synchronized long settledOutputDropFrames() { return settledOutputDropFrames; }
    synchronized void recordRecovery(long durationNanos) {
        recoveryCount++;
        recoveryDurationMs = Math.addExact(recoveryDurationMs, Math.max(0L, durationNanos) / 1_000_000L);
    }
    synchronized long recoveryCount() { return recoveryCount; }
    synchronized long recoveryDurationMs() { return recoveryDurationMs; }
    synchronized long settledLossFrames() {
        return settledPcDropFrames + settledReceiverDropFrames
                + settledTransportMissingFrames + settledOutputDropFrames;
    }
    synchronized long settledEligibleSourceFrames() {
        return settledGoodSourceFrames + settledPcDropFrames + settledReceiverDropFrames
                + settledTransportMissingFrames + settledOutputDropFrames
                + settledUnknownRanges.coveredFrames();
    }
    synchronized Float lossPercent() {
        long eligible = settledEligibleSourceFrames();
        if (!complete() || eligible <= 0) return null;
        long loss = settledPcDropFrames + settledReceiverDropFrames
                + settledTransportMissingFrames + settledOutputDropFrames;
        return (float) (100.0 * loss / eligible);
    }
    synchronized int sourceGapCount() { return gapCount; }
    synchronized long sourceGapStart(int index) { return gapStarts[index]; }
    synchronized long sourceGapEnd(int index) { return gapEnds[index]; }
    synchronized boolean sourceGapTrackingComplete() { return gapTrackingComplete; }
}
