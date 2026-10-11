package com.anpod.server;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
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

        try {
            Pcm16FrameStream truncated = new Pcm16FrameStream();
            ByteArrayInputStream truncatedInput = new ByteArrayInputStream(new byte[] {1, 2, 3});
            while (truncated.readFrames(truncatedInput, buffer, buffer.length) >= 0) { }
            throw new AssertionError("incomplete final frame was silently discarded");
        } catch (IOException expectedFailure) {
            if (!expectedFailure.getMessage().contains("incomplete stereo frame")) throw expectedFailure;
        }
        System.out.println("Android PCM framing and mode checks passed.");
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
}
