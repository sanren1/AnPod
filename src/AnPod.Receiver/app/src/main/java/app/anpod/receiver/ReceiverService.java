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
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.util.Enumeration;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
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
    static final AtomicLong PCM_BYTES_WRITTEN = new AtomicLong();

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
    static volatile String networkAudioState = "stopped";
    static volatile String networkAudioFailure = "";
    static volatile String networkAudioAddress = "";
    static volatile int networkAudioPort;
    private static long statusAudioId;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable timeout;
    private volatile AudioSession currentAudio;
    private AudioRequest pendingAudio;
    private int latestStartId;
    private volatile boolean destroyed;
    private volatile long networkRecoveryDeadlineElapsedMs;
    private volatile P5cReceiverCoordinator networkCoordinator;

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
        networkAudioState = "stopped";
        networkAudioFailure = "";
        networkAudioAddress = "";
        networkAudioPort = 0;
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        latestStartId = startId;
        boolean networkAudio = intent != null && BootstrapActivity.ACTION_START_NETWORK_AUDIO.equals(intent.getAction());
        boolean audio = intent != null && BootstrapActivity.ACTION_START_AUDIO.equals(intent.getAction());
        // A management retry must not downgrade an active mediaPlayback FGS to
        // shortService or attach a short-service timeout to the audio session.
        if (!audio && currentAudio != null) return START_NOT_STICKY;
        if (networkAudio && networkCoordinator != null) return START_NOT_STICKY;

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

        if (networkAudio) {
            if (timeout != null) handler.removeCallbacks(timeout);
            timeout = () -> stopNetworkBootstrap("BootstrapTimeout");
            handler.postDelayed(timeout, 25_000L);
            startNetworkBootstrap(intent.getStringExtra("networkAddress"), intent.getIntExtra("networkPrefixLength", 0));
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
        if (pendingAudio != null) pendingAudio.destroy();
        pendingAudio = null;
        AudioSession session = currentAudio;
        currentAudio = null;
        if (session != null) cancelAudio(session, "AudioServiceDestroyed");
        P5cReceiverCoordinator coordinator = networkCoordinator;
        networkCoordinator = null;
        if (coordinator != null) coordinator.close();
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

    private AudioSession startAudio(AudioRequest request) {
        if (destroyed) return null;
        AudioSession existing = currentAudio;
        if (existing != null) {
            if (pendingAudio != null) pendingAudio.destroy();
            pendingAudio = request;
            cancelAudio(existing, "AudioSessionReplaced");
            return null;
        }
        if (request.udpContext == null && (request.socketName == null || request.socketName.isEmpty()
                || request.connectCode == null || request.connectCode.isEmpty())) {
            recordStartFailure("InvalidAudioRequest");
            request.destroy();
            stopSelf(latestStartId);
            return null;
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
        if (request.udpContext != null) PCM_BYTES_WRITTEN.set(0);
        session.thread = new Thread(() -> receiveAudio(session), "AnPod audio receiver");
        session.thread.start();
        return session;
    }

    /** In-process hook for an authenticated session coordinator; no Intent or exported API calls it. */
    void startUdpAudio(UdpSessionContext context) {
        Runnable start = () -> {
            if (destroyed) context.destroy();
            else startAudio(AudioRequest.udp(context));
        };
        if (Looper.myLooper() == Looper.getMainLooper()) start.run();
        else if (!handler.post(start)) context.destroy();
    }

    private void startNetworkBootstrap(String address, int prefixLength) {
        try {
            networkAudioState = "starting";
            networkAudioFailure = "";
            startFailure = "";
            networkRecoveryDeadlineElapsedMs = 0;
            InetAddress local = parseIpv4(address);
            if (!isAssignedPrivateAddress(local, prefixLength)) throw new IOException("InvalidReceiverInterface");
            PairingProtocol.BindingMaterial binding = new BindingStore(getFilesDir()).load();
            P5cReceiverCoordinator.PlaybackEndpoint playback = new P5cReceiverCoordinator.PlaybackEndpoint() {
                private AudioSession ownedSession;

                @Override public boolean start(UdpSessionContext context) throws IOException {
                    AudioStartTicket ticket = new AudioStartTicket();
                    boolean posted = handler.post(() -> {
                        synchronized (ticket) {
                            try {
                                if (ticket.cancelled || destroyed || currentAudio != null || pendingAudio != null) {
                                    context.destroy();
                                    return;
                                }
                                Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                                        ? new Notification.Builder(ReceiverService.this, CHANNEL_ID)
                                        : new Notification.Builder(ReceiverService.this);
                                Notification notification = builder.setSmallIcon(android.R.drawable.stat_sys_headset)
                                        .setContentTitle("AnPod Receiver").setContentText("正在播放电脑音频。")
                                        .setOngoing(true).build();
                                if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION_ID, notification,
                                        ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
                                else startForeground(NOTIFICATION_ID, notification);
                                AudioSession accepted = startAudio(AudioRequest.udp(context));
                                if (accepted == null || accepted.request.udpContext != context || currentAudio != accepted) {
                                    throw new IOException("NetworkAudioSessionNotAccepted");
                                }
                                ticket.accepted = accepted;
                                ownedSession = accepted;
                                if (timeout != null) handler.removeCallbacks(timeout);
                                timeout = null;
                                foregroundReady = true;
                                networkRecoveryDeadlineElapsedMs = 0;
                                networkAudioState = "receiverRunning";
                            } catch (IOException | RuntimeException error) {
                                recordNetworkFailure(error instanceof IOException ? error.getMessage() : error.getClass().getSimpleName());
                                context.destroy();
                            } finally { ticket.finished.countDown(); }
                        }
                    });
                    if (!posted) { context.destroy(); return false; }
                    try {
                        if (!ticket.finished.await(3, TimeUnit.SECONDS)) {
                            synchronized (ticket) {
                                ticket.cancelled = true;
                                if (ticket.accepted != null) cancelAudio(ticket.accepted, "NetworkStartTimedOut");
                            }
                            return false;
                        }
                    } catch (InterruptedException interrupted) {
                        Thread.currentThread().interrupt();
                        synchronized (ticket) {
                            ticket.cancelled = true;
                            if (ticket.accepted != null) cancelAudio(ticket.accepted, "NetworkStartInterrupted");
                        }
                        return false;
                    }
                    return ticket.accepted != null && currentAudio == ticket.accepted
                            && ticket.accepted.request.udpContext == context;
                }
                @Override public void pause() throws IOException {
                    setUdpAudioPaused(ownedSession, true);
                    networkAudioState = "paused";
                }
                @Override public void resume() throws IOException {
                    setUdpAudioPaused(ownedSession, false);
                    networkAudioState = "receiverRunning";
                }
                @Override public void stop() {
                    AudioSession session = ownedSession;
                    if (session != null && currentAudio == session) cancelAudio(session, "NetworkSessionStopped");
                    networkRecoveryDeadlineElapsedMs = android.os.SystemClock.elapsedRealtime() + 30_000L;
                    networkAudioState = "authenticated";
                }
                @Override public boolean streaming() throws IOException {
                    if ("failed".equals(networkAudioState)) throw new IOException(networkAudioFailure.isEmpty()
                            ? "NetworkAudioWorkerFailed" : networkAudioFailure);
                    AudioSession session = ownedSession;
                    boolean streaming = session != null && currentAudio == session && PCM_BYTES_WRITTEN.get() > 0;
                    if (streaming) networkAudioState = "streaming";
                    else if (session != null && currentAudio == session && "streaming".equals(networkAudioState)) networkAudioState = "receiverRunning";
                    return streaming;
                }
                @Override public void shutdown() {
                    networkAudioState = "stopped";
                    networkCoordinator = null;
                    networkAudioAddress = "";
                    networkAudioPort = 0;
                    handler.post(() -> { if (currentAudio == null) stopSelf(latestStartId); });
                }
            };
            P5cReceiverCoordinator coordinator = P5cReceiverCoordinator.openOnInterface(binding, playback, local, prefixLength);
            networkCoordinator = coordinator;
            networkAudioAddress = coordinator.localAddress().getHostAddress();
            networkAudioPort = coordinator.port();
            coordinator.start();
            coordinator.startDiscovery();
            networkAudioState = "authenticated";
            Thread waiter = new Thread(() -> {
                try {
                    while (true) {
                        boolean completed = coordinator.await(250, TimeUnit.MILLISECONDS);
                        if (completed) {
                            if (networkCoordinator == coordinator) networkCoordinator = null;
                            if (currentAudio == null) handler.post(() -> stopSelf(latestStartId));
                            break;
                        }
                        long recoveryDeadline = networkRecoveryDeadlineElapsedMs;
                        if (recoveryDeadline != 0 && currentAudio == null
                                && android.os.SystemClock.elapsedRealtime() >= recoveryDeadline) {
                            stopNetworkBootstrap("RecoveryTimeout");
                            break;
                        }
                    }
                } catch (InterruptedException ignored) { Thread.currentThread().interrupt(); }
                finally { if (networkCoordinator == coordinator) networkCoordinator = null; }
            }, "AnPod P5-C bootstrap timeout");
            waiter.setDaemon(true);
            waiter.start();
        } catch (Exception error) {
            recordNetworkFailure(error instanceof IOException ? error.getMessage() : error.getClass().getSimpleName());
            stopSelf(latestStartId);
        }
    }

    private static boolean isAssignedPrivateAddress(InetAddress address, int prefixLength) throws SocketException {
        if (!(address instanceof Inet4Address) || !address.isSiteLocalAddress() || prefixLength < 1 || prefixLength > 32) return false;
        Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
        while (interfaces != null && interfaces.hasMoreElements()) {
            NetworkInterface network = interfaces.nextElement();
            if (!network.isUp() || network.isLoopback()) continue;
            for (java.net.InterfaceAddress item : network.getInterfaceAddresses())
                if (address.equals(item.getAddress()) && item.getNetworkPrefixLength() == prefixLength) return true;
        }
        return false;
    }

    private static InetAddress parseIpv4(String text) throws IOException {
        String[] parts = text == null ? new String[0] : text.split("\\.", -1);
        if (parts.length != 4) throw new IOException("InvalidReceiverInterface");
        byte[] address = new byte[4];
        for (int i = 0; i < parts.length; i++) {
            try {
                int value = Integer.parseInt(parts[i]);
                if (value < 0 || value > 255 || !Integer.toString(value).equals(parts[i]))
                    throw new IOException("InvalidReceiverInterface");
                address[i] = (byte) value;
            } catch (NumberFormatException error) { throw new IOException("InvalidReceiverInterface"); }
        }
        return InetAddress.getByAddress(address);
    }

    private void stopNetworkBootstrap(String reason) {
        P5cReceiverCoordinator coordinator = networkCoordinator;
        networkCoordinator = null;
        if (coordinator != null) coordinator.close();
        AudioSession session = currentAudio;
        if (session != null && session.request.udpContext != null) cancelAudio(session, reason);
        networkAudioAddress = "";
        networkAudioPort = 0;
        if (networkAudioState.equals("authenticated") || networkAudioState.equals("receiverRunning")) recordNetworkFailure(reason);
        handler.post(() -> stopSelf(latestStartId));
    }

    private static void recordNetworkFailure(String reason) {
        networkAudioFailure = reason == null ? "NetworkStartFailed" : reason;
        networkAudioState = "failed";
        startFailure = networkAudioFailure;
    }

    /** Internal pause hook: drop pending and incoming history without adding a control datagram. */
    private void setUdpAudioPaused(AudioSession session, boolean paused) throws IOException {
        if (session == null || currentAudio != session || session.request.udpContext == null || session.udpQueue == null)
            throw new IOException("NetworkAudioSessionNotOwned");
        session.udpQueue.setPaused(paused);
    }

    private void receiveAudio(AudioSession session) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
        AudioTrack track = null;
        AudioManager manager = null;
        AudioFocusRequest focusRequest = null;
        AudioManager.OnAudioFocusChangeListener focusListener = null;
        UdpPcmQueue udpQueue = null;
        UdpReceiver udpReceiver = null;
        Thread networkThread = null;
        String failure = "";
        try {
            ensureRunning(session);
            LocalSocket ownedSocket = session.request.udpContext == null ? new LocalSocket() : null;
            try (ownedSocket) {
                InputStream input;
                if (ownedSocket == null) {
                    udpQueue = new UdpPcmQueue(session.request.udpContext.parameters);
                    udpReceiver = new UdpReceiver(session.request.udpContext, udpQueue);
                    UdpReceiver receiverForThread = udpReceiver;
                    networkThread = new Thread(() -> {
                        Process.setThreadPriority(Process.THREAD_PRIORITY_DEFAULT);
                        receiverForThread.run();
                    }, "AnPod UDP packet receiver");
                    synchronized (session) {
                        ensureRunning(session);
                        session.udpQueue = udpQueue;
                        session.udpReceiver = udpReceiver;
                        session.networkThread = networkThread;
                        networkThread.start();
                    }
                    input = udpQueue.asPcmInputStream();
                } else {
                    synchronized (session) {
                        ensureRunning(session);
                        session.socket = ownedSocket;
                    }
                    ownedSocket.connect(new LocalSocketAddress(session.request.socketName, LocalSocketAddress.Namespace.ABSTRACT));
                    ownedSocket.getOutputStream().write(session.request.connectCode.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                    ownedSocket.getOutputStream().flush();
                    input = ownedSocket.getInputStream();
                }
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
                    } else publishMode(session, "balanced");
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
            if (udpReceiver != null) udpReceiver.close();
            if (udpQueue != null) udpQueue.stop();
            if (networkThread != null && networkThread != Thread.currentThread()) {
                boolean interrupted = Thread.interrupted();
                while (networkThread.isAlive()) {
                    try {
                        networkThread.join();
                    } catch (InterruptedException ignored) {
                        interrupted = true;
                    }
                }
                if (interrupted) Thread.currentThread().interrupt();
            }
            session.request.destroy();
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) {
                    audioActive = false;
                    if (failure.isEmpty()) failure = session.stopReason.isEmpty() ? "AudioSessionEnded" : session.stopReason;
                }
            }
            synchronized (session) {
                session.socket = null;
                if (session.udpReceiver == udpReceiver) session.udpReceiver = null;
                if (session.udpQueue == udpQueue) session.udpQueue = null;
                if (session.networkThread == networkThread) session.networkThread = null;
            }
            String finalFailure = failure;
            if (session.request.udpContext != null && !finalFailure.isEmpty()
                    && !"NetworkSessionStopped".equals(finalFailure)) {
                networkAudioFailure = finalFailure;
                networkAudioState = "failed";
                startFailure = finalFailure;
            }
            handler.post(() -> finishAudio(session, finalFailure));
        }
    }

    private void writeFrames(AudioSession session, AudioTrack track, Pcm16FrameStream pcm, byte[] buffer, int count, int frameBytes) throws IOException {
        pcm.writeFramesForChannels((data, offset, length) -> {
            synchronized (session) {
                ensureRunning(session);
                int written = track.write(data, offset, length, AudioTrack.WRITE_BLOCKING);
                if (written <= 0) throw new IOException("AudioTrack.write failed: " + written);
                if (session.request.udpContext != null) PCM_BYTES_WRITTEN.addAndGet(written);
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
            if (session.request.udpContext != null) {
                if (failure.isEmpty() || "NetworkSessionStopped".equals(failure)) networkAudioState = networkCoordinator != null ? "authenticated" : "stopped";
                else if (!networkAudioState.equals("failed")) recordNetworkFailure(failure);
            }
            if (session.request.udpContext == null || "stopped".equals(networkAudioState)) stopSelf(latestStartId);
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
        UdpReceiver receiver = session.udpReceiver;
        if (receiver != null) receiver.close();
        UdpPcmQueue queue = session.udpQueue;
        if (queue != null) queue.stop();
        Thread networkThread = session.networkThread;
        if (networkThread != null) networkThread.interrupt();
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
        final UdpSessionContext udpContext;

        AudioRequest(String socketName, String connectCode, String playbackMode) {
            this(socketName, connectCode, playbackMode, null);
        }

        private AudioRequest(String socketName, String connectCode, String playbackMode, UdpSessionContext udpContext) {
            this.socketName = socketName;
            this.connectCode = connectCode;
            this.playbackMode = playbackMode;
            this.udpContext = udpContext;
        }

        static AudioRequest udp(UdpSessionContext context) {
            return new AudioRequest(null, null, context.playbackMode, context);
        }

        void destroy() { if (udpContext != null) udpContext.destroy(); }
    }

    private static final class AudioStartTicket {
        final CountDownLatch finished = new CountDownLatch(1);
        volatile boolean cancelled;
        volatile AudioSession accepted;

        AudioStartTicket() {}
    }

    private static final class AudioSession {
        final long id;
        final AudioRequest request;
        volatile Thread thread;
        volatile Thread networkThread;
        volatile LocalSocket socket;
        volatile UdpReceiver udpReceiver;
        volatile UdpPcmQueue udpQueue;
        volatile boolean cancelled;
        volatile String stopReason = "";

        AudioSession(long id, AudioRequest request) {
            this.id = id;
            this.request = request;
        }
    }
}
