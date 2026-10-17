package app.anpod.receiver;

/**
 * Non-blocking PCM16 envelope tap for the AOD waveform.
 *
 * <p>The audio thread only feeds samples; the UI thread copies the latest points. Work per write is
 * bounded by sampling at most 128 frames per channel, and the tap never waits for the UI.
 */
final class PcmWaveformTap {
    static final int POINTS = 48;
    private static final long BUCKET_NANOS = 10_000_000L;
    private static final float DECAY = 0.75f;
    private static final long SILENCE_RESET_NANOS = 400_000_000L;

    private final float[] left = new float[POINTS];
    private final float[] right = new float[POINTS];
    private int next;
    private long bucketStartNanos = -1;
    private long lastAcceptNanos = -1;
    private double leftSum;
    private double rightSum;
    private int samples;
    private float lastLeft;
    private float lastRight;

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
            leftSum += square(leftSample);
            rightSum += square(rightSample);
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
        leftSum = 0;
        rightSum = 0;
        samples = 0;
        lastLeft = 0f;
        lastRight = 0f;
    }

    synchronized Waveform waveform(long nowNanos) {
        boolean silent = lastAcceptNanos < 0 || nowNanos - lastAcceptNanos > SILENCE_RESET_NANOS;
        float[] leftCopy = new float[POINTS];
        float[] rightCopy = new float[POINTS];
        if (!silent) {
            for (int index = 0; index < POINTS; index++) {
                leftCopy[index] = left[(next + index) % POINTS];
                rightCopy[index] = right[(next + index) % POINTS];
            }
        }
        return new Waveform(leftCopy, rightCopy);
    }

    private void push(long nowNanos) {
        float leftLevel = (float) Math.sqrt(leftSum / Math.max(1, samples));
        float rightLevel = (float) Math.sqrt(rightSum / Math.max(1, samples));
        lastLeft = Math.max(leftLevel, lastLeft * DECAY);
        lastRight = Math.max(rightLevel, lastRight * DECAY);
        left[next] = Math.min(1f, lastLeft);
        right[next] = Math.min(1f, lastRight);
        next = (next + 1) % POINTS;
        leftSum = 0;
        rightSum = 0;
        samples = 0;
        bucketStartNanos = nowNanos;
    }

    private static short sample(byte[] pcm, int index) {
        return (short) ((pcm[index] & 0xff) | (pcm[index + 1] << 8));
    }

    private static double square(short sample) {
        double normalized = sample / 32768.0;
        return normalized * normalized;
    }

    static final class Waveform {
        final float[] left;
        final float[] right;

        Waveform(float[] left, float[] right) {
            this.left = left;
            this.right = right;
        }
    }
}
