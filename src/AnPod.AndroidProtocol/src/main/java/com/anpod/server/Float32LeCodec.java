package com.anpod.server;

/** IEEE754 Float32 little-endian PCM conversion without changing finite sample bits. */
public final class Float32LeCodec {
    private Float32LeCodec() { }

    public static void decode(byte[] packed, int byteOffset, int sampleCount,
            float[] decoded, int sampleOffset) {
        if (packed == null || decoded == null) throw new NullPointerException();
        if (byteOffset < 0 || sampleCount < 0 || (long) byteOffset + (long) sampleCount * 4 > packed.length
                || sampleOffset < 0 || sampleOffset > decoded.length - sampleCount)
            throw new IllegalArgumentException("Float32 range is outside its buffer");
        for (int i = 0; i < sampleCount; i++) {
            int offset = byteOffset + i * 4;
            int bits = (packed[offset] & 0xff) | ((packed[offset + 1] & 0xff) << 8)
                    | ((packed[offset + 2] & 0xff) << 16) | (packed[offset + 3] << 24);
            float sample = Float.intBitsToFloat(bits);
            if ((bits & 0x7f80_0000) == 0x7f80_0000)
                throw new IllegalArgumentException("Float32 PCM sample must be finite");
            decoded[sampleOffset + i] = sample;
        }
    }
}
