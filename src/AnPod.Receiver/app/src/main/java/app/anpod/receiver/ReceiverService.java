package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioTrack;
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
import java.util.concurrent.atomic.AtomicLong;

/**
 * Android playback receiver adapted from ysbing/AudioShare's server Main.java
 * (LGPL-3.0-or-later). The protocol and AudioTrack streaming path are shared
 * with the app_process implementation; see upstream/PATCHES.md for changes.
 */
public final class ReceiverService extends Service {
    private static final String CHANNEL_ID = "receiver-management";
    private static final int NOTIFICATION_ID = 1;
    private static final long MAX_RUN_MS = 120_000L;
    private static final Object STATUS_LOCK = new Object();
    private static final AtomicLong NEXT_AUDIO_ID = new AtomicLong();

    static volatile boolean foregroundReady;
    static volatile long startedAtElapsedMs;
    static volatile String startFailure = "";
    static volatile boolean audioActive;
    static volatile String actualMode = "";
    static volatile int actualSampleRate = -1;
    static volatile int bufferCapacityFrames = -1;
    static volatile int effectiveBufferFrames = -1;
    static volatile int startThresholdFrames = -1;
    static volatile int underrunCount = -1;
    private static long statusAudioId;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable timeout;
    private volatile AudioSession currentAudio;
    private AudioRequest pendingAudio;
    private int latestStartId;
    private volatile boolean destroyed;

    @Override
    public void onCreate() {
        super.onCreate();
        foregroundReady = false;
        audioActive = false;
        actualMode = "";
        actualSampleRate = -1;
        bufferCapacityFrames = -1;
        effectiveBufferFrames = -1;
        startThresholdFrames = -1;
        underrunCount = -1;
        startedAtElapsedMs = 0;
        startFailure = "";
        synchronized (STATUS_LOCK) { statusAudioId = 0; }
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        latestStartId = startId;
        boolean audio = intent != null && BootstrapActivity.ACTION_START_AUDIO.equals(intent.getAction());
        // A management retry must not downgrade an active mediaPlayback FGS to
        // shortService or attach a short-service timeout to the audio session.
        if (!audio && currentAudio != null) return START_NOT_STICKY;

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
                        ? ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                        : ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE;
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
            startAudio(new AudioRequest(intent.getStringExtra("socketName"), intent.getStringExtra("connectCode"),
                    intent.getStringExtra("playbackMode")));
            return START_NOT_STICKY;
        }

        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = () -> stopSelf(latestStartId);
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
        destroyed = true;
        foregroundReady = false;
        pendingAudio = null;
        AudioSession session = currentAudio;
        currentAudio = null;
        if (session != null) cancelAudio(session, "AudioServiceDestroyed");
        if (timeout != null) handler.removeCallbacks(timeout);
        synchronized (STATUS_LOCK) {
            if (session != null && statusAudioId == session.id) {
                statusAudioId = 0;
                audioActive = false;
                startFailure = "AudioServiceDestroyed";
            }
        }
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

    private void startAudio(AudioRequest request) {
        if (destroyed) return;
        AudioSession existing = currentAudio;
        if (existing != null) {
            pendingAudio = request;
            cancelAudio(existing, "AudioSessionReplaced");
            return;
        }
        if (request.socketName == null || request.socketName.isEmpty()
                || request.connectCode == null || request.connectCode.isEmpty()) {
            recordStartFailure("InvalidAudioRequest");
            stopSelf(latestStartId);
            return;
        }
        AudioSession session = new AudioSession(NEXT_AUDIO_ID.incrementAndGet(), request);
        currentAudio = session;
        synchronized (STATUS_LOCK) {
            statusAudioId = session.id;
            audioActive = false;
            actualMode = "";
            actualSampleRate = -1;
            bufferCapacityFrames = -1;
            effectiveBufferFrames = -1;
            startThresholdFrames = -1;
            underrunCount = -1;
            startFailure = "";
        }
        session.thread = new Thread(() -> receiveAudio(session), "AnPod audio receiver");
        session.thread.start();
    }

    private void receiveAudio(AudioSession session) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
        AudioTrack track = null;
        AudioManager manager = null;
        AudioFocusRequest focusRequest = null;
        AudioManager.OnAudioFocusChangeListener focusListener = null;
        String failure = "";
        try {
            ensureRunning(session);
            LocalSocket socket = new LocalSocket();
            synchronized (session) {
                ensureRunning(session);
                session.socket = socket;
            }
            try (LocalSocket ownedSocket = socket) {
                ownedSocket.connect(new LocalSocketAddress(session.request.socketName, LocalSocketAddress.Namespace.ABSTRACT));
                ownedSocket.getOutputStream().write(session.request.connectCode.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                ownedSocket.getOutputStream().flush();
                InputStream input = ownedSocket.getInputStream();
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
                int capacityFrames;
                int requestedBufferFrames = -1;
                int requestedStartFrames = -1;
                int minBufferFrames = Math.max(1, (minBuffer + frameBytes - 1) / frameBytes);
                synchronized (session) {
                    ensureRunning(session);
                    AudioTrack.Builder builder = new AudioTrack.Builder()
                            .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                            .setAudioFormat(new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                                    .setSampleRate(sampleRate).setChannelMask(channelMask).build())
                            .setBufferSizeInBytes(minBuffer * 2).setTransferMode(AudioTrack.MODE_STREAM);
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY);
                    track = builder.build();
                    if (track.getState() != AudioTrack.STATE_INITIALIZED) throw new IOException("AudioTrack initialization failed");
                    capacityFrames = Build.VERSION.SDK_INT >= 24
                            ? track.getBufferCapacityInFrames() : track.getBufferSizeInFrames();
                    if ("lowlatency".equalsIgnoreCase(session.request.playbackMode)) {
                        if (Build.VERSION.SDK_INT >= 24) {
                            int requested = Math.min(capacityFrames, Math.max(1, sampleRate * 15 / 1000));
                            int accepted = track.setBufferSizeInFrames(requested);
                            if (accepted < 0) publishMode(session, "balanced-fallback");
                            else {
                                requestedBufferFrames = requested;
                                String mode = accepted == requested ? "low-latency" : "low-latency-limited";
                                if (Build.VERSION.SDK_INT >= 31) {
                                    requestedStartFrames = Math.min(accepted, Math.max(1, sampleRate * 5 / 1000));
                                    track.setStartThresholdInFrames(requestedStartFrames);
                                }
                                publishMode(session, mode);
                            }
                        } else publishMode(session, "balanced-fallback");
                    }
                    int effectiveFrames = track.getBufferSizeInFrames();
                    int thresholdFrames = Build.VERSION.SDK_INT >= 31 ? track.getStartThresholdInFrames() : -1;
                    String mode = currentMode(session);
                    if (mode.startsWith("low-latency")
                            && ((requestedBufferFrames > 0 && effectiveFrames != requestedBufferFrames)
                                    || (requestedStartFrames > 0 && thresholdFrames != requestedStartFrames))) {
                        mode = "low-latency-limited";
                    }
                    publishMetrics(session, track.getSampleRate(), capacityFrames, effectiveFrames, thresholdFrames,
                            Build.VERSION.SDK_INT >= 24 ? track.getUnderrunCount() : -1, mode);
                }

                byte[] buffer = new byte[Math.max(frameBytes, minBuffer + frameBytes - 1)];
                Pcm16FrameStream pcm = new Pcm16FrameStream(channels);
                int effectiveFrames = track.getBufferSizeInFrames();
                int thresholdFrames = Build.VERSION.SDK_INT >= 31 ? track.getStartThresholdInFrames() : -1;
                int preRollFrames = currentMode(session).startsWith("low-latency") && thresholdFrames > 0
                        ? Math.min(effectiveFrames, thresholdFrames) : Math.min(effectiveFrames, minBufferFrames);
                int preRollBytes = preRollFrames * frameBytes;
                int preRolled = 0;
                while (preRolled < preRollBytes) {
                    ensureRunning(session);
                    int count = pcm.readFrames(input, buffer, Math.min(buffer.length, preRollBytes - preRolled));
                    if (count < 0) throw new IOException("AudioStreamEndedBeforePlayback");
                    writeFrames(session, track, pcm, buffer, count, frameBytes);
                    preRolled += count;
                }

                manager = getSystemService(AudioManager.class);
                final AudioSession focusSession = session;
                focusListener = change -> {
                    if (change != AudioManager.AUDIOFOCUS_GAIN) cancelAudio(focusSession, "AudioFocusLost");
                };
                synchronized (session) {
                    ensureRunning(session);
                    if (Build.VERSION.SDK_INT >= 26) {
                        focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                                .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                                .setOnAudioFocusChangeListener(focusListener).build();
                        if (manager.requestAudioFocus(focusRequest) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
                            throw new IOException("Audio focus denied");
                    } else if (manager.requestAudioFocus(focusListener, AudioManager.STREAM_MUSIC,
                            AudioManager.AUDIOFOCUS_GAIN) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                        throw new IOException("Audio focus denied");
                    }
                    ensureRunning(session);
                    track.play();
                    if (track.getPlayState() != AudioTrack.PLAYSTATE_PLAYING) throw new IOException("AudioTrack did not start");
                    synchronized (STATUS_LOCK) {
                        if (statusAudioId == session.id) audioActive = true;
                    }
                }
                while (true) {
                    ensureRunning(session);
                    int count = pcm.readFrames(input, buffer, buffer.length);
                    if (count < 0) {
                        failure = "AudioStreamEnded";
                        break;
                    }
                    writeFrames(session, track, pcm, buffer, count, frameBytes);
                    if (Build.VERSION.SDK_INT >= 24) {
                        int underruns = track.getUnderrunCount();
                        synchronized (STATUS_LOCK) {
                            if (statusAudioId == session.id) underrunCount = underruns;
                        }
                    }
                }
            }
        } catch (IOException | RuntimeException error) {
            failure = session.cancelled ? session.stopReason : describeFailure(error);
        } finally {
            String trackReleaseFailure = releaseTrack(track);
            String focusReleaseFailure = abandonAudioFocus(manager, focusRequest, focusListener);
            if (!trackReleaseFailure.isEmpty()) failure = appendFailure(failure, trackReleaseFailure);
            if (!focusReleaseFailure.isEmpty()) failure = appendFailure(failure, focusReleaseFailure);
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) {
                    audioActive = false;
                    if (failure.isEmpty()) failure = session.stopReason.isEmpty() ? "AudioSessionEnded" : session.stopReason;
                }
            }
            synchronized (session) { session.socket = null; }
            String finalFailure = failure;
            handler.post(() -> finishAudio(session, finalFailure));
        }
    }

    private void writeFrames(AudioSession session, AudioTrack track, Pcm16FrameStream pcm, byte[] buffer, int count, int frameBytes) throws IOException {
        pcm.writeFramesForChannels((data, offset, length) -> {
            synchronized (session) {
                ensureRunning(session);
                int written = track.write(data, offset, length, AudioTrack.WRITE_BLOCKING);
                if (written <= 0) throw new IOException("AudioTrack.write failed: " + written);
                return written;
            }
        }, buffer, count);
        if (count % frameBytes != 0) throw new IOException("PCM frame alignment failed");
    }

    private void finishAudio(AudioSession session, String failure) {
        if (currentAudio != session) return;
        currentAudio = null;
        if (pendingAudio != null && !destroyed) {
            AudioRequest next = pendingAudio;
            pendingAudio = null;
            startAudio(next);
        } else {
            if (!destroyed && statusAudioId == session.id) startFailure = failure;
            stopSelf(latestStartId);
        }
    }

    private void cancelAudio(AudioSession session, String reason) {
        synchronized (session) {
            if (!session.cancelled) {
                session.cancelled = true;
                session.stopReason = reason;
            }
        }
        synchronized (STATUS_LOCK) { if (statusAudioId == session.id) audioActive = false; }
        Thread thread = session.thread;
        if (thread != null) thread.interrupt();
        LocalSocket socket = session.socket;
        if (socket != null) try { socket.close(); } catch (IOException ignored) {}
    }

    private void ensureRunning(AudioSession session) throws IOException {
        synchronized (session) {
            if (session.cancelled || destroyed || currentAudio != session || Thread.currentThread().isInterrupted())
                throw new IOException(session.stopReason.isEmpty() ? "AudioSessionStopped" : session.stopReason);
        }
    }

    private void publishMetrics(AudioSession session, int rate, int capacity, int effective, int threshold, int undrruns, String mode) {
        synchronized (STATUS_LOCK) {
            if (statusAudioId != session.id) return;
            actualSampleRate = rate;
            bufferCapacityFrames = capacity;
            effectiveBufferFrames = effective;
            startThresholdFrames = threshold;
            underrunCount = undrruns;
            actualMode = mode;
        }
    }

    private void publishMode(AudioSession session, String mode) {
        synchronized (STATUS_LOCK) { if (statusAudioId == session.id) actualMode = mode; }
    }

    private String currentMode(AudioSession session) {
        synchronized (STATUS_LOCK) { return statusAudioId == session.id ? actualMode : ""; }
    }

    private static String describeFailure(Exception error) {
        String message = error.getMessage();
        if (message == null || message.isEmpty()) return error.getClass().getSimpleName();
        String description = (error.getClass().getSimpleName() + ": " + message).replace('\r', ' ').replace('\n', ' ');
        return description.substring(0, Math.min(description.length(), 180));
    }

    private static String releaseTrack(AudioTrack track) {
        if (track == null) return "";
        String failure = "";
        try { track.pause(); } catch (RuntimeException error) { failure = appendFailure(failure, describeFailure(error)); }
        try { track.flush(); } catch (RuntimeException error) { failure = appendFailure(failure, describeFailure(error)); }
        try { track.stop(); } catch (RuntimeException error) { failure = appendFailure(failure, describeFailure(error)); }
        try { track.release(); } catch (RuntimeException error) { failure = appendFailure(failure, describeFailure(error)); }
        return failure;
    }

    private static String abandonAudioFocus(AudioManager manager, AudioFocusRequest request,
            AudioManager.OnAudioFocusChangeListener listener) {
        if (manager == null || listener == null) return "";
        try {
            if (Build.VERSION.SDK_INT >= 26 && request != null) manager.abandonAudioFocusRequest(request);
            else manager.abandonAudioFocus(listener);
            return "";
        } catch (RuntimeException error) {
            return "AudioFocusRelease: " + describeFailure(error);
        }
    }

    private static String appendFailure(String previous, String next) {
        if (previous == null || previous.isEmpty()) return next;
        return previous + "; " + next;
    }

    private static int littleInt(byte[] bytes, int offset) {
        return (bytes[offset] & 255) | ((bytes[offset + 1] & 255) << 8)
                | ((bytes[offset + 2] & 255) << 16) | ((bytes[offset + 3] & 255) << 24);
    }

    private static int littleShort(byte[] bytes, int offset) {
        return (bytes[offset] & 255) | ((bytes[offset + 1] & 255) << 8);
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "AnPod Receiver", NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("显示 Receiver 管理与音频播放服务状态。");
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
    }

    private static final class AudioRequest {
        final String socketName;
        final String connectCode;
        final String playbackMode;

        AudioRequest(String socketName, String connectCode, String playbackMode) {
            this.socketName = socketName;
            this.connectCode = connectCode;
            this.playbackMode = playbackMode;
        }
    }

    private static final class AudioSession {
        final long id;
        final AudioRequest request;
        volatile Thread thread;
        volatile LocalSocket socket;
        volatile boolean cancelled;
        volatile String stopReason = "";

        AudioSession(long id, AudioRequest request) {
            this.id = id;
            this.request = request;
        }
    }
}
