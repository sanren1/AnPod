package com.anpod.server;

import java.io.IOException;
import java.io.InputStream;

/** Preserves PCM16 stereo frame boundaries across arbitrary socket reads/writes. */
final class Pcm16FrameStream {
    static final int FRAME_BYTES = 4;
    private final byte[] tail = new byte[FRAME_BYTES - 1];
    private int tailLength;

    interface Writer {
        int write(byte[] data, int offset, int length);
    }

    /** Returns complete frames, -1 at clean EOF; retains up to three bytes. */
    int readFrames(InputStream input, byte[] buffer, int maxBytes) throws IOException {
        int limit = maxBytes - maxBytes % FRAME_BYTES;
        if (limit < FRAME_BYTES || buffer.length < limit) {
            throw new IllegalArgumentException("buffer must fit at least one complete frame");
        }

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
                throw new IOException("PCM stream ended with an incomplete stereo frame (" + prefix + " bytes)");
            }
            tailLength = 0;
            return -1;
        }

        int total = prefix + count;
        int complete = total - total % FRAME_BYTES;
        tailLength = total - complete;
        if (tailLength > 0) {
            System.arraycopy(buffer, complete, tail, 0, tailLength);
        }
        return complete;
    }

    static void writeFrames(Writer writer, byte[] buffer, int length) throws IOException {
        if (length % FRAME_BYTES != 0) {
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
            if (written % FRAME_BYTES != 0) {
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
