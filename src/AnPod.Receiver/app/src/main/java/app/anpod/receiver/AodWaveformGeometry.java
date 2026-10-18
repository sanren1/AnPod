package app.anpod.receiver;

/** Small platform-free geometry shared by the AOD renderer and its offline checks. */
final class AodWaveformGeometry {
    static final float STROKE_WIDTH = 3.6f;
    static final float GLOW_STROKE_WIDTH = 12f;
    static final float EDGE_FADE_FRACTION = 0.16f;
    /** Share of the usable gap between clock and metrics kept as peak amplitude. */
    private static final float AMPLITUDE_FILL = 0.60f;
    private static final float CARRIER_STEP_RADIANS = 0.34f;
    /** Phase travel speed; halved from 30.0 so the curve drifts calmly instead of racing. */
    private static final double CARRIER_RADIANS_PER_SECOND = 15.0;
    private static final double AMPLITUDE_LOG_GAIN = 10_000.0;
    private static final double AMPLITUDE_LOG_RANGE = Math.log1p(AMPLITUDE_LOG_GAIN);

    private AodWaveformGeometry() {}

    static Layout layout(float clockBottom, float footerTop,
                         float maximumDrift, float clearance) {
        float center = (clockBottom + footerTop) / 2f;
        float half = Math.max(0f, Math.min(center - clockBottom, footerTop - center)
                - maximumDrift - clearance);
        float regionHeight = half * 2f;
        // 2/3 of the previous 0.90 fill, keeping the glow clear of clock and metrics.
        float maximumAmplitude = Math.max(0f, half - GLOW_STROKE_WIDTH / 2f) * AMPLITUDE_FILL;
        return new Layout(center, regionHeight, maximumAmplitude);
    }

    /**
     * Returns one channel's curve: newest bucket at center, older PCM history toward both edges.
     * Values are signed around the shared baseline. The right channel is inverted, so identical
     * left/right audio always renders as one trace above the baseline and its mirror below.
     */
    static float[] centeredCurve(float[] history, float maximumAmplitude, long sampledAtNanos,
                                 boolean rightChannel) {
        if (history == null || history.length < 2) return new float[0];
        int center = history.length - 1;
        float[] offsets = new float[center * 2 + 1];
        double phasePeriodNanos = 2.0 * Math.PI / CARRIER_RADIANS_PER_SECOND * 1_000_000_000.0;
        float phase = (float) ((sampledAtNanos % phasePeriodNanos) / phasePeriodNanos * 2.0 * Math.PI);
        for (int point = 0; point < offsets.length; point++) {
            int age = Math.abs(point - center);
            float peak = Math.max(0f, Math.min(1f, history[age]));
            float level = (float) (Math.log1p(peak * AMPLITUDE_LOG_GAIN) / AMPLITUDE_LOG_RANGE);
            float distance = (float) age / center;
            float taper = (float) (0.5 + 0.5 * Math.cos(Math.PI * distance));
            taper *= taper;
            float radialDistance = (float) (Math.sqrt(age * age + 16.0) - 4.0);
            float carrier = (float) Math.cos(radialDistance * CARRIER_STEP_RADIANS - phase);
            float value = level * taper * carrier * maximumAmplitude;
            offsets[point] = rightChannel ? -value : value;
        }
        return offsets;
    }

    static final class Layout {
        final float centerY;
        final float regionHeight;
        final float maximumAmplitude;

        Layout(float centerY, float regionHeight, float maximumAmplitude) {
            this.centerY = centerY;
            this.regionHeight = regionHeight;
            this.maximumAmplitude = maximumAmplitude;
        }
    }
}
