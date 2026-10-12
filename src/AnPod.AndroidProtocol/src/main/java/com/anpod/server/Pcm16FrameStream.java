package com.anpod.server;

import java.io.IOException;
import java.io.InputStream;

/** Preserves PCM16 frame boundaries across arbitrary socket reads/writes. */
public final class Pcm16FrameStream {
    public static final int FRAME_BYTES = 4;
    private final byte[] tail = new byte[FRAME_BYTES - 1];
    private final int frameBytes;
    private int tailLength;
    private int maxBacklogFrames = -1;
    private int fadeFramesTotal;
    private int fadeFramesRemaining;
    private long droppedBacklogFrames;

    public interface Writer {
        int write(byte[] data, int offset, int length) throws IOException;
    }

    public Pcm16FrameStream() { this(2); }

    public Pcm16FrameStream(int channels) {
        if (channels != 1 && channels != 2) throw new IllegalArgumentException("channels must be mono or stereo");
        frameBytes = channels * 2;
    }

    /** Bounds socket backlog after accounting for the AudioTrack pre-roll window. */
    public static int backlogLimitFrames(int sampleRate, int preRollFrames, boolean lowLatency) {
        int fiveMs = millisToFrames(sampleRate, 5);
        int lowLatencyLimit = Math.max(fiveMs, millisToFrames(sampleRate, 20) - preRollFrames);
        return lowLatency ? lowLatencyLimit
                : Math.max(lowLatencyLimit + 1, preRollFrames + millisToFrames(sampleRate, 20));
    }

    public void setBacklogLimit(int maxFrames, int fadeFrames) {
        if (maxFrames < 0 || fadeFrames < 0) {
            throw new IllegalArgumentException("backlog and fade frame limits must not be negative");
        }
        maxBacklogFrames = maxFrames;
        fadeFramesTotal = fadeFrames;
    }

    public long getDroppedBacklogFrames() { return droppedBacklogFrames; }

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
        if (maxBacklogFrames < 0) return;
        int available = input.available();
        long completeFrames = (tailLength + (long) available) / frameBytes;
        long excessFrames = completeFrames - maxBacklogFrames;
        if (excessFrames <= 0) return;

        int framesToDrop = (int) Math.min(excessFrames, Math.max(1, buffer.length / frameBytes));
        int inputBytesToDrop = framesToDrop * frameBytes - tailLength;
        int remaining = inputBytesToDrop;
        while (remaining > 0) {
            int count = input.read(buffer, 0, Math.min(remaining, buffer.length));
            if (count < 0) throw new IOException("PCM backlog trim ended before a frame boundary");
            if (count == 0) throw new IOException("PCM backlog trim made no progress");
            remaining -= count;
        }

        tailLength = 0;
        droppedBacklogFrames += framesToDrop;
        fadeFramesRemaining = fadeFramesTotal;
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
