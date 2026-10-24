package com.anpod.server;

/** Signed little-endian PCM24 packed conversion to and from normalized Float32 samples. */
public final class Pcm24PackedCodec {
    public static final int BYTES_PER_SAMPLE = 3;
    public static final int SCALE = 1 << 23;
    private static final int MIN_SAMPLE = -SCALE;
    private static final int MAX_SAMPLE = SCALE - 1;

    private Pcm24PackedCodec() { }

    public static int bytesForSamples(int sampleCount) {
        if (sampleCount < 0 || sampleCount > Integer.MAX_VALUE / BYTES_PER_SAMPLE)
            throw new IllegalArgumentException("invalid PCM24 sample count");
        return sampleCount * BYTES_PER_SAMPLE;
    }

    public static void decodeToFloat(byte[] packed, int byteOffset, int sampleCount,
            float[] decoded, int sampleOffset) {
        if (packed == null || decoded == null) throw new NullPointerException();
        int byteCount = bytesForSamples(sampleCount);
        checkRange(packed.length, byteOffset, byteCount);
        checkRange(decoded.length, sampleOffset, sampleCount);
        for (int sample = 0, source = byteOffset, destination = sampleOffset;
                sample < sampleCount; sample++, source += 3, destination++) {
            int value = (packed[source] & 0xff)
                    | ((packed[source + 1] & 0xff) << 8)
                    | (packed[source + 2] << 16);
            if ((value & 0x0080_0000) != 0) value |= 0xff00_0000;
            decoded[destination] = value / (float) SCALE;
        }
    }

    public static void encodeFromFloat(float[] samples, int sampleOffset, int sampleCount,
            byte[] packed, int byteOffset) {
        if (samples == null || packed == null) throw new NullPointerException();
        int byteCount = bytesForSamples(sampleCount);
        checkRange(samples.length, sampleOffset, sampleCount);
        checkRange(packed.length, byteOffset, byteCount);
        for (int sample = 0, source = sampleOffset, destination = byteOffset;
                sample < sampleCount; sample++, source++, destination += 3) {
            int value = quantize(samples[source]);
            packed[destination] = (byte) value;
            packed[destination + 1] = (byte) (value >>> 8);
            packed[destination + 2] = (byte) (value >>> 16);
        }
    }

    static int quantize(float sample) {
        if (Float.isNaN(sample) || Float.isInfinite(sample))
            throw new IllegalArgumentException("PCM24 input must be finite");
        double clipped = Math.max(-1.0, Math.min(1.0, (double) sample));
        double scaled = clipped * SCALE;
        long rounded = scaled < 0
                ? -(long) Math.floor(-scaled + 0.5)
                : (long) Math.floor(scaled + 0.5);
        return (int) Math.max(MIN_SAMPLE, Math.min(MAX_SAMPLE, rounded));
    }

    private static void checkRange(int length, int offset, int count) {
        if (offset < 0 || count < 0 || offset > length - count)
            throw new IllegalArgumentException("PCM24 range is outside its buffer");
    }
}
