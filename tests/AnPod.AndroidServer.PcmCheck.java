package com.anpod.server;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.io.SequenceInputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** One-file regression check: javac Options.java Pcm16FrameStream.java tests/AnPod.AndroidServer.PcmCheck.java */
final class PcmCheck {
    public static void main(String[] args) throws Exception {
        byte[] expected = new byte[40];
        for (int i = 0; i < expected.length; i++) expected[i] = (byte)i;

        Pcm16FrameStream stream = new Pcm16FrameStream();
        FragmentedInput input = new FragmentedInput(expected, 1, 2, 1, 3, 5, 1, 7, 4, 2, 6, 8);
        byte[] buffer = new byte[16];
        List<Byte> readBack = new ArrayList<>();
        int count;
        int preRoll = 0;
        while (preRoll < 8) {
            count = stream.readFrames(input, buffer, 8 - preRoll);
            if (count < 0) throw new AssertionError("fragmented stream ended during pre-roll");
            for (int i = 0; i < count; i++) readBack.add(buffer[i]);
            preRoll += count;
        }
        while ((count = stream.readFrames(input, buffer, buffer.length)) >= 0) {
            for (int i = 0; i < count; i++) readBack.add(buffer[i]);
        }
        byte[] actual = new byte[readBack.size()];
        for (int i = 0; i < actual.length; i++) actual[i] = readBack.get(i);
        if (!Arrays.equals(expected, actual)) throw new AssertionError("fragmented PCM read lost or reordered bytes");

        byte[] preRollExpected = new byte[40];
        for (int i = 0; i < preRollExpected.length; i++) preRollExpected[i] = (byte)(i + 40);
        Pcm16FrameStream preRollStream = new Pcm16FrameStream();
        FragmentedInput preRollInput = new FragmentedInput(preRollExpected, 3, 5, 2, 7, 1, 9, 13);
        byte[] smallBuffer = new byte[16];
        byte[] preRollActual = new byte[preRollExpected.length];
        int preRolled = 0;
        while (preRolled < preRollExpected.length) {
            int read = preRollStream.readFrames(preRollInput, smallBuffer,
                    Math.min(smallBuffer.length, preRollExpected.length - preRolled));
            if (read < 0) throw new AssertionError("PCM stream ended during multi-buffer pre-roll");
            System.arraycopy(smallBuffer, 0, preRollActual, preRolled, read);
            preRolled += read;
        }
        if (!Arrays.equals(preRollExpected, preRollActual))
            throw new AssertionError("pre-roll target larger than read buffer lost PCM bytes");

        List<Byte> written = new ArrayList<>();
        final boolean[] returnedZero = {false};
        Pcm16FrameStream.writeFrames((data, offset, length) -> {
            if (!returnedZero[0]) { returnedZero[0] = true; return 0; }
            int amount = Math.min(8, length);
            for (int i = 0; i < amount; i++) written.add(data[offset + i]);
            return amount;
        }, expected, expected.length);
        if (!Arrays.equals(expected, toArray(written))) throw new AssertionError("partial/zero writes lost PCM bytes");

        try {
            Pcm16FrameStream.writeFrames((data, offset, length) -> -6, expected, 4);
            throw new AssertionError("AudioTrack write failure was ignored");
        } catch (IOException expectedFailure) {
            if (!expectedFailure.getMessage().contains("-6")) throw expectedFailure;
        }

        Options balanced = Options.parse("socketName=s", "connectCode=c");
        Options lowLatency = Options.parse("socketName=s", "connectCode=c", "playbackMode=low-latency");
        if (!"balanced".equals(balanced.playbackMode) || !"low-latency".equals(lowLatency.playbackMode))
            throw new AssertionError("playback mode arguments did not default/parse");

        if (Pcm16FrameStream.requestedAudioTrackBufferFrames(48000, 10000, true) != 720
                || Pcm16FrameStream.requestedAudioTrackBufferFrames(48000, 10000, false) != 2400
                || Pcm16FrameStream.requestedAudioTrackBufferFrames(48000, 1200, false) != 1200
                || Pcm16FrameStream.requestedAudioTrackBufferFrames(48000, 0, false) != 0) {
            throw new AssertionError("AudioTrack buffer request was not mode-bounded by actual capacity");
        }
        Pcm16FrameStream.PlaybackHeadTracker playbackHead = new Pcm16FrameStream.PlaybackHeadTracker();
        playbackHead.reset(0xffff_fffe);
        if (playbackHead.update(0xffff_ffff) != 1 || playbackHead.update(0) != 2
                || Pcm16FrameStream.pendingFrames(600, playbackHead.update(100)) != 498
                || Pcm16FrameStream.pendingFrames(100, 150) != 0) {
            throw new AssertionError("AudioTrack pending frames lost pre-roll or unsigned head wrap");
        }
        Pcm16FrameStream.PlaybackHeadTracker reorderedPlaybackHead = new Pcm16FrameStream.PlaybackHeadTracker();
        reorderedPlaybackHead.reset(100);
        if (reorderedPlaybackHead.update(110) != 10 || reorderedPlaybackHead.update(105) != 10
                || reorderedPlaybackHead.update(120) != 20) {
            throw new AssertionError("out-of-order AudioTrack head sample was counted more than once");
        }
        Pcm16FrameStream.PlaybackHeadTracker cadencePlaybackHead = new Pcm16FrameStream.PlaybackHeadTracker();
        cadencePlaybackHead.reset(0, 0);
        if (!cadencePlaybackHead.isAdvancing(100_000_000L)
                || cadencePlaybackHead.isAdvancing(100_000_001L)
                || cadencePlaybackHead.update(960, 120_000_000L) != 960
                || !cadencePlaybackHead.isAdvancing(120_000_000L)) {
            throw new AssertionError("playback head cadence did not tolerate intervals between advances");
        }
        Pcm16FrameStream monoStream = new Pcm16FrameStream(1);
        byte[] monoBuffer = new byte[6];
        if (monoStream.readFrames(new ByteArrayInputStream(new byte[] {1, 2, 3, 4, 5, 6}), monoBuffer,
                monoBuffer.length) != monoBuffer.length) {
            throw new AssertionError("mono AudioTrack pre-roll did not preserve mono frame boundaries");
        }

        try {
            Pcm16FrameStream truncated = new Pcm16FrameStream();
            ByteArrayInputStream truncatedInput = new ByteArrayInputStream(new byte[] {1, 2, 3});
            while (truncated.readFrames(truncatedInput, buffer, buffer.length) >= 0) { }
            throw new AssertionError("incomplete final frame was silently discarded");
        } catch (IOException expectedFailure) {
            if (!expectedFailure.getMessage().contains("incomplete stereo frame")) throw expectedFailure;
        }

        int lowLatencyTarget = Pcm16FrameStream.backlogTargetFrames(48000, true);
        int lowLatencyTrigger = Pcm16FrameStream.backlogTriggerFrames(48000, true);
        int balancedTarget = Pcm16FrameStream.backlogTargetFrames(48000, false);
        int balancedTrigger = Pcm16FrameStream.backlogTriggerFrames(48000, false);
        if (lowLatencyTarget != 240 || lowLatencyTrigger != 720
                || balancedTarget != 480 || balancedTrigger != 1_440) {
            throw new AssertionError("wired backlog target/trigger did not match mode-specific bounds");
        }
        byte[] backlog = new byte[20 * Pcm16FrameStream.FRAME_BYTES];
        for (int frame = 0; frame < 20; frame++) {
            int sample = 10000 + frame * 100;
            for (int channel = 0; channel < 2; channel++) {
                int offset = frame * Pcm16FrameStream.FRAME_BYTES + channel * 2;
                backlog[offset] = (byte) sample;
                backlog[offset + 1] = (byte) (sample >> 8);
            }
        }
        byte[] backlogBuffer = new byte[16];
        byte[] belowTrigger = Arrays.copyOf(backlog, 7 * Pcm16FrameStream.FRAME_BYTES);
        Pcm16FrameStream belowTriggerStream = new Pcm16FrameStream();
        belowTriggerStream.setBacklogLimit(2, 8, 2);
        ByteArrayInputStream belowTriggerInput = new ByteArrayInputStream(belowTrigger);
        byte[] belowTriggerOutput = new byte[belowTrigger.length];
        int belowTriggerBytes = 0;
        int belowTriggerRead;
        while ((belowTriggerRead = belowTriggerStream.readFrames(belowTriggerInput, backlogBuffer,
                backlogBuffer.length)) >= 0) {
            System.arraycopy(backlogBuffer, 0, belowTriggerOutput, belowTriggerBytes, belowTriggerRead);
            belowTriggerBytes += belowTriggerRead;
        }
        if (belowTriggerBytes != belowTrigger.length || !Arrays.equals(belowTrigger, belowTriggerOutput)
                || belowTriggerStream.getDroppedBacklogFrames() != 0) {
            throw new AssertionError("backlog below trigger discarded PCM");
        }

        byte[] firstBurst = Arrays.copyOf(backlog, 12 * Pcm16FrameStream.FRAME_BYTES);
        byte[] nextBurst = new byte[3 * Pcm16FrameStream.FRAME_BYTES];
        for (int frame = 0; frame < 3; frame++) {
            int sample = 12_000 + frame * 100;
            for (int channel = 0; channel < 2; channel++) {
                int offset = frame * Pcm16FrameStream.FRAME_BYTES + channel * 2;
                nextBurst[offset] = (byte) sample;
                nextBurst[offset + 1] = (byte) (sample >> 8);
            }
        }
        Pcm16FrameStream burstStream = new Pcm16FrameStream();
        burstStream.setBacklogLimit(2, 8, 2);
        SequenceInputStream consecutiveBursts = new SequenceInputStream(
                new ByteArrayInputStream(firstBurst), new ByteArrayInputStream(nextBurst));
        int afterBurstBytes = burstStream.readFrames(consecutiveBursts, backlogBuffer, backlogBuffer.length);
        int afterBurstFirstSample = readSample(backlogBuffer, 0);
        int afterBurstLastSample = readSample(backlogBuffer, 4);
        int followingBurstBytes = burstStream.readFrames(consecutiveBursts, backlogBuffer, backlogBuffer.length);
        if (afterBurstBytes != 2 * Pcm16FrameStream.FRAME_BYTES
                || afterBurstFirstSample != 0 || afterBurstLastSample != 5_550
                || followingBurstBytes != nextBurst.length
                || readSample(backlogBuffer, 0) != 12_000 || readSample(backlogBuffer, 8) != 12_200
                || burstStream.getDroppedBacklogFrames() != 10) {
            throw new AssertionError("consecutive burst was trimmed more than once or lost fresh PCM");
        }

        Pcm16FrameStream monoBacklogStream = new Pcm16FrameStream(1);
        monoBacklogStream.setBacklogLimit(2, 4, 1);
        byte[] monoBacklog = new byte[7 * 2];
        for (int frame = 0; frame < 7; frame++) {
            int sample = 1_000 + frame * 100;
            monoBacklog[frame * 2] = (byte) sample;
            monoBacklog[frame * 2 + 1] = (byte) (sample >> 8);
        }
        byte[] monoBacklogOutput = new byte[8];
        int monoBacklogBytes = monoBacklogStream.readFrames(new ByteArrayInputStream(monoBacklog),
                monoBacklogOutput, monoBacklogOutput.length);
        if (monoBacklogBytes != 4 || monoBacklogStream.getDroppedBacklogFrames() != 5
                || readSample(monoBacklogOutput, 0) != 0 || readSample(monoBacklogOutput, 2) != 1_600) {
            throw new AssertionError("mono backlog trim lost sample alignment or retained old PCM");
        }

        // Model PCM accumulated during focus pause; the first read after resume must skip stale history.
        Pcm16FrameStream backlogStream = new Pcm16FrameStream();
        backlogStream.setBacklogLimit(2, 8, 2);
        BacklogInput backlogInput = new BacklogInput(backlog);
        if (backlogStream.readFrames(backlogInput, backlogBuffer, backlogBuffer.length) != 0) {
            throw new AssertionError("partial frame should stay buffered until completed");
        }
        int backlogBytes = backlogStream.readFrames(backlogInput, backlogBuffer, backlogBuffer.length);
        if (backlogBytes != 2 * Pcm16FrameStream.FRAME_BYTES
                || backlogStream.getDroppedBacklogFrames() != 18 || backlogInput.unreadBytes() != 0
                || backlogInput.availableCalls != 2) {
            throw new AssertionError("backlog trim did not discard the oldest complete frames");
        }
        if (readSample(backlogBuffer, 0) != 0 || readSample(backlogBuffer, 2) != 0
                || readSample(backlogBuffer, 4) != 5950 || readSample(backlogBuffer, 6) != 5950) {
            throw new AssertionError("backlog trim lost frame alignment or did not fade in");
        }

        Pcm16FrameStream interruptedBacklogStream = new Pcm16FrameStream();
        interruptedBacklogStream.setBacklogLimit(2, 8, 2);
        BacklogInput interruptedBacklogInput = new BacklogInput(backlog);
        if (interruptedBacklogStream.readFrames(interruptedBacklogInput, backlogBuffer,
                backlogBuffer.length) != 0) {
            throw new AssertionError("partial frame should stay buffered before stop interruption");
        }
        Thread.currentThread().interrupt();
        try {
            interruptedBacklogStream.readFrames(interruptedBacklogInput, backlogBuffer, backlogBuffer.length);
            throw new AssertionError("interrupted backlog trim did not stop");
        } catch (InterruptedIOException interrupted) {
            if (interruptedBacklogStream.getDroppedBacklogFrames() != 0)
                throw new AssertionError("interrupted trim reported frames that it did not discard");
        } finally {
            Thread.interrupted();
        }

        checkDriftCorrection();
        System.out.println("Android PCM framing and mode checks passed.");
    }

    private static void checkDriftCorrection() throws Exception {
        DriftResult zero = simulateDrift(0, 48_000, 2L * 60 * 60 * 1000 / 10);
        if (zero.stream.getInsertedDriftFrames() != 0 || zero.stream.getDeletedDriftFrames() != 0
                || zero.underflows != 0) {
            throw new AssertionError("zero-ppm input triggered drift correction or an underflow");
        }
        checkDriftBound(simulateDrift(100, 48_000, 2L * 60 * 60 * 1000 / 10), 100);
        checkDriftBound(simulateDrift(-100, 48_000, 2L * 60 * 60 * 1000 / 10), -100);
        checkDriftBound(simulateDrift(500, 48_000, 2L * 60 * 60 * 1000 / 10), 500);
        checkDriftBound(simulateDrift(-500, 48_000, 2L * 60 * 60 * 1000 / 10), -500);
        checkDriftReadAdjustmentPairing(true);
        checkDriftReadAdjustmentPairing(false);
        long cadenceTicks = 2L * 60 * 60 * 1000 / 5;
        checkHeadCadenceDrift(simulateDriftWithHeadCadence(0, 48_000, cadenceTicks), 0);
        checkHeadCadenceDrift(simulateDriftWithHeadCadence(100, 48_000, cadenceTicks), 100);
        checkHeadCadenceDrift(simulateDriftWithHeadCadence(-100, 48_000, cadenceTicks), -100);
        checkHeadCadenceDrift(simulateDriftWithHeadCadence(500, 48_000, cadenceTicks), 500);
        checkHeadCadenceDrift(simulateDriftWithHeadCadence(-500, 48_000, cadenceTicks), -500);
        checkDriftBurstAndTargetShift();
        checkDriftPartialReadsAndReset();
        checkFrameEdits(1, true);
        checkFrameEdits(1, false);
        checkFrameEdits(2, true);
        checkFrameEdits(2, false);
    }

    private static DriftResult simulateDrift(int ppm, int sampleRate, long ticks) {
        final int blockFrames = sampleRate / 100;
        final int sourceTargetFrames = Pcm16FrameStream.backlogTargetFrames(sampleRate, true);
        final int frameBytes = 4;
        int reserveFrames = Pcm16FrameStream.driftReserveFrames(sampleRate);
        byte[] buffer = new byte[(blockFrames + reserveFrames) * frameBytes];
        long queueFrames = sourceTargetFrames;
        long pendingFrames = sampleRate / 200;
        long fractionalArrivals = 0;
        long totalArrivals = 0;
        long elapsedFrameNanos = 0;
        long processedBlocks = 0;
        int previousOutputFrames = blockFrames;
        long maxDeviation = 0;
        long minDeviation = Long.MAX_VALUE;
        int underflows = 0;
        Pcm16FrameStream stream = new Pcm16FrameStream(2);
        stream.beginDriftCorrection(sampleRate, sourceTargetFrames, 0);

        long durationFrameNanos = ticks * 10_000_000L * sampleRate;
        while (elapsedFrameNanos < durationFrameNanos) {
            fractionalArrivals += (long) previousOutputFrames * (1_000_000L + ppm);
            int arrived = (int) (fractionalArrivals / 1_000_000L);
            fractionalArrivals %= 1_000_000L;
            totalArrivals += arrived;
            queueFrames += arrived;
            if (queueFrames < blockFrames) {
                underflows++;
                break;
            }
            queueFrames -= blockFrames;

            long nowNanos = elapsedFrameNanos / sampleRate;
            long waterline = queueFrames + blockFrames + pendingFrames;
            int outputBytes = stream.correctDrift(buffer, blockFrames * frameBytes,
                    queueFrames, pendingFrames, sourceTargetFrames, nowNanos, true, false);
            previousOutputFrames = outputBytes / frameBytes;
            processedBlocks++;
            elapsedFrameNanos += (long) previousOutputFrames * 1_000_000_000L;

            long baseline = stream.getDriftBaselineFrames();
            if (baseline >= 0) {
                long deviation = waterline - baseline;
                maxDeviation = Math.max(maxDeviation, deviation);
                minDeviation = Math.min(minDeviation, deviation);
            }
        }
        return new DriftResult(stream, processedBlocks * blockFrames, totalArrivals,
                ticks,
                queueFrames, pendingFrames, maxDeviation, minDeviation, underflows);
    }

    private static void checkDriftBound(DriftResult result, int ppm) {
        int sampleRate = 48_000;
        long baseline = result.stream.getDriftBufferedBaselineFrames();
        long deadband = Math.max((sampleRate + 999L) / 1_000L,
                Math.min((sampleRate * 10L + 999L) / 1_000L, Math.max(1, baseline / 4)));
        long maxAllowedDeviation = deadband + sampleRate * 3L / 1_000L + 1;
        long signedCorrections = result.stream.getDeletedDriftFrames()
                - result.stream.getInsertedDriftFrames();
        long expectedDriftFrames = (long) sampleRate * ppm * result.durationTicks
                / (100L * 1_000_000L);
        long endingLevelDelta = result.queueFrames + result.pendingFrames
                - (Pcm16FrameStream.backlogTargetFrames(sampleRate, true) + sampleRate / 200L);
        if (result.underflows != 0 || baseline < 0
                || result.maxDeviation > maxAllowedDeviation
                || result.minDeviation < -maxAllowedDeviation
                || Math.abs(signedCorrections - expectedDriftFrames) > maxAllowedDeviation
                || (ppm > 0 && (result.stream.getDeletedDriftFrames() == 0
                        || result.stream.getInsertedDriftFrames() != 0))
                || (ppm < 0 && (result.stream.getInsertedDriftFrames() == 0
                        || result.stream.getDeletedDriftFrames() != 0))) {
            throw new AssertionError("drift correction did not stay bounded for " + ppm + "ppm"
                    + " baseline=" + baseline + " deviation=" + result.minDeviation + ".."
                    + result.maxDeviation + " corrected=" + signedCorrections
                    + " expectedDrift=" + expectedDriftFrames + " levelDelta=" + endingLevelDelta
                    + " underflows=" + result.underflows);
        }
    }

    private static void checkHeadCadenceDrift(DriftResult result, int ppm) {
        long signedCorrections = result.stream.getDeletedDriftFrames()
                - result.stream.getInsertedDriftFrames();
        long expectedDriftFrames = 48_000L * ppm * result.durationTicks
                / (100L * 1_000_000L);
        long toleranceFrames = 48_000 / 100;
        if (result.underflows != 0 || result.stream.getDriftBaselineFrames() < 0
                || Math.max(Math.abs(result.maxDeviation), Math.abs(result.minDeviation)) > toleranceFrames
                || Math.abs(signedCorrections - expectedDriftFrames) > toleranceFrames
                || (ppm > 0 && signedCorrections <= 0)
                || (ppm < 0 && signedCorrections >= 0)
                || (ppm == 0 && result.stream.getInsertedDriftFrames() + result.stream.getDeletedDriftFrames() > 1)
                || result.maxPreTrimQueueFrames > Pcm16FrameStream.backlogTriggerFrames(48_000, true) + 240
                || result.maxQueueFrames > Pcm16FrameStream.backlogTriggerFrames(48_000, true)
                || result.trimmedFrames != 0
                || (ppm > 0 && (result.stream.getDeletedDriftFrames() == 0
                        || result.stream.getInsertedDriftFrames() != 0))
                || (ppm < 0 && (result.stream.getInsertedDriftFrames() == 0
                        || result.stream.getDeletedDriftFrames() != 0))) {
            throw new AssertionError("5ms/20ms playback-head cadence failed for " + ppm + "ppm"
                    + " corrected=" + signedCorrections
                    + " expected=" + expectedDriftFrames
                    + " deviation=" + result.minDeviation + ".." + result.maxDeviation
                    + " baseline=" + result.stream.getDriftBaselineFrames()
                    + " queue=" + result.maxPreTrimQueueFrames + "/" + result.maxQueueFrames
                    + " trimmed=" + result.trimmedFrames
                    + " underflows=" + result.underflows);
        }
    }

    private static DriftResult simulateDriftWithHeadCadence(int ppm, int sampleRate, long ticks) {
        final int blockFrames = sampleRate / 200;
        final int sourceTargetFrames = Pcm16FrameStream.backlogTargetFrames(sampleRate, true);
        final int preRollFrames = sampleRate / 200;
        final int backlogTriggerFrames = Pcm16FrameStream.backlogTriggerFrames(sampleRate, true);
        final int frameBytes = 4;
        int reserveFrames = Pcm16FrameStream.driftReserveFrames(sampleRate);
        byte[] buffer = new byte[(blockFrames + reserveFrames) * frameBytes];
        long queueFrames = sourceTargetFrames;
        long totalFramesWritten = preRollFrames;
        long elapsedFrameNanos = 0;
        long processedBlocks = 0;
        long fractionalArrivals = 0;
        long totalArrivals = 0;
        long maxDeviation = 0;
        long minDeviation = Long.MAX_VALUE;
        long maxPreTrimQueueFrames = 0;
        long maxQueueFrames = 0;
        long trimmedFrames = 0;
        int underflows = 0;
        int playbackPosition = 0;
        int reportedPlaybackPosition = 0;
        long lastObservedPlayedFrames = 0;
        long nextHeadReportNanos = 20_000_000L;
        Pcm16FrameStream stream = new Pcm16FrameStream(2);
        Pcm16FrameStream.PlaybackHeadTracker playbackHead = new Pcm16FrameStream.PlaybackHeadTracker();
        playbackHead.reset(playbackPosition, 0);
        stream.beginDriftCorrection(sampleRate, sourceTargetFrames, 0);

        long durationFrameNanos = ticks * 5_000_000L;
        while (elapsedFrameNanos < durationFrameNanos) {
            fractionalArrivals += (long) blockFrames * (1_000_000L + ppm);
            int arrived = (int) (fractionalArrivals / 1_000_000L);
            fractionalArrivals %= 1_000_000L;
            totalArrivals += arrived;
            queueFrames += arrived;
            maxPreTrimQueueFrames = Math.max(maxPreTrimQueueFrames, queueFrames);
            if (queueFrames > backlogTriggerFrames) {
                trimmedFrames += queueFrames - sourceTargetFrames;
                queueFrames = sourceTargetFrames;
            }
            int sourceReadAdjustmentFrames = stream.driftReadFrameAdjustment(elapsedFrameNanos + 5_000_000L);
            int sourceReadFrames = blockFrames + sourceReadAdjustmentFrames;
            if (queueFrames < sourceReadFrames) {
                underflows++;
                break;
            }
            queueFrames -= sourceReadFrames;
            maxQueueFrames = Math.max(maxQueueFrames, queueFrames);

            long readStartNanos = elapsedFrameNanos;
            long nowNanos = readStartNanos + 5_000_000L;
            long actualPlayedFrames = readStartNanos / 5_000_000L * blockFrames;
            if (readStartNanos >= nextHeadReportNanos) {
                reportedPlaybackPosition = (int) actualPlayedFrames;
                nextHeadReportNanos = (readStartNanos / 20_000_000L + 1) * 20_000_000L;
            }
            playbackPosition = reportedPlaybackPosition;
            long reportedPlayedFrames = playbackHead.update(playbackPosition, readStartNanos);
            playbackHead.update(playbackPosition, nowNanos);
            boolean headAdvanced = reportedPlayedFrames > lastObservedPlayedFrames;
            lastObservedPlayedFrames = reportedPlayedFrames;
            boolean playbackAdvancing = playbackHead.isAdvancing(nowNanos);
            long pendingActualFrames = Pcm16FrameStream.pendingFrames(totalFramesWritten, actualPlayedFrames);
            long pendingReportedFrames = Pcm16FrameStream.pendingFrames(totalFramesWritten,
                    playbackHead.update(playbackPosition, nowNanos));
            long actualWaterline = queueFrames + sourceReadFrames + pendingActualFrames;
            long baselineWaterline = sourceTargetFrames + blockFrames + preRollFrames;
            long deviation = actualWaterline - baselineWaterline;
            maxDeviation = Math.max(maxDeviation, deviation);
            minDeviation = Math.min(minDeviation, deviation);
            int outputBytes = stream.correctDrift(buffer, sourceReadFrames * frameBytes,
                    queueFrames, pendingReportedFrames, sourceTargetFrames, nowNanos,
                    playbackAdvancing, false, sourceReadAdjustmentFrames, headAdvanced);
            int outputFrames = outputBytes / frameBytes;
            totalFramesWritten += outputFrames;
            processedBlocks++;
            elapsedFrameNanos = nowNanos;
        }
        long actualPlayedFrames = elapsedFrameNanos / 5_000_000L * blockFrames;
        long pendingActualFrames = Pcm16FrameStream.pendingFrames(totalFramesWritten, actualPlayedFrames);
        if (maxPreTrimQueueFrames > backlogTriggerFrames + blockFrames) underflows++;
        return new DriftResult(stream, processedBlocks * blockFrames, totalArrivals,
                ticks / 2, queueFrames, pendingActualFrames, maxDeviation, minDeviation, underflows,
                maxPreTrimQueueFrames, maxQueueFrames, trimmedFrames);
    }

    private static void checkDriftBurstAndTargetShift() {
        int sampleRate = 48_000;
        int blockFrames = sampleRate / 100;
        int frameBytes = 4;
        int targetFrames = 240;
        byte[] buffer = new byte[(blockFrames + Pcm16FrameStream.driftReserveFrames(sampleRate)) * frameBytes];
        Pcm16FrameStream stream = new Pcm16FrameStream(2);
        stream.beginDriftCorrection(sampleRate, targetFrames, 0);
        long nowNanos = 0;
        for (int tick = 1; tick <= 100; tick++) {
            nowNanos += 10_000_000L;
            stream.correctDrift(buffer, blockFrames * frameBytes, targetFrames, targetFrames,
                    targetFrames, nowNanos, true, false);
        }
        int shiftedTarget = targetFrames + 240;
        nowNanos += 10_000_000L;
        int shiftedOutput = stream.correctDrift(buffer, blockFrames * frameBytes,
                shiftedTarget, targetFrames, shiftedTarget, nowNanos, true, false);
        if (shiftedOutput != blockFrames * frameBytes)
            throw new AssertionError("jitter target step created a spurious correction");
        nowNanos += 10_000_000L;
        int burstOutput = stream.correctDrift(buffer, blockFrames * frameBytes,
                shiftedTarget + 240, targetFrames, shiftedTarget, nowNanos, true, false);
        if (burstOutput != blockFrames * frameBytes)
            throw new AssertionError("single 5ms input burst triggered drift correction");
        for (int tick = 0; tick < 300; tick++) {
            nowNanos += 10_000_000L;
            int queued = shiftedTarget;
            int expected = stream.correctDrift(buffer, blockFrames * frameBytes, queued,
                    targetFrames, shiftedTarget, nowNanos, true, false);
            if (expected != blockFrames * frameBytes)
                throw new AssertionError("target settling after a 5ms burst triggered drift correction");
        }
        if (stream.getInsertedDriftFrames() != 0 || stream.getDeletedDriftFrames() != 0)
            throw new AssertionError("stable zero-drift target adjustment accumulated frame edits");

        stream.correctDrift(buffer, blockFrames * frameBytes, shiftedTarget, 0,
                shiftedTarget, nowNanos + 10_000_000L, false, false);
        for (int tick = 0; tick < 100; tick++) {
            nowNanos += 10_000_000L;
            int output = stream.correctDrift(buffer, blockFrames * frameBytes, 0, 0,
                    shiftedTarget, nowNanos, true, false);
            if (output != blockFrames * frameBytes)
                throw new AssertionError("playback pause retained drift debt across the one-second warm-up");
        }
    }

    private static void checkDriftPartialReadsAndReset() throws Exception {
        byte[] expected = new byte[40];
        for (int index = 0; index < expected.length; index++) expected[index] = (byte) (index * 3);
        Pcm16FrameStream stream = new Pcm16FrameStream(2);
        byte[] buffer = new byte[16 + Pcm16FrameStream.driftReserveFrames(48_000) * 4];
        stream.beginDriftCorrection(48_000, 240, 0);
        FragmentedInput input = new FragmentedInput(expected, 1, 2, 1, 3, 5, 1, 7, 4, 2, 6, 8);
        byte[] actual = new byte[expected.length];
        int actualLength = 0;
        long nowNanos = 0;
        int sourceBytes;
        while ((sourceBytes = stream.readFrames(input, buffer, 16)) >= 0) {
            nowNanos += 10_000_000L;
            int outputBytes = stream.correctDrift(buffer, sourceBytes, 0, 0, 240,
                    nowNanos, true, false);
            if (actualLength + outputBytes > actual.length)
                throw new AssertionError("drift correction exceeded partial-read output capacity");
            System.arraycopy(buffer, 0, actual, actualLength, outputBytes);
            actualLength += outputBytes;
        }
        if (actualLength != expected.length || !Arrays.equals(actual, expected))
            throw new AssertionError("drift handling changed fragmented reads or clean EOF output");

        stream.resetDriftCorrection();
        stream.beginDriftCorrection(48_000, 240, nowNanos);
        int pausedOutput = stream.correctDrift(buffer, 16, 0, 0, 240,
                nowNanos + 10_000_000L, false, false);
        int stoppedOutput = stream.correctDrift(buffer, 16, 0, 0, 240,
                nowNanos + 20_000_000L, true, false);
        if (pausedOutput != 16 || stoppedOutput != 16
                || stream.getInsertedDriftFrames() != 0 || stream.getDeletedDriftFrames() != 0)
            throw new AssertionError("pause/stop reset retained drift history or debt");

        Pcm16FrameStream steadyStream = new Pcm16FrameStream(2);
        Pcm16FrameStream.PlaybackHeadTracker head = new Pcm16FrameStream.PlaybackHeadTracker();
        head.reset(0, 0);
        steadyStream.beginDriftCorrection(48_000, 240, 0);
        for (int tick = 1; tick <= 200; tick++) {
            long now = tick * 5_000_000L;
            if (tick % 4 == 0) head.update((int) (tick / 4 * 960), now);
            steadyStream.correctDrift(buffer, 16, 0, 0, 240, now,
                    head.isAdvancing(now), false);
        }
        if (steadyStream.getDriftBaselineFrames() < 0)
            throw new AssertionError("20ms playback-head cadence repeatedly restarted baseline warm-up");
        long baseline = steadyStream.getDriftBaselineFrames();
        for (int tick = 1; tick <= 15; tick++) {
            long now = 1_000_000_000L + tick * 5_000_000L;
            steadyStream.correctDrift(buffer, 0, 0, 0, 240, now,
                    head.isAdvancing(now), false);
        }
        if (steadyStream.getDriftBaselineFrames() != baseline)
            throw new AssertionError("frame-incomplete source reads restarted the drift baseline");
        long stoppedAt = 1_100_000_001L;
        if (head.isAdvancing(stoppedAt))
            throw new AssertionError("stalled playback head was still treated as advancing");
        steadyStream.correctDrift(buffer, 0, 0, 0, 240, stoppedAt,
                head.isAdvancing(stoppedAt), false);
        if (steadyStream.getDriftBaselineFrames() != -1
                || steadyStream.getInsertedDriftFrames() != 0 || steadyStream.getDeletedDriftFrames() != 0)
            throw new AssertionError("long playback-head stall did not reset the correction baseline");
        long stalledBaselineStart = steadyStream.getDriftBaselineStartNanos();
        for (int tick = 1; tick <= 20; tick++) {
            long now = stoppedAt + tick * 5_000_000L;
            steadyStream.correctDrift(buffer, 0, 0, 0, 240, now,
                    head.isAdvancing(now), false);
            if (steadyStream.getDriftBaselineStartNanos() != stalledBaselineStart)
                throw new AssertionError("empty reads repeatedly restarted a stalled baseline");
        }
        long resumedAt = stoppedAt + 105_000_000L;
        head.update(57_840, resumedAt);
        steadyStream.correctDrift(buffer, 16, 0, 0, 240, resumedAt,
                head.isAdvancing(resumedAt), false);
        if (steadyStream.getDriftBaselineStartNanos() != resumedAt
                || steadyStream.getDriftBaselineFrames() != -1)
            throw new AssertionError("playback resume did not start a fresh drift baseline");
    }

    private static void checkDriftReadAdjustmentPairing(boolean consumeExtraSourceFrame) {
        int sampleRate = 48_000;
        int targetFrames = sampleRate / 200;
        int blockFrames = sampleRate / 200;
        int frameBytes = 4;
        byte[] buffer = new byte[(blockFrames + Pcm16FrameStream.driftReserveFrames(sampleRate)) * frameBytes];
        Pcm16FrameStream stream = new Pcm16FrameStream(2);
        stream.beginDriftCorrection(sampleRate, targetFrames, 0);
        long nowNanos = 0;
        int adjustment = 0;
        for (int tick = 0; tick < 2_000 && adjustment == 0; tick++) {
            nowNanos += 10_000_000L;
            long sourceBufferedFrames = tick < 120
                    ? (consumeExtraSourceFrame ? 0 : blockFrames)
                    : (consumeExtraSourceFrame ? blockFrames : 0);
            int outputBytes = stream.correctDrift(buffer, blockFrames * frameBytes,
                    sourceBufferedFrames, 0, targetFrames, nowNanos, true, false, 0);
            if (outputBytes != blockFrames * frameBytes)
                throw new AssertionError("paired drift credit changed output before a read adjustment");
            adjustment = stream.driftReadFrameAdjustment(nowNanos + 5_000_000L);
        }
        int expectedAdjustment = consumeExtraSourceFrame ? 1 : -1;
        if (adjustment != expectedAdjustment)
            throw new AssertionError("drift credit did not reach the expected paired read adjustment"
                    + " requested=" + expectedAdjustment + " actual=" + adjustment);

        int adjustedSourceBytes = (blockFrames + adjustment) * frameBytes;
        Arrays.fill(buffer, (byte) 0x2a);
        int pairedOutputBytes = stream.correctDrift(buffer, adjustedSourceBytes,
                consumeExtraSourceFrame ? 0 : blockFrames, 0, targetFrames,
                nowNanos + 10_000_000L, true, true, adjustment, false);
        if (pairedOutputBytes != blockFrames * frameBytes
                || stream.getDriftBaselineFrames() != -1
                || stream.driftReadFrameAdjustment(nowNanos + 15_000_000L) != 0
                || (consumeExtraSourceFrame
                        ? stream.getDeletedDriftFrames() != 1 || stream.getInsertedDriftFrames() != 0
                        : stream.getInsertedDriftFrames() != 1 || stream.getDeletedDriftFrames() != 0)) {
            throw new AssertionError("PLC/underrun discontinuity did not pair its source read edit"
                    + " with a nominal output frame count");
        }

        int resumedBytes = stream.correctDrift(buffer, blockFrames * frameBytes, 0, 0,
                targetFrames, nowNanos + 20_000_000L, true, false);
        if (resumedBytes != blockFrames * frameBytes
                || stream.getInsertedDriftFrames() + stream.getDeletedDriftFrames() != 1) {
            throw new AssertionError("real audio recovery carried drift debt across the discontinuity");
        }
    }

    private static void checkFrameEdits(int channels, boolean insertion) {
        int sampleRate = 48_000;
        int blockFrames = sampleRate / 100;
        int frameBytes = channels * 2;
        int targetFrames = 240;
        byte[] buffer = new byte[(blockFrames + Pcm16FrameStream.driftReserveFrames(sampleRate)) * frameBytes];
        Pcm16FrameStream stream = new Pcm16FrameStream(channels);
        fillRamp(buffer, blockFrames, channels);
        stream.beginDriftCorrection(sampleRate, targetFrames, 0);
        long nowNanos = 0;
        for (int tick = 1; tick <= 100; tick++) {
            nowNanos += 10_000_000L;
            stream.correctDrift(buffer, blockFrames * frameBytes, targetFrames, targetFrames,
                    targetFrames, nowNanos, true, false);
        }

        int changedLength = blockFrames * frameBytes;
        boolean sawEdit = false;
        for (int tick = 0; tick < 500 && !sawEdit; tick++) {
            nowNanos += 10_000_000L;
            fillRamp(buffer, blockFrames, channels);
            long sourceBuffered = insertion ? 0 : 1_200;
            long pending = insertion ? 0 : targetFrames;
            changedLength = stream.correctDrift(buffer, blockFrames * frameBytes,
                    sourceBuffered, pending, targetFrames, nowNanos, true, false);
            sawEdit = changedLength != blockFrames * frameBytes;
        }
        if (!sawEdit) throw new AssertionError("drift correction failed to apply a whole PCM frame");
        if (stream.getDroppedBacklogFrames() != 0)
            throw new AssertionError("drift frame edits were counted as backlog-trim drops");
        if (insertion) {
            if (changedLength != (blockFrames + 1) * frameBytes
                    || readSample(buffer, 0) != 1_000
                    || readSample(buffer, frameBytes) != 1_002
                    || channels == 2 && (readSample(buffer, 2) != -2_000
                            || readSample(buffer, frameBytes + 2) != -2_003)) {
                throw new AssertionError("inserted PCM frame was not interleaved linear interpolation");
            }
        } else if (changedLength != (blockFrames - 1) * frameBytes
                || readSample(buffer, 0) != 1_004
                || channels == 2 && readSample(buffer, 2) != -2_006) {
            throw new AssertionError("deleted PCM frame damaged mono/stereo sample alignment");
        }
    }

    private static void fillRamp(byte[] buffer, int frames, int channels) {
        for (int frame = 0; frame < frames; frame++) {
            int left = 1_000 + frame * 4;
            buffer[frame * channels * 2] = (byte) left;
            buffer[frame * channels * 2 + 1] = (byte) (left >> 8);
            if (channels == 2) {
                int right = -2_000 - frame * 6;
                buffer[frame * channels * 2 + 2] = (byte) right;
                buffer[frame * channels * 2 + 3] = (byte) (right >> 8);
            }
        }
    }

    private static final class DriftResult {
        final Pcm16FrameStream stream;
        final long nominalSourceFrames;
        final long totalArrivals;
        final long durationTicks;
        final long queueFrames;
        final long pendingFrames;
        final long maxDeviation;
        final long minDeviation;
        final int underflows;
        final long maxPreTrimQueueFrames;
        final long maxQueueFrames;
        final long trimmedFrames;

        DriftResult(Pcm16FrameStream stream, long nominalSourceFrames, long totalArrivals,
                long durationTicks,
                long queueFrames, long pendingFrames, long maxDeviation, long minDeviation,
                int underflows) {
            this(stream, nominalSourceFrames, totalArrivals, durationTicks, queueFrames,
                    pendingFrames, maxDeviation, minDeviation, underflows, 0, 0, 0);
        }

        DriftResult(Pcm16FrameStream stream, long nominalSourceFrames, long totalArrivals,
                long durationTicks,
                long queueFrames, long pendingFrames, long maxDeviation, long minDeviation,
                int underflows, long maxPreTrimQueueFrames, long maxQueueFrames, long trimmedFrames) {
            this.stream = stream;
            this.nominalSourceFrames = nominalSourceFrames;
            this.totalArrivals = totalArrivals;
            this.durationTicks = durationTicks;
            this.queueFrames = queueFrames;
            this.pendingFrames = pendingFrames;
            this.maxDeviation = maxDeviation;
            this.minDeviation = minDeviation;
            this.underflows = underflows;
            this.maxPreTrimQueueFrames = maxPreTrimQueueFrames;
            this.maxQueueFrames = maxQueueFrames;
            this.trimmedFrames = trimmedFrames;
        }
    }

    private static int readSample(byte[] data, int offset) {
        return (short) ((data[offset] & 0xff) | (data[offset + 1] << 8));
    }

    private static byte[] toArray(List<Byte> bytes) {
        byte[] result = new byte[bytes.size()];
        for (int i = 0; i < result.length; i++) result[i] = bytes.get(i);
        return result;
    }

    private static final class FragmentedInput extends InputStream {
        private final byte[] data;
        private final int[] fragments;
        private int offset;
        private int next;

        FragmentedInput(byte[] data, int... fragments) { this.data = data; this.fragments = fragments; }
        @Override public int read() { return offset == data.length ? -1 : data[offset++] & 0xff; }
        @Override public int read(byte[] target, int start, int length) {
            if (offset == data.length) return -1;
            int count = Math.min(Math.min(length, fragments[next++ % fragments.length]), data.length - offset);
            System.arraycopy(data, offset, target, start, count);
            offset += count;
            return count;
        }
    }

    private static final class BacklogInput extends InputStream {
        private final byte[] data;
        private int offset;
        int availableCalls;

        BacklogInput(byte[] data) { this.data = data; }
        @Override public int available() { return availableCalls++ == 0 ? 0 : data.length - offset; }
        @Override public int read() { return offset == data.length ? -1 : data[offset++] & 0xff; }
        int unreadBytes() { return data.length - offset; }
        @Override public int read(byte[] target, int start, int length) {
            if (offset == data.length) return -1;
            int count = availableCalls == 1 ? 1 : Math.min(length, data.length - offset);
            System.arraycopy(data, offset, target, start, count);
            offset += count;
            return count;
        }
    }
}
