package com.anpod.server;

import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;

/** Preserves PCM16 frame boundaries across arbitrary socket reads/writes. */
public final class Pcm16FrameStream {
    public static final int FRAME_BYTES = 4;
    public static final int LOW_LATENCY_AUDIO_TRACK_BUFFER_MS = 15;
    public static final int LOW_LATENCY_START_THRESHOLD_MS = 5;
    public static final int BALANCED_AUDIO_TRACK_BUFFER_MS = 50;
    private static final long DRIFT_BASELINE_NANOS = 1_000_000_000L;
    private static final long DRIFT_BASELINE_SAMPLE_NANOS = 10_000_000L;
    private static final long DRIFT_PERSISTENCE_NANOS = 2_000_000_000L;
    private static final long DRIFT_MAX_SAMPLE_GAP_NANOS = 100_000_000L;
    private static final long DRIFT_CREDIT_DENOMINATOR = 1_000_000_000_000L;
    private final byte[] tail = new byte[FRAME_BYTES - 1];
    private final long[] driftBaselineSamples = new long[101];
    private final long[] driftBufferedBaselineSamples = new long[101];
    private final int frameBytes;
    private int tailLength;
    private int targetBacklogFrames = -1;
    private int triggerBacklogFrames = -1;
    private int fadeFramesTotal;
    private int fadeFramesRemaining;
    private long droppedBacklogFrames;
    private boolean driftEnabled;
    private boolean driftBaselineReady;
    private boolean driftPlaybackAdvancing = true;
    private int driftSampleRate;
    private int driftSourceTargetFrames;
    private int driftBaselineSampleCount;
    private long driftBaselineStartNanos;
    private long driftNextBaselineSampleNanos;
    private long driftBaselineOffsetFrames;
    private long driftBufferedBaselineOffsetFrames;
    private long driftLastObservationNanos;
    private long driftLastFrameAdjustmentNanos;
    private long driftOutsideSinceNanos;
    private int driftOutsideDirection;
    private double driftFilteredErrorFrames;
    private long driftCreditNumerator;
    private long insertedDriftFrames;
    private long deletedDriftFrames;

    public interface Writer {
        int write(byte[] data, int offset, int length) throws IOException;
    }

    public Pcm16FrameStream() { this(2); }

    public Pcm16FrameStream(int channels) {
        if (channels != 1 && channels != 2) throw new IllegalArgumentException("channels must be mono or stereo");
        frameBytes = channels * 2;
    }

    /** Network PCM target, independent of AudioTrack pre-roll: 5ms low-latency, 10ms balanced. */
    public static int backlogTargetFrames(int sampleRate, boolean lowLatency) {
        return millisToFrames(sampleRate, lowLatency ? 5 : 10);
    }

    /** Trim trigger includes a separate 10ms low-latency or 20ms balanced burst allowance. */
    public static int backlogTriggerFrames(int sampleRate, boolean lowLatency) {
        int targetFrames = backlogTargetFrames(sampleRate, lowLatency);
        int triggerFrames = millisToFrames(sampleRate, lowLatency ? 15 : 30);
        return Math.max(targetFrames + 1, triggerFrames);
    }

    public static int requestedAudioTrackBufferFrames(int sampleRate, int capacityFrames, boolean lowLatency) {
        if (sampleRate <= 0 || capacityFrames <= 0) return 0;
        int targetMs = lowLatency ? LOW_LATENCY_AUDIO_TRACK_BUFFER_MS : BALANCED_AUDIO_TRACK_BUFFER_MS;
        return Math.min(capacityFrames, Math.max(1, sampleRate * targetMs / 1000));
    }

    /** Maximum insertion room needed for a bounded 100ms control interval at 1000ppm. */
    public static int driftReserveFrames(int sampleRate) {
        return sampleRate <= 0 ? 0 : (sampleRate + 9_999) / 10_000;
    }

    public static long pendingFrames(long framesWritten, long framesPlayed) {
        return Math.max(0, framesWritten - framesPlayed);
    }

    public static final class PlaybackHeadTracker {
        private static final long HEAD_MASK = 0xffff_ffffL;
        private static final long MAX_FORWARD_DELTA = 0x8000_0000L;
        private static final long STALLED_NANOS = 100_000_000L;

        private boolean initialized;
        private long lastPosition;
        private long framesPlayed;
        private long lastProgressNanos;

        public synchronized void reset(int position) {
            reset(position, System.nanoTime());
        }

        public synchronized void reset(int position, long nowNanos) {
            initialized = true;
            lastPosition = Integer.toUnsignedLong(position);
            framesPlayed = 0;
            lastProgressNanos = nowNanos;
        }

        public synchronized void reset() {
            initialized = false;
            lastPosition = 0;
            framesPlayed = 0;
            lastProgressNanos = 0;
        }

        public long update(int position) {
            return update(position, System.nanoTime());
        }

        public synchronized long update(int position, long nowNanos) {
            long current = Integer.toUnsignedLong(position);
            if (!initialized) {
                initialized = true;
                lastPosition = current;
                lastProgressNanos = nowNanos;
                return framesPlayed;
            }
            long advance = (current - lastPosition) & HEAD_MASK;
            if (advance > 0 && advance < MAX_FORWARD_DELTA) {
                framesPlayed += advance;
                lastPosition = current;
                lastProgressNanos = nowNanos;
            }
            return framesPlayed;
        }

        public synchronized boolean isAdvancing(long nowNanos) {
            long sinceProgressNanos = nowNanos - lastProgressNanos;
            return sinceProgressNanos >= 0 && sinceProgressNanos <= STALLED_NANOS;
        }
    }

    public void setBacklogLimit(int targetFrames, int triggerFrames, int fadeFrames) {
        if (targetFrames < 0 || triggerFrames < targetFrames || fadeFrames < 0) {
            throw new IllegalArgumentException("backlog target, trigger, and fade limits are inconsistent");
        }
        targetBacklogFrames = targetFrames;
        triggerBacklogFrames = triggerFrames;
        fadeFramesTotal = fadeFrames;
    }

    public long getDroppedBacklogFrames() { return droppedBacklogFrames; }

    public long getInsertedDriftFrames() { return insertedDriftFrames; }
    public long getDeletedDriftFrames() { return deletedDriftFrames; }
    /** Frames to add to the next source read: positive consumes more source, negative less. */
    public int driftReadFrameAdjustment(long nowNanos) {
        if (!driftEnabled) return 0;
        long maxRateIntervalNanos = (1_000_000_000L + Math.max(1, driftSampleRate / 1_000) - 1)
                / Math.max(1, driftSampleRate / 1_000);
        if (nowNanos - driftLastFrameAdjustmentNanos < maxRateIntervalNanos) return 0;
        if (driftCreditNumerator <= -DRIFT_CREDIT_DENOMINATOR) return 1;
        if (driftCreditNumerator >= DRIFT_CREDIT_DENOMINATOR) return -1;
        return 0;
    }
    long getDriftBaselineFrames() {
        return driftBaselineReady ? driftBaselineOffsetFrames + driftSourceTargetFrames : -1;
    }
    long getDriftBufferedBaselineFrames() {
        return driftBaselineReady
                ? driftBufferedBaselineOffsetFrames + driftSourceTargetFrames : -1;
    }
    long getDriftBaselineStartNanos() { return driftBaselineStartNanos; }

    /** Starts a fresh one-second waterline baseline after playback is actually started. */
    public void beginDriftCorrection(int sampleRate, int sourceTargetFrames, long nowNanos) {
        if (sampleRate <= 0 || sourceTargetFrames < 0)
            throw new IllegalArgumentException("invalid drift correction format or target");
        driftEnabled = true;
        driftBaselineReady = false;
        driftPlaybackAdvancing = true;
        driftSampleRate = sampleRate;
        driftSourceTargetFrames = sourceTargetFrames;
        driftBaselineSampleCount = 0;
        driftBaselineStartNanos = nowNanos;
        driftNextBaselineSampleNanos = nowNanos;
        driftBaselineOffsetFrames = 0;
        driftBufferedBaselineOffsetFrames = 0;
        driftLastObservationNanos = nowNanos;
        driftLastFrameAdjustmentNanos = nowNanos;
        clearDriftDebt();
    }

    /** Drops all drift history; the next playback start must establish a new baseline. */
    public void resetDriftCorrection() {
        driftEnabled = false;
        driftBaselineReady = false;
        driftBaselineSampleCount = 0;
        driftBaselineOffsetFrames = 0;
        driftBufferedBaselineOffsetFrames = 0;
        clearDriftDebt();
    }

    /**
     * Applies bounded whole-frame drift correction in place. The returned length is output frames;
     * sourceBytes remains the number of source bytes read from the PCM stream.
     */
    public int correctDrift(byte[] buffer, int sourceBytes, long sourceBufferedFrames,
            long pendingTrackFrames, int sourceTargetFrames, long nowNanos,
            boolean playbackAdvancing, boolean discontinuity) {
        return correctDriftInternal(buffer, sourceBytes, sourceBufferedFrames, pendingTrackFrames,
                sourceTargetFrames, nowNanos, playbackAdvancing, discontinuity, 0, true, true);
    }

    public int correctDrift(byte[] buffer, int sourceBytes, long sourceBufferedFrames,
            long pendingTrackFrames, int sourceTargetFrames, long nowNanos,
            boolean playbackAdvancing, boolean discontinuity, int sourceReadAdjustmentFrames) {
        return correctDriftInternal(buffer, sourceBytes, sourceBufferedFrames, pendingTrackFrames,
                sourceTargetFrames, nowNanos, playbackAdvancing, discontinuity,
                sourceReadAdjustmentFrames, false, true);
    }

    /** Pairs any requested source-read edit while sampling waterline only after playback-head progress. */
    public int correctDrift(byte[] buffer, int sourceBytes, long sourceBufferedFrames,
            long pendingTrackFrames, int sourceTargetFrames, long nowNanos,
            boolean playbackAdvancing, boolean discontinuity, int sourceReadAdjustmentFrames,
            boolean observePlaybackWaterline) {
        return correctDriftInternal(buffer, sourceBytes, sourceBufferedFrames, pendingTrackFrames,
                sourceTargetFrames, nowNanos, playbackAdvancing, discontinuity,
                sourceReadAdjustmentFrames, false, observePlaybackWaterline);
    }

    private int correctDriftInternal(byte[] buffer, int sourceBytes, long sourceBufferedFrames,
            long pendingTrackFrames, int sourceTargetFrames, long nowNanos,
            boolean playbackAdvancing, boolean discontinuity, int sourceReadAdjustmentFrames,
            boolean allowUnpairedEdits, boolean observePlaybackWaterline) {
        if (buffer == null) throw new NullPointerException("buffer");
        if (sourceBytes < 0 || sourceBytes % frameBytes != 0 || sourceBytes > buffer.length
                || sourceBufferedFrames < 0 || pendingTrackFrames < 0 || sourceTargetFrames < 0
                || sourceReadAdjustmentFrames < -1 || sourceReadAdjustmentFrames > 1) {
            throw new IllegalArgumentException("invalid drift waterline or PCM length");
        }
        if (sourceReadAdjustmentFrames != 0) {
            int adjustedBytes = sourceBytes;
            long correction = sourceReadAdjustmentFrames > 0
                    ? -DRIFT_CREDIT_DENOMINATOR : DRIFT_CREDIT_DENOMINATOR;
            if (sourceReadAdjustmentFrames > 0 && sourceBytes / frameBytes >= 2) {
                deleteLowDifferenceFrame(buffer, sourceBytes);
                adjustedBytes -= frameBytes;
                deletedDriftFrames++;
            } else if (sourceReadAdjustmentFrames < 0
                    && sourceBytes / frameBytes > 0
                    && buffer.length - sourceBytes >= frameBytes) {
                insertInterpolatedFrame(buffer, sourceBytes);
                adjustedBytes += frameBytes;
                insertedDriftFrames++;
            } else {
                throw new IllegalArgumentException("source read adjustment does not fit PCM buffer");
            }
            if (sourceReadAdjustmentFrames > 0 && driftCreditNumerator <= -DRIFT_CREDIT_DENOMINATOR
                    || sourceReadAdjustmentFrames < 0
                    && driftCreditNumerator >= DRIFT_CREDIT_DENOMINATOR) {
                driftCreditNumerator -= correction;
            }
            driftLastFrameAdjustmentNanos = nowNanos;
            sourceBytes = adjustedBytes;
        }
        if (!driftEnabled) return sourceBytes;

        if (discontinuity) {
            beginDriftCorrection(driftSampleRate, sourceTargetFrames, nowNanos);
            driftPlaybackAdvancing = playbackAdvancing;
            return sourceBytes;
        }
        if (!playbackAdvancing) {
            if (driftPlaybackAdvancing) beginDriftCorrection(driftSampleRate, sourceTargetFrames, nowNanos);
            driftPlaybackAdvancing = false;
            return sourceBytes;
        }
        if (!observePlaybackWaterline) return sourceBytes;
        if (!driftPlaybackAdvancing) beginDriftCorrection(driftSampleRate, sourceTargetFrames, nowNanos);
        driftPlaybackAdvancing = true;
        if (sourceBytes == 0) {
            return 0;
        }
        long elapsedNanos = nowNanos - driftLastObservationNanos;
        if (elapsedNanos <= 0 || elapsedNanos > DRIFT_MAX_SAMPLE_GAP_NANOS) {
            beginDriftCorrection(driftSampleRate, sourceTargetFrames, nowNanos);
            return sourceBytes;
        }

        if (sourceTargetFrames != driftSourceTargetFrames) {
            driftSourceTargetFrames = sourceTargetFrames;
            clearDriftDebt();
        }

        long waterlineFrames = sourceBufferedFrames + sourceBytes / frameBytes + pendingTrackFrames;
        if (!driftBaselineReady) {
            collectDriftBaseline(nowNanos, waterlineFrames, sourceBufferedFrames,
                    sourceBytes / frameBytes, sourceTargetFrames);
            driftLastObservationNanos = nowNanos;
            return sourceBytes;
        }

        double errorFrames = waterlineFrames - (driftBaselineOffsetFrames + sourceTargetFrames);
        double alpha = elapsedNanos / (double) (DRIFT_BASELINE_NANOS + elapsedNanos);
        driftFilteredErrorFrames += (errorFrames - driftFilteredErrorFrames) * alpha;
        long deadbandFrames = driftDeadbandFrames(driftBufferedBaselineOffsetFrames + sourceTargetFrames,
                driftSampleRate);
        int direction = driftFilteredErrorFrames > deadbandFrames ? 1
                : driftFilteredErrorFrames < -deadbandFrames ? -1 : 0;
        if (direction == 0) {
            clearDriftDebt();
            driftLastObservationNanos = nowNanos;
            return sourceBytes;
        }
        if (direction != driftOutsideDirection) {
            driftOutsideDirection = direction;
            driftOutsideSinceNanos = nowNanos;
            driftCreditNumerator = 0;
        } else if (nowNanos - driftOutsideSinceNanos >= DRIFT_PERSISTENCE_NANOS) {
            long excessFrames = (long) Math.abs(driftFilteredErrorFrames) - deadbandFrames;
            int correctionPpm = (int) Math.min(1_000L,
                    excessFrames * 1_000L / Math.max(1, deadbandFrames));
            long credit = (long) driftSampleRate * elapsedNanos * correctionPpm / 1_000L;
            // 1e12 numerator units per frame; correctionPpm rises proportionally to excess waterline.
            driftCreditNumerator += direction > 0 ? -credit : credit;
        }

        int outputBytes = sourceBytes;
        long wholeEdits = allowUnpairedEdits
                ? Math.abs(driftCreditNumerator) / DRIFT_CREDIT_DENOMINATOR : 0;
        int frameCount = sourceBytes / frameBytes;
        int capacityEdits = driftCreditNumerator > 0
                ? frameCount < 2 ? 0 : (buffer.length - outputBytes) / frameBytes
                : Math.max(0, frameCount - 2);
        int edits = (int) Math.min(wholeEdits, capacityEdits);
        for (int edit = 0; edit < edits; edit++) {
            if (driftCreditNumerator > 0) {
                insertInterpolatedFrame(buffer, outputBytes);
                outputBytes += frameBytes;
                frameCount++;
                insertedDriftFrames++;
                driftCreditNumerator -= DRIFT_CREDIT_DENOMINATOR;
            } else {
                deleteLowDifferenceFrame(buffer, outputBytes);
                outputBytes -= frameBytes;
                frameCount--;
                deletedDriftFrames++;
                driftCreditNumerator += DRIFT_CREDIT_DENOMINATOR;
            }
        }
        if (edits < wholeEdits) {
            driftCreditNumerator = driftCreditNumerator > 0
                    ? DRIFT_CREDIT_DENOMINATOR - 1 : -(DRIFT_CREDIT_DENOMINATOR - 1);
        }
        driftLastObservationNanos = nowNanos;
        return outputBytes;
    }

    private void collectDriftBaseline(long nowNanos, long waterlineFrames, long sourceBufferedFrames,
            int sourceBlockFrames, int sourceTargetFrames) {
        if (nowNanos >= driftNextBaselineSampleNanos
                && driftBaselineSampleCount < driftBaselineSamples.length) {
            driftBaselineSamples[driftBaselineSampleCount] = waterlineFrames - sourceTargetFrames;
            driftBufferedBaselineSamples[driftBaselineSampleCount] = sourceBufferedFrames - sourceTargetFrames;
            driftBaselineSampleCount++;
            driftNextBaselineSampleNanos = nowNanos + DRIFT_BASELINE_SAMPLE_NANOS;
        }
        if (nowNanos - driftBaselineStartNanos < DRIFT_BASELINE_NANOS
                || driftBaselineSampleCount == 0) return;

        for (int index = 1; index < driftBaselineSampleCount; index++) {
            long sample = driftBaselineSamples[index];
            int insert = index;
            while (insert > 0 && driftBaselineSamples[insert - 1] > sample) {
                driftBaselineSamples[insert] = driftBaselineSamples[insert - 1];
                insert--;
            }
            driftBaselineSamples[insert] = sample;
        }
        driftBaselineOffsetFrames = driftBaselineSamples[driftBaselineSampleCount / 2];
        for (int index = 1; index < driftBaselineSampleCount; index++) {
            long sample = driftBufferedBaselineSamples[index];
            int insert = index;
            while (insert > 0 && driftBufferedBaselineSamples[insert - 1] > sample) {
                driftBufferedBaselineSamples[insert] = driftBufferedBaselineSamples[insert - 1];
                insert--;
            }
            driftBufferedBaselineSamples[insert] = sample;
        }
        driftBufferedBaselineOffsetFrames =
                driftBufferedBaselineSamples[driftBaselineSampleCount / 2];
        driftBaselineReady = true;
        driftFilteredErrorFrames = 0;
        clearDriftDebt();
    }

    private static long driftDeadbandFrames(long baselineFrames, int sampleRate) {
        long minimum = Math.max(1, (sampleRate + 999L) / 1_000L);
        long maximum = Math.max(minimum, (sampleRate * 10L + 999L) / 1_000L);
        long quarter = Math.max(1, baselineFrames / 4);
        return Math.max(minimum, Math.min(maximum, quarter));
    }

    private void clearDriftDebt() {
        driftCreditNumerator = 0;
        driftOutsideSinceNanos = 0;
        driftOutsideDirection = 0;
    }

    private void insertInterpolatedFrame(byte[] buffer, int length) {
        int frames = length / frameBytes;
        int pair = lowestDifferencePair(buffer, frames);
        int sourceOffset = (pair + 1) * frameBytes;
        int destinationOffset = (pair + 2) * frameBytes;
        System.arraycopy(buffer, sourceOffset, buffer, destinationOffset, length - sourceOffset);
        for (int channel = 0; channel < frameBytes; channel += 2) {
            int left = readSample(buffer, pair * frameBytes + channel);
            int right = readSample(buffer, (pair + 2) * frameBytes + channel);
            writeSample(buffer, (pair + 1) * frameBytes + channel, (left + right) / 2);
        }
    }

    private void deleteLowDifferenceFrame(byte[] buffer, int length) {
        int frames = length / frameBytes;
        int remove = lowestDifferenceFrame(buffer, frames);
        for (int channel = 0; channel < frameBytes; channel += 2) {
            int before = readSample(buffer, (remove - 1) * frameBytes + channel);
            int after = readSample(buffer, (remove + 1) * frameBytes + channel);
            writeSample(buffer, (remove - 1) * frameBytes + channel, (before + after) / 2);
        }
        int sourceOffset = (remove + 1) * frameBytes;
        System.arraycopy(buffer, sourceOffset, buffer, remove * frameBytes, length - sourceOffset);
    }

    private int lowestDifferencePair(byte[] buffer, int frames) {
        int stride = Math.max(1, frames / 64);
        int bestPair = 0;
        long bestDifference = Long.MAX_VALUE;
        for (int pair = 0; pair < frames - 1; pair += stride) {
            long difference = 0;
            for (int channel = 0; channel < frameBytes; channel += 2)
                difference += Math.abs(readSample(buffer, pair * frameBytes + channel)
                        - readSample(buffer, (pair + 1) * frameBytes + channel));
            if (difference < bestDifference) {
                bestDifference = difference;
                bestPair = pair;
            }
        }
        return bestPair;
    }

    private int lowestDifferenceFrame(byte[] buffer, int frames) {
        int stride = Math.max(1, frames / 64);
        int bestFrame = 1;
        long bestDifference = Long.MAX_VALUE;
        for (int frame = 1; frame < frames - 1; frame += stride) {
            long difference = 0;
            for (int channel = 0; channel < frameBytes; channel += 2)
                difference += Math.abs(readSample(buffer, (frame - 1) * frameBytes + channel)
                        - readSample(buffer, (frame + 1) * frameBytes + channel));
            if (difference < bestDifference) {
                bestDifference = difference;
                bestFrame = frame;
            }
        }
        return bestFrame;
    }

    private static int readSample(byte[] buffer, int offset) {
        return (short) ((buffer[offset] & 0xff) | (buffer[offset + 1] << 8));
    }

    private static void writeSample(byte[] buffer, int offset, int sample) {
        buffer[offset] = (byte) sample;
        buffer[offset + 1] = (byte) (sample >> 8);
    }

    /** Returns complete frames, -1 at clean EOF; retains up to three bytes. */
    public int readFrames(InputStream input, byte[] buffer, int maxBytes) throws IOException {
        int limit = maxBytes - maxBytes % frameBytes;
        if (limit < frameBytes || buffer.length < limit) {
            throw new IllegalArgumentException("buffer must fit at least one complete frame");
        }

        trimBacklog(input, buffer);

        int prefix = tailLength;
        System.arraycopy(tail, 0, buffer, 0, prefix);
        int count;
        do {
            count = input.read(buffer, prefix, limit - prefix);
            if (count == 0) {
                pauseOnNoProgress();
            }
        } while (count == 0);

        if (count < 0) {
            if (prefix != 0) {
                throw new IOException("PCM stream ended with an incomplete "
                        + (frameBytes == FRAME_BYTES ? "stereo " : "") + "frame (" + prefix + " bytes)");
            }
            tailLength = 0;
            return -1;
        }

        int total = prefix + count;
        int complete = total - total % frameBytes;
        tailLength = total - complete;
        if (tailLength > 0) {
            System.arraycopy(buffer, complete, tail, 0, tailLength);
        }
        fadeIn(buffer, complete);
        return complete;
    }

    private void trimBacklog(InputStream input, byte[] buffer) throws IOException {
        if (triggerBacklogFrames < 0) return;
        int available = input.available();
        long completeFrames = (tailLength + (long) available) / frameBytes;
        if (completeFrames <= triggerBacklogFrames) return;

        long framesToDrop = completeFrames - targetBacklogFrames;
        long remaining = framesToDrop * frameBytes - tailLength;
        while (remaining > 0) {
            if (Thread.currentThread().isInterrupted()) {
                throw new InterruptedIOException("PCM backlog trim interrupted");
            }
            int count = input.read(buffer, 0, (int) Math.min(remaining, (long) buffer.length));
            if (count < 0) throw new IOException("PCM backlog trim ended before a frame boundary");
            if (count == 0) {
                pauseOnNoProgress();
                continue;
            }
            remaining -= count;
        }

        tailLength = 0;
        droppedBacklogFrames += framesToDrop;
        fadeFramesRemaining = fadeFramesTotal;
        if (Thread.currentThread().isInterrupted()) {
            throw new InterruptedIOException("PCM backlog trim interrupted");
        }
    }

    private void fadeIn(byte[] buffer, int length) {
        if (fadeFramesRemaining == 0) return;
        int frames = Math.min(fadeFramesRemaining, length / frameBytes);
        for (int frame = 0; frame < frames; frame++) {
            int gainNumerator = fadeFramesTotal - fadeFramesRemaining;
            int offset = frame * frameBytes;
            for (int channelOffset = 0; channelOffset < frameBytes; channelOffset += 2) {
                int sample = (short) ((buffer[offset + channelOffset] & 0xff)
                        | (buffer[offset + channelOffset + 1] << 8));
                int faded = sample * gainNumerator / fadeFramesTotal;
                buffer[offset + channelOffset] = (byte) faded;
                buffer[offset + channelOffset + 1] = (byte) (faded >> 8);
            }
            fadeFramesRemaining--;
        }
    }

    private static int millisToFrames(int sampleRate, int millis) {
        return (sampleRate * millis + 999) / 1000;
    }

    public void writeFramesForChannels(Writer writer, byte[] buffer, int length) throws IOException {
        writeAlignedFrames(writer, buffer, length, frameBytes);
    }

    public static void writeFrames(Writer writer, byte[] buffer, int length) throws IOException {
        writeAlignedFrames(writer, buffer, length, FRAME_BYTES);
    }

    private static void writeAlignedFrames(Writer writer, byte[] buffer, int length, int frameBytes) throws IOException {
        if (length % frameBytes != 0) {
            throw new IllegalArgumentException("write length is not frame aligned");
        }

        int offset = 0;
        while (offset < length) {
            int written = writer.write(buffer, offset, length - offset);
            if (written < 0) {
                throw new IOException("AudioTrack.write failed: " + written);
            }
            if (written == 0) {
                pauseOnNoProgress();
                continue;
            }
            if (written % frameBytes != 0) {
                throw new IOException("AudioTrack.write returned a partial PCM frame: " + written);
            }
            offset += written;
        }
    }

    private static void pauseOnNoProgress() throws IOException {
        try {
            Thread.sleep(2);
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new IOException("PCM stream interrupted", interrupted);
        }
    }
}
