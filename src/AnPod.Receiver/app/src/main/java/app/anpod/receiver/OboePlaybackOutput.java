package app.anpod.receiver;

import android.media.AudioTrack;
import com.anpod.server.Pcm16FrameStream;
import java.io.IOException;

/** Float32 Oboe stream adapter. The Java worker is the sole ring producer. */
final class OboePlaybackOutput implements PlaybackOutput {
    private static final int INFO_ACTUAL_RATE = 1;
    private static final int INFO_ACTUAL_CHANNELS = 2;
    private static final int INFO_ACTUAL_FORMAT = 3;
    private static final int INFO_FRAMES_PER_BURST = 4;
    private static final int INFO_STREAM_BUFFER = 5;
    private static final int INFO_STREAM_CAPACITY = 6;
    private static final int INFO_RING_CAPACITY = 7;
    private static final int INFO_TOTAL_BUDGET = 11;

    private final long handle;
    private final int sampleRate;
    private final int channels;
    private final int[] info = new int[OboeNativeBridge.INFO_LENGTH];
    private final int[] metrics = new int[OboeNativeBridge.METRICS_LENGTH];
    private final long[] timestamp = new long[OboeNativeBridge.TIMESTAMP_LENGTH];
    private final float[] pcmScratch;
    private boolean closed;
    private int outputEpoch = 1;
    private final int[] previousCounters = new int[4];
    private final long[] extendedCounters = new long[4];

    static OboePlaybackOutput open(int sampleRate, int channels, int totalBudgetFrames, int maxWriteFrames)
            throws IOException {
        OboeNativeBridge.load();
        int[] info = new int[OboeNativeBridge.INFO_LENGTH];
        long handle = OboeNativeBridge.open(sampleRate, channels, totalBudgetFrames,
                sampleRate * 5 / 1000, info);
        if (handle == 0) throw failure("Oboe open", info[0]);
        OboePlaybackOutput output = new OboePlaybackOutput(handle, sampleRate, channels, maxWriteFrames, info);
        try {
            output.validateInfo();
            return output;
        } catch (IOException error) {
            try { output.close(); } catch (IOException closeError) { error.addSuppressed(closeError); }
            if (error instanceof OboeOutputException && !output.closed)
                ReceiverService.retainDeferredOboeClose(output);
            throw error;
        }
    }

    private OboePlaybackOutput(long handle, int sampleRate, int channels, int maxWriteFrames, int[] openedInfo) {
        this.handle = handle;
        this.sampleRate = sampleRate;
        this.channels = channels;
        System.arraycopy(openedInfo, 0, info, 0, info.length);
        this.pcmScratch = new float[Math.max(1, maxWriteFrames) * channels];
        java.util.Arrays.fill(timestamp, -1L);
        resetMetricsEpoch();
    }

    private void validateInfo() throws IOException {
        if (info[0] != 0) throw failure("Oboe open", info[0]);
        if (info[INFO_ACTUAL_RATE] != sampleRate || info[INFO_ACTUAL_CHANNELS] != channels
                || info[INFO_ACTUAL_FORMAT] != 1)
            throw new OboeOutputException("Oboe opened a different rate, channel count, or format");
        int burst = info[INFO_FRAMES_PER_BURST];
        int ring = info[INFO_RING_CAPACITY];
        if (burst <= 0 || ring < burst || ring + info[INFO_STREAM_BUFFER] > info[INFO_TOTAL_BUDGET])
            throw new OboeOutputException("Oboe output buffer budget cannot hold one callback burst");
    }

    private static OboeOutputException failure(String operation, int code) {
        String detail;
        try { detail = OboeNativeBridge.lastErrorMessage(); }
        catch (RuntimeException error) { detail = ""; }
        return new OboeOutputException(operation + " failed (" + code + ")"
                + (detail == null || detail.isEmpty() ? "" : ": " + detail));
    }

    private void ensureOpen() throws IOException {
        if (closed) throw new OboeOutputException("Oboe output is closed");
    }

    @Override public String engine() { return "oboeExperimental"; }
    @Override public AudioTrack audioTrack() { return null; }
    @Override public synchronized int sampleRate() { return info[INFO_ACTUAL_RATE]; }
    @Override public synchronized int channels() { return info[INFO_ACTUAL_CHANNELS]; }
    @Override public int formatCode() { return 3; }
    @Override public synchronized int capacityFrames() { return info[INFO_STREAM_CAPACITY]; }
    @Override public synchronized int effectiveBufferFrames() { return info[INFO_STREAM_BUFFER]; }
    @Override public int startThresholdFrames() { return -1; }
    @Override public synchronized int framesPerBurst() { return info[INFO_FRAMES_PER_BURST]; }
    @Override public synchronized int ringCapacityFrames() { return info[INFO_RING_CAPACITY]; }
    @Override public synchronized int underrunCount() { return -1; }
    @Override public synchronized int discontinuityCount() throws IOException { return oboeXrunCount(); }
    @Override public synchronized int oboeXrunCount() throws IOException {
        readMetrics();
        return metrics[8];
    }
    @Override public synchronized long playbackHeadPosition() throws IOException {
        readMetrics();
        return metrics[5] < 0 ? -1L : Integer.toUnsignedLong(metrics[7]);
    }
    @Override public synchronized long pendingFrames(long acceptedFrames, long playedFrames) throws IOException {
        readMetrics();
        if (metrics[5] < 0) return -1;
        long streamPending = metrics[5];
        return Math.max(0, (long) metrics[4] + streamPending);
    }

    @Override public synchronized int writePcm16(byte[] data, int offsetBytes, int lengthBytes) throws IOException {
        ensureOpen();
        int frameBytes = channels * 2;
        if ((offsetBytes % frameBytes) != 0 || (lengthBytes % frameBytes) != 0)
            throw new IOException("PCM16 output block is not frame aligned");
        int requestedFrames = Math.min(lengthBytes / frameBytes, pcmScratch.length / channels);
        if (requestedFrames <= 0) return 0;
        int samples = requestedFrames * channels;
        int sourceOffset = offsetBytes;
        for (int i = 0; i < samples; i++) {
            int value = (short) ((data[sourceOffset] & 0xff) | (data[sourceOffset + 1] << 8));
            pcmScratch[i] = value / 32768f;
            sourceOffset += 2;
        }
        int accepted = OboeNativeBridge.writeFloat(handle, pcmScratch, 0, requestedFrames);
        if (accepted < 0) throw failure("Oboe PCM16 write", accepted);
        return accepted * frameBytes;
    }

    @Override public synchronized int writeFloatFrames(float[] samples, int offsetFrames, int frameCount,
            byte[] packedScratch) throws IOException {
        ensureOpen();
        int accepted = OboeNativeBridge.writeFloat(handle, samples, offsetFrames, frameCount);
        if (accepted < 0) throw failure("Oboe Float32 write", accepted);
        return accepted;
    }

    @Override public synchronized void play() throws IOException { start(); }

    private void start() throws IOException {
        ensureOpen();
        int result = OboeNativeBridge.start(handle, info);
        if (result != 0) throw failure("Oboe start", result);
        validateInfo();
        outputEpoch++;
        resetMetricsEpoch();
    }

    @Override public synchronized void requestPause() throws IOException {
        ensureOpen();
        int result = OboeNativeBridge.requestPause(handle);
        if (result != 0) throw failure("Oboe pause request", result);
    }

    @Override public synchronized void pauseAndFlush() throws IOException {
        ensureOpen();
        int result = OboeNativeBridge.pauseAndFlush(handle);
        if (result != 0) throw failure("Oboe pause/flush", result);
        resetMetricsEpoch();
    }

    @Override public synchronized void setVolume(float volume) throws IOException {
        ensureOpen();
        int result = OboeNativeBridge.setVolume(handle, volume);
        if (result != 0) throw failure("Oboe volume change", result);
    }

    @Override public synchronized int[] info() { return info.clone(); }

    @Override public synchronized int[] metrics() throws IOException {
        readMetrics();
        return metrics.clone();
    }

    @Override public synchronized long[] timestamp() throws IOException {
        readMetrics();
        return timestamp.clone();
    }

    private synchronized void readMetrics() throws IOException {
        ensureOpen();
        int result = OboeNativeBridge.getMetrics(handle, metrics, timestamp);
        if (result != 0) throw failure("Oboe metrics", result);
        if (metrics[10] != 0) throw new OboeOutputException("Oboe callback error: " + metrics[10]);
        for (int i = 0; i < 4; i++) {
            if (metrics[i] != previousCounters[i]) extendedCounters[i] += unsignedDelta(metrics[i], previousCounters[i]);
            previousCounters[i] = metrics[i];
        }
    }

    private void resetMetricsEpoch() {
        java.util.Arrays.fill(previousCounters, 0);
        java.util.Arrays.fill(extendedCounters, 0L);
        java.util.Arrays.fill(metrics, 0);
        java.util.Arrays.fill(timestamp, -1L);
    }

    private static long unsignedDelta(int current, int previous) {
        return Integer.toUnsignedLong(current - previous);
    }

    synchronized long extendedCounter(int index) {
        if (index < 0 || index >= extendedCounters.length) throw new IllegalArgumentException("index");
        return extendedCounters[index];
    }

    synchronized int outputEpoch() { return outputEpoch; }

    @Override public synchronized void close() throws IOException {
        if (closed) return;
        int result = OboeNativeBridge.close(handle);
        if (result != 0) throw failure("Oboe close", result);
        closed = true;
    }
}
