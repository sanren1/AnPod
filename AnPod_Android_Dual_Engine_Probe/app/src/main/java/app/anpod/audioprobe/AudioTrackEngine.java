package app.anpod.audioprobe;

import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioTimestamp;
import android.media.AudioTrack;
import android.os.Build;
import android.os.Process;
import org.json.JSONObject;
import java.util.concurrent.atomic.AtomicLong;

/** Dedicated writer thread using the identical PCM16 clip passed to the native Oboe callback. */
final class AudioTrackEngine {
    private static final int RATE = 48000;
    private static final int CHANNELS = 2;
    private static final int FRAME_BYTES = CHANNELS * 2;
    private volatile boolean stopped = true;
    private volatile AudioTrack track;
    private Thread writer;
    private final AtomicLong writtenFrames = new AtomicLong();
    private final AtomicLong writes = new AtomicLong();
    private final AtomicLong longestWriteNs = new AtomicLong();
    private int requestedBufferFrames;
    private int actualStartThreshold = -1;
    private int burstHint = -1;

    String start(short[] clip, int bursts, int framesPerBufferHint) {
        stop();
        if (clip == null || clip.length % CHANNELS != 0 || clip.length == 0) return "Invalid PCM clip";
        burstHint = framesPerBufferHint > 0 ? framesPerBufferHint : 192;
        requestedBufferFrames = Math.max(1, burstHint * bursts);
        actualStartThreshold = -1;
        writtenFrames.set(0);
        writes.set(0);
        longestWriteNs.set(0);
        try {
            int minBytes = AudioTrack.getMinBufferSize(RATE,
                    AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT);
            if (minBytes <= 0) return "AudioTrack.getMinBufferSize failed: " + minBytes;
            int capacityBytes = Math.max(minBytes * 2, requestedBufferFrames * FRAME_BYTES);
            AudioTrack t = new AudioTrack.Builder()
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                    .setAudioFormat(new AudioFormat.Builder()
                            .setSampleRate(RATE)
                            .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                            .setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .setBufferSizeInBytes(capacityBytes)
                    .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
                    .build();
            if (t.getState() != AudioTrack.STATE_INITIALIZED) {
                t.release();
                return "AudioTrack did not initialize";
            }
            t.setBufferSizeInFrames(requestedBufferFrames); // may round or refuse: report actual
            if (Build.VERSION.SDK_INT >= 31) {
                int target = Math.max(1, burstHint);
                int reply = t.setStartThresholdInFrames(target);
                if (reply > 0) actualStartThreshold = reply;
            }
            stopped = false;
            track = t;
            t.play();
            final int blockFrames = Math.max(1, burstHint);
            writer = new Thread(() -> writeLoop(t, clip, blockFrames), "AnPod AudioTrack writer");
            writer.start();
            return "";
        } catch (Exception err) {
            stopped = true;
            if (track != null) { try { track.release(); } catch (Exception ignored) {} }
            track = null;
            return err.toString();
        }
    }

    private void writeLoop(AudioTrack local, short[] clip, int framesPerWrite) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
        short[] chunk = new short[framesPerWrite * CHANNELS];
        int offset = 0;
        try {
            while (!stopped) {
                int copied = 0;
                while (copied < chunk.length) {
                    int n = Math.min(chunk.length - copied, clip.length - offset);
                    System.arraycopy(clip, offset, chunk, copied, n);
                    copied += n;
                    offset = (offset + n == clip.length) ? 0 : offset + n;
                }
                int pos = 0;
                while (pos < chunk.length && !stopped) {
                    long begin = System.nanoTime();
                    int n = local.write(chunk, pos, chunk.length - pos, AudioTrack.WRITE_BLOCKING);
                    long duration = System.nanoTime() - begin;
                    longestWriteNs.accumulateAndGet(duration, Math::max);
                    writes.incrementAndGet();
                    if (n <= 0) { stopped = true; break; }
                    pos += n;
                    writtenFrames.addAndGet(n / CHANNELS);
                }
            }
        } catch (Exception ignored) {
            stopped = true;
        }
    }

    void stop() {
        stopped = true;
        AudioTrack t = track;
        if (t != null) {
            try { t.pause(); } catch (Exception ignored) {}
            try { t.flush(); } catch (Exception ignored) {}
        }
        Thread w = writer;
        if (w != null) {
            try { w.join(800); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
            if (w.isAlive()) {
                // Do not release the track while its blocking writer might still access it.
                // The process will close the stream; a new test should only start once it stops.
                throw new IllegalStateException("AudioTrack writer did not exit within 800ms");
            }
        }
        writer = null;
        track = null;
        if (t != null) { try { t.release(); } catch (Exception ignored) {} }
    }

    JSONObject snapshot() {
        JSONObject j = new JSONObject();
        AudioTrack t = track;
        try {
            j.put("running", t != null && !stopped);
            if (t == null) return j;
            int rate = t.getSampleRate();
            j.put("engine", "AudioTrack");
            j.put("api", "Java AudioTrack");
            j.put("sharing", "system");
            j.put("perf", t.getPerformanceMode() == AudioTrack.PERFORMANCE_MODE_LOW_LATENCY
                    ? "LowLatency" : "Other");
            j.put("mmap", "not exposed");
            j.put("sampleRate", rate);
            j.put("channels", t.getChannelCount());
            j.put("framesPerBurst", burstHint);
            j.put("burstIsHint", true);
            j.put("bufferFrames", t.getBufferSizeInFrames());
            j.put("capacityFrames", t.getBufferCapacityInFrames());
            j.put("startThresholdFrames", actualStartThreshold);
            j.put("xrun", t.getUnderrunCount());
            long written = writtenFrames.get();
            j.put("framesWritten", written);
            j.put("callbackCount", writes.get());
            j.put("callbackGapMs", -1);
            j.put("maxCallbackGapMs", longestWriteNs.get() / 1e6);
            AudioTimestamp ts = new AudioTimestamp();
            double latencyMs = -1;
            if (t.getTimestamp(ts) && rate > 0 && written >= ts.framePosition) {
                latencyMs = ((ts.nanoTime - System.nanoTime()) +
                        (written - ts.framePosition) * (1e9 / rate)) / 1e6;
                if (latencyMs < 0 || latencyMs > 1000) latencyMs = -1;
            }
            j.put("latencyMs", latencyMs);
        } catch (Exception e) {
            try { j.put("snapshotError", e.toString()); } catch (Exception ignored) {}
        }
        return j;
    }
}
