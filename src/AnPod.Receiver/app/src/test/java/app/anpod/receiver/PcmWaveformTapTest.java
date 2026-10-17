package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class PcmWaveformTapTest {
    private static final int SAMPLE_RATE = 48_000;

    @Test
    public void sineProducesEnvelopeOnBothChannels() {
        PcmWaveformTap tap = new PcmWaveformTap();
        long now = 0;

        for (int chunk = 0; chunk < 10; chunk++) {
            tap.accept(sine(480, 0.5, 0.5, chunk * 480), 0, 480 * 4, 4, now);
            now += 10_000_000L;
        }

        PcmWaveformTap.Waveform waveform = tap.waveform(now);
        assertTrue(max(waveform.left) > 0.2f);
        assertTrue(max(waveform.right) > 0.2f);
        assertTrue(max(waveform.left) <= 0.75f);
    }

    @Test
    public void channelsAreIndependent() {
        PcmWaveformTap tap = new PcmWaveformTap();
        long now = 0;

        for (int chunk = 0; chunk < 10; chunk++) {
            tap.accept(sine(480, 0.5, 0.0, chunk * 480), 0, 480 * 4, 4, now);
            now += 10_000_000L;
        }

        PcmWaveformTap.Waveform waveform = tap.waveform(now);
        assertTrue(max(waveform.left) > max(waveform.right) * 3f);
    }

    @Test
    public void silenceCollapsesTheWaveform() {
        PcmWaveformTap tap = new PcmWaveformTap();
        long now = 0;
        for (int chunk = 0; chunk < 10; chunk++) {
            tap.accept(sine(480, 0.5, 0.5, chunk * 480), 0, 480 * 4, 4, now);
            now += 10_000_000L;
        }

        PcmWaveformTap.Waveform waveform = tap.waveform(now + 500_000_000L);
        assertEquals(0f, max(waveform.left), 0.0001f);
        assertEquals(0f, max(waveform.right), 0.0001f);
    }

    @Test
    public void digitalSilenceDecaysThroughBuckets() {
        PcmWaveformTap tap = new PcmWaveformTap();
        long now = 0;
        for (int chunk = 0; chunk < 10; chunk++) {
            tap.accept(sine(480, 0.5, 0.5, chunk * 480), 0, 480 * 4, 4, now);
            now += 10_000_000L;
        }
        for (int chunk = 0; chunk < 20; chunk++) {
            tap.accept(new byte[480 * 4], 0, 480 * 4, 4, now);
            now += 10_000_000L;
        }

        PcmWaveformTap.Waveform waveform = tap.waveform(now);
        float last = waveform.left[PcmWaveformTap.POINTS - 1];
        assertTrue(last < 0.05f);
    }

    private static byte[] sine(int frames, double leftAmplitude, double rightAmplitude, int frameOffset) {
        byte[] pcm = new byte[frames * 4];
        for (int frame = 0; frame < frames; frame++) {
            double phase = 2 * Math.PI * 1000 * (frameOffset + frame) / SAMPLE_RATE;
            short left = (short) Math.round(Math.sin(phase) * leftAmplitude * 32767);
            short right = (short) Math.round(Math.sin(phase) * rightAmplitude * 32767);
            pcm[frame * 4] = (byte) (left & 0xff);
            pcm[frame * 4 + 1] = (byte) (left >> 8);
            pcm[frame * 4 + 2] = (byte) (right & 0xff);
            pcm[frame * 4 + 3] = (byte) (right >> 8);
        }
        return pcm;
    }

    private static float max(float[] values) {
        float max = 0f;
        for (float value : values) max = Math.max(max, value);
        return max;
    }
}
