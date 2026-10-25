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
    public void floatSamplesPreserveIndependentChannelPeaks() {
        PcmWaveformTap tap = new PcmWaveformTap();
        float[] frames = new float[480 * 2];
        for (int frame = 0; frame < 480; frame++) {
            frames[frame * 2] = 0.375f;
            frames[frame * 2 + 1] = -0.125f;
        }
        long now = 0;
        for (int chunk = 0; chunk < 10; chunk++) {
            tap.accept(frames, 0, 480, 2, now);
            now += 10_000_000L;
        }

        PcmWaveformTap.Waveform waveform = tap.waveform(now);
        assertTrue(max(waveform.left) > 0.35f);
        assertTrue(max(waveform.left) < 0.4f);
        assertTrue(max(waveform.right) > 0.1f);
        assertTrue(max(waveform.right) < 0.15f);
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
    public void digitalSilenceMovesPeakIntoHistoryAndTimesOut() {
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
        assertEquals(0f, waveform.left[0], 0.0001f);
        assertTrue(max(waveform.left) > 0.2f);

        PcmWaveformTap.Waveform timedOut = tap.waveform(now + 401_000_000L);
        assertTrue(timedOut.silent);
        assertEquals(0f, max(timedOut.left), 0.0001f);
    }

    @Test
    public void productionCurveMovesItsPeakOutwardAndSeparatesChannels() {
        float[] history = new float[PcmWaveformTap.POINTS];
        java.util.Arrays.fill(history, 0.5f);
        float[] atStart = AodWaveformGeometry.centeredCurve(history, 20f, 0L, false);
        float[] afterFiftyMs = AodWaveformGeometry.centeredCurve(history, 20f, 50_000_000L, false);
        int startPeak = peakDistanceFromCenter(atStart);
        int movingPeak = peakDistanceFromCenter(afterFiftyMs);
        int center = atStart.length / 2;
        assertEquals(0, startPeak);
        assertTrue(Math.abs(atStart[center] - atStart[center - 1]) < 0.5f);
        assertTrue(movingPeak > startPeak);

        float[] right = AodWaveformGeometry.centeredCurve(history, 20f, 50_000_000L, true);
        assertTrue(!java.util.Arrays.equals(afterFiftyMs, right));
        for (int index = 0; index < afterFiftyMs.length; index++) {
            assertEquals(afterFiftyMs[index], afterFiftyMs[afterFiftyMs.length - 1 - index], 0.0001f);
            assertEquals(right[index], right[right.length - 1 - index], 0.0001f);
        }
    }

    @Test
    public void waveformLayoutKeepsGlowClearOfClockAndFooter() {
        AodWaveformGeometry.Layout layout = AodWaveformGeometry.layout(560f, 760f, 4f, 4f);
        float requiredGap = layout.maximumAmplitude + AodWaveformGeometry.GLOW_STROKE_WIDTH / 2f + 4f + 4f;
        assertTrue(requiredGap <= layout.centerY - 560f);
        assertTrue(requiredGap <= 760f - layout.centerY);

        AodWaveformGeometry.Layout playback = AodWaveformGeometry.layout(480f, 880f, 8f, 8f);
        // Same layout as 1.8.1 (usable half height 184 px) but this batch keeps 2/3 of that maximum.
        float previousMaximum = (184f - AodWaveformGeometry.GLOW_STROKE_WIDTH / 2f) * 0.90f;
        assertEquals(previousMaximum * 2f / 3f, playback.maximumAmplitude, 0.05f);
        assertEquals(680f, playback.centerY, 0.0001f);
    }

    @Test
    public void waveformReserveKeepsShortScreenLayoutUsable() {
        for (float screenHeight : new float[] {240f, 320f, 400f}) {
            float footerTop = screenHeight * 0.88f;
            float baseClockHeight = Math.max(96f, Math.min(184f, screenHeight * 0.40f));
            float clockBottom = AodWaveformGeometry.clockBottom(
                    screenHeight, baseClockHeight, footerTop, 9f, 4f, 64f);
            AodWaveformGeometry.Layout layout = AodWaveformGeometry.layout(clockBottom, footerTop, 9f, 4f);
            assertTrue("screen height " + screenHeight, layout.regionHeight > 0f);
        }

        float screenHeight = 200f;
        float footerTop = screenHeight * 0.88f;
        float clockBottom = AodWaveformGeometry.clockBottom(screenHeight, 96f, footerTop, 9f, 4f, 64f);
        AodWaveformGeometry.Layout bold = AodWaveformGeometry.layout(clockBottom, footerTop, 9f, 4f);
        float boldPeakPlusGlowAndClearance = bold.maximumAmplitude +
                AodWaveformGeometry.GLOW_STROKE_WIDTH + 4f;
        assertTrue(boldPeakPlusGlowAndClearance <= bold.regionHeight / 2f);
    }

    @Test
    public void carrierDriftStaysSlowEnoughPerFrame() {
        float[] history = new float[PcmWaveformTap.POINTS];
        java.util.Arrays.fill(history, 0.5f);
        float[] first = AodWaveformGeometry.centeredCurve(history, 100f, 0L, false);
        float[] nextFrame = AodWaveformGeometry.centeredCurve(history, 100f, 33_000_000L, false);
        float largestStep = 0f;
        for (int index = 0; index < first.length; index++) {
            largestStep = Math.max(largestStep, Math.abs(first[index] - nextFrame[index]));
        }
        // 15 rad/s is at most ~0.5 rad per 33 ms frame, well under 0.6 of the peak amplitude.
        assertTrue(largestStep > 1f);
        assertTrue(largestStep < 60f);
    }

    @Test
    public void identicalChannelsRenderAsOppositeMirrors() {
        float[] history = new float[PcmWaveformTap.POINTS];
        java.util.Arrays.fill(history, 0.6f);
        float[] left = AodWaveformGeometry.centeredCurve(history, 40f, 25_000_000L, false);
        float[] right = AodWaveformGeometry.centeredCurve(history, 40f, 25_000_000L, true);
        for (int index = 0; index < left.length; index++) {
            // Same PCM on both sides must still start in opposite directions.
            assertEquals(-left[index], right[index], 0.0001f);
        }
        assertTrue(max(left) > 1f);
        assertTrue(min(right) < -1f);
    }

    @Test
    public void channelsShareTheBaselineAndOneSidedAudioStaysOneSided() {
        float[] loud = new float[PcmWaveformTap.POINTS];
        float[] silent = new float[PcmWaveformTap.POINTS];
        java.util.Arrays.fill(loud, 0.8f);
        float[] left = AodWaveformGeometry.centeredCurve(loud, 40f, 0L, false);
        float[] right = AodWaveformGeometry.centeredCurve(silent, 40f, 0L, true);
        // Both traces stay on the shared baseline, so a channel can swing either side of it.
        assertTrue(max(left) > 20f);
        assertTrue(min(left) < -10f);
        assertEquals(0f, max(right), 0.0001f);
        assertEquals(0f, min(right), 0.0001f);
        assertTrue(!java.util.Arrays.equals(left, right));
    }

    @Test
    public void quietPcmIsVisibleWhileSilenceStaysFlatAndLouderSignalsRemainDistinct() {
        float[] history = new float[PcmWaveformTap.POINTS];
        assertEquals(0f, max(AodWaveformGeometry.centeredCurve(history, 20f, 0L, false)), 0f);
        java.util.Arrays.fill(history, 0.01f);
        float quiet = max(AodWaveformGeometry.centeredCurve(history, 20f, 0L, false));
        assertTrue(quiet >= 9f);
        java.util.Arrays.fill(history, 0.1f);
        float medium = max(AodWaveformGeometry.centeredCurve(history, 20f, 0L, false));
        java.util.Arrays.fill(history, 1f);
        float loud = max(AodWaveformGeometry.centeredCurve(history, 20f, 0L, false));
        assertTrue(quiet < medium);
        assertTrue(medium < loud);
        assertTrue(loud <= 20f);
    }

    private static int peakDistanceFromCenter(float[] values) {
        int center = values.length / 2;
        int peak = center;
        for (int index = 0; index < values.length; index++) {
            if (values[index] > values[peak]) peak = index;
        }
        return Math.abs(peak - center);
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

    private static float min(float[] values) {
        float min = 0f;
        for (float value : values) min = Math.min(min, value);
        return min;
    }
}
