package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioTrack;
import android.content.Intent;
import android.net.LocalSocket;
import android.net.LocalSocketAddress;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.Process;

import com.anpod.server.Pcm16FrameStream;

import java.io.DataInputStream;
import java.io.IOException;
import java.io.InputStream;

public final class ReceiverService extends Service {
    private static final String CHANNEL_ID = "receiver-management";
    private static final int NOTIFICATION_ID = 1;
    private static final long MAX_RUN_MS = 120_000L;

    static volatile boolean foregroundReady;
    static volatile long startedAtElapsedMs;
    static volatile String startFailure = "";
    static volatile boolean audioActive;
    static volatile String actualMode = "";

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable timeout;
    private volatile Thread audioThread;
    private volatile LocalSocket audioSocket;
    private AudioTrack audioTrack;
    private AudioManager audioManager;
    private AudioFocusRequest focusRequest;
    private AudioManager.OnAudioFocusChangeListener focusListener;

    @Override
    public void onCreate() {
        super.onCreate();
        foregroundReady = false;
        audioActive = false;
        actualMode = "";
        startedAtElapsedMs = 0;
        startFailure = "";
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        boolean audio = intent != null && BootstrapActivity.ACTION_START_AUDIO.equals(intent.getAction());
        try {
            Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                    ? new Notification.Builder(this, CHANNEL_ID)
                    : new Notification.Builder(this);
            Notification notification = builder
                    .setSmallIcon(android.R.drawable.stat_sys_headset)
                    .setContentTitle("AnPod Receiver")
                    .setContentText(audio ? "AnPod 音频连接正在运行。" : "管理检查进行中；当前不播放音频。")
                    .setOngoing(true)
                    .build();
            if (Build.VERSION.SDK_INT >= 34) {
                int type = audio
                        ? android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                        : android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE;
                startForeground(NOTIFICATION_ID, notification, type);
            } else {
                startForeground(NOTIFICATION_ID, notification);
            }
            startedAtElapsedMs = android.os.SystemClock.elapsedRealtime();
            foregroundReady = true;
        } catch (RuntimeException error) {
            recordStartFailure(error.getClass().getSimpleName());
            stopSelf(startId);
            return START_NOT_STICKY;
        }

        if (audio) {
            if (timeout != null) handler.removeCallbacks(timeout);
            startAudio(intent.getStringExtra("socketName"), intent.getStringExtra("connectCode"),
                    intent.getStringExtra("playbackMode"));
            return START_NOT_STICKY;
        }

        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = () -> stopSelf(startId);
        handler.postDelayed(timeout, MAX_RUN_MS);
        return START_NOT_STICKY;
    }

    @Override
    public void onTimeout(int startId) {
        foregroundReady = false;
        stopSelf(startId);
    }

    @Override
    public void onDestroy() {
        foregroundReady = false;
        stopAudio();
        if (timeout != null) handler.removeCallbacks(timeout);
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE);
        else stopForeground(true);
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    static void recordStartFailure(String reason) {
        startFailure = reason;
        foregroundReady = false;
    }

    private void startAudio(String socketName, String connectCode, String playbackMode) {
        startFailure = "";
        if (socketName == null || socketName.isEmpty() || connectCode == null || connectCode.isEmpty()) {
            recordStartFailure("InvalidAudioRequest");
            stopSelf();
            return;
        }
        if (audioThread != null) {
            startFailure = "AudioSessionAlreadyActive";
            return;
        }
        actualMode = "";
        audioThread = new Thread(() -> receiveAudio(socketName, connectCode, playbackMode), "AnPod audio receiver");
        audioThread.start();
    }

    private void receiveAudio(String socketName, String connectCode, String playbackMode) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
        try (LocalSocket socket = new LocalSocket()) {
            audioSocket = socket;
            socket.connect(new LocalSocketAddress(socketName, LocalSocketAddress.Namespace.ABSTRACT));
            socket.getOutputStream().write(connectCode.getBytes(java.nio.charset.StandardCharsets.UTF_8));
            socket.getOutputStream().flush();
            InputStream input = socket.getInputStream();
            byte[] header = new byte[8];
            new DataInputStream(input).readFully(header);
            int sampleRate = littleInt(header, 0);
            int channels = littleShort(header, 4);
            int bits = littleShort(header, 6);
            if (sampleRate < 8000 || sampleRate > 192000 || (channels != 1 && channels != 2) || bits != 16)
                throw new IOException("Unsupported PCM format");

            int channelMask = channels == 1 ? AudioFormat.CHANNEL_OUT_MONO : AudioFormat.CHANNEL_OUT_STEREO;
            int minBuffer = AudioTrack.getMinBufferSize(sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT);
            if (minBuffer <= 0) throw new IOException("Unsupported AudioTrack format");
            int frameBytes = channels * 2;
            int bufferBytes = minBuffer * 2;
            AudioTrack.Builder builder = new AudioTrack.Builder()
                    .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                    .setAudioFormat(new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                            .setSampleRate(sampleRate).setChannelMask(channelMask).build())
                    .setBufferSizeInBytes(bufferBytes).setTransferMode(AudioTrack.MODE_STREAM);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY);
            audioTrack = builder.build();
            if (audioTrack.getState() != AudioTrack.STATE_INITIALIZED) throw new IOException("AudioTrack initialization failed");

            requestAudioFocus();
            actualMode = "lowlatency".equalsIgnoreCase(playbackMode) ? "low-latency" : "balanced";
            int capacityFrames = Build.VERSION.SDK_INT >= 24
                    ? audioTrack.getBufferCapacityInFrames() : audioTrack.getBufferSizeInFrames();
            int minBufferFrames = Math.max(1, (minBuffer + frameBytes - 1) / frameBytes);
            int requestedStartFrames = capacityFrames;
            boolean lowLatencyBufferApplied = false;
            if ("low-latency".equals(actualMode)) {
                if (Build.VERSION.SDK_INT >= 24) {
                    int requested = Math.min(capacityFrames, Math.max(1, sampleRate * 15 / 1000));
                    int accepted = audioTrack.setBufferSizeInFrames(requested);
                    if (accepted < 0) actualMode = "balanced-fallback";
                    else {
                        lowLatencyBufferApplied = true;
                        if (accepted != requested) actualMode = "low-latency-limited";
                        if (Build.VERSION.SDK_INT >= 31 && !"balanced-fallback".equals(actualMode)) {
                            requestedStartFrames = Math.min(accepted, Math.max(1, sampleRate * 5 / 1000));
                            audioTrack.setStartThresholdInFrames(requestedStartFrames);
                        }
                    }
                } else actualMode = "balanced-fallback";
            }
            int effectiveFrames = audioTrack.getBufferSizeInFrames();
            int startThresholdFrames = Build.VERSION.SDK_INT >= 31 ? audioTrack.getStartThresholdInFrames() : -1;
            if (lowLatencyBufferApplied && effectiveFrames != Math.min(capacityFrames, Math.max(1, sampleRate * 15 / 1000)))
                actualMode = "low-latency-limited";
            byte[] buffer = new byte[Math.max(frameBytes, minBuffer + frameBytes - 1)];
            Pcm16FrameStream pcm = new Pcm16FrameStream(channels);
            int preRollFrames = lowLatencyBufferApplied
                    ? (startThresholdFrames > 0 ? Math.min(effectiveFrames, startThresholdFrames) : effectiveFrames)
                    : Math.min(effectiveFrames, minBufferFrames);
            int preRollBytes = preRollFrames * frameBytes;
            int preRolled = 0;
            while (preRolled < preRollBytes && !Thread.currentThread().isInterrupted()) {
                int count = pcm.readFrames(input, buffer, Math.min(buffer.length, preRollBytes - preRolled));
                if (count < 0) return;
                pcm.writeFramesForChannels((data, offset, length) ->
                        audioTrack.write(data, offset, length, AudioTrack.WRITE_BLOCKING), buffer, count);
                preRolled += count;
            }
            audioTrack.play();
            audioActive = true;
            while (!Thread.currentThread().isInterrupted()) {
                int count = pcm.readFrames(input, buffer, buffer.length);
                if (count < 0) break;
                pcm.writeFramesForChannels((data, offset, length) ->
                        audioTrack.write(data, offset, length, AudioTrack.WRITE_BLOCKING), buffer, count);
            }
        } catch (IOException | RuntimeException error) {
            startFailure = error.getClass().getSimpleName();
        } finally {
            audioActive = false;
            audioSocket = null;
            releaseAudio();
            audioThread = null;
            if (!foregroundReady) stopSelf();
        }
    }

    private static int littleInt(byte[] bytes, int offset) {
        return (bytes[offset] & 255) | ((bytes[offset + 1] & 255) << 8)
                | ((bytes[offset + 2] & 255) << 16) | ((bytes[offset + 3] & 255) << 24);
    }

    private static int littleShort(byte[] bytes, int offset) {
        return (bytes[offset] & 255) | ((bytes[offset + 1] & 255) << 8);
    }

    private void requestAudioFocus() throws IOException {
        audioManager = getSystemService(AudioManager.class);
        focusListener = change -> { if (change != AudioManager.AUDIOFOCUS_GAIN) stopSelf(); };
        int result;
        if (Build.VERSION.SDK_INT >= 26) {
            focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                    .setOnAudioFocusChangeListener(focusListener).build();
            result = audioManager.requestAudioFocus(focusRequest);
        } else result = audioManager.requestAudioFocus(focusListener, AudioManager.STREAM_MUSIC, AudioManager.AUDIOFOCUS_GAIN);
        if (result != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) throw new IOException("Audio focus denied");
    }

    private void stopAudio() {
        Thread thread = audioThread;
        if (thread != null) thread.interrupt();
        LocalSocket socket = audioSocket;
        if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        audioActive = false;
        releaseAudio();
    }

    private synchronized void releaseAudio() {
        AudioTrack track = audioTrack;
        audioTrack = null;
        if (track != null) {
            try { track.pause(); } catch (RuntimeException ignored) {}
            try { track.flush(); } catch (RuntimeException ignored) {}
            try { track.stop(); } catch (RuntimeException ignored) {}
            track.release();
        }
        if (audioManager != null && focusListener != null) {
            if (Build.VERSION.SDK_INT >= 26 && focusRequest != null) audioManager.abandonAudioFocusRequest(focusRequest);
            else audioManager.abandonAudioFocus(focusListener);
        }
        focusRequest = null;
        focusListener = null;
        audioManager = null;
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "AnPod Receiver", NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("显示 Receiver 管理与音频播放服务状态。");
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
    }
}
