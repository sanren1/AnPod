package app.anpod.receiver;

/**
 * Non-blocking PCM16 peak-history tap for the AOD waveform.
 *
 * <p>The audio thread only feeds samples; the UI thread copies the latest points. Work per write is
 * bounded by sampling at most 128 frames per channel, and the tap never waits for the UI.
 */
final class PcmWaveformTap {
    static final int POINTS = 48;
    private static final long BUCKET_NANOS = 10_000_000L;
    private static final long SILENCE_RESET_NANOS = 400_000_000L;

    private final float[] left = new float[POINTS];
    private final float[] right = new float[POINTS];
    private int next;
    private long bucketStartNanos = -1;
    private long lastAcceptNanos = -1;
    private float leftPeak;
    private float rightPeak;
    private int samples;

    synchronized void accept(byte[] pcm, int offset, int length, int frameBytes, long nowNanos) {
        if (pcm == null || frameBytes < 2 || length < frameBytes) return;
        int frames = length / frameBytes;
        int channels = frameBytes / 2;
        int stride = Math.max(1, frames / 128);
        for (int frame = 0; frame < frames; frame += stride) {
            int base = offset + frame * frameBytes;
            if (base + frameBytes > offset + length) break;
            short leftSample = sample(pcm, base);
            short rightSample = channels > 1 ? sample(pcm, base + 2) : leftSample;
            leftPeak = Math.max(leftPeak, Math.abs(leftSample) / 32768f);
            rightPeak = Math.max(rightPeak, Math.abs(rightSample) / 32768f);
            samples++;
        }
        lastAcceptNanos = nowNanos;
        if (bucketStartNanos < 0) bucketStartNanos = nowNanos;
        if (nowNanos - bucketStartNanos >= BUCKET_NANOS) push(nowNanos);
    }

    synchronized void reset() {
        java.util.Arrays.fill(left, 0f);
        java.util.Arrays.fill(right, 0f);
        next = 0;
        bucketStartNanos = -1;
        lastAcceptNanos = -1;
        leftPeak = 0;
        rightPeak = 0;
        samples = 0;
    }

    synchronized Waveform waveform(long nowNanos) {
        boolean silent = lastAcceptNanos < 0 || nowNanos - lastAcceptNanos > SILENCE_RESET_NANOS;
        float[] leftCopy = new float[POINTS];
        float[] rightCopy = new float[POINTS];
        if (!silent) {
            for (int age = 0; age < POINTS; age++) {
                int sample = (next - 1 - age + POINTS) % POINTS;
                leftCopy[age] = left[sample];
                rightCopy[age] = right[sample];
            }
        }
        return new Waveform(leftCopy, rightCopy, nowNanos, silent);
    }

    private void push(long nowNanos) {
        left[next] = Math.min(1f, leftPeak);
        right[next] = Math.min(1f, rightPeak);
        next = (next + 1) % POINTS;
        leftPeak = 0;
        rightPeak = 0;
        samples = 0;
        bucketStartNanos = nowNanos;
    }

    private static short sample(byte[] pcm, int index) {
        return (short) ((pcm[index] & 0xff) | (pcm[index + 1] << 8));
    }

    static final class Waveform {
        final float[] left;
        final float[] right;
        final long sampledAtNanos;
        final boolean silent;

        Waveform(float[] left, float[] right, long sampledAtNanos, boolean silent) {
            this.left = left;
            this.right = right;
            this.sampledAtNanos = sampledAtNanos;
            this.silent = silent;
        }
    }
}
