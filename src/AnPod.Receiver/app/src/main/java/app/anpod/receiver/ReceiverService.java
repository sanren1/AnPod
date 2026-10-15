package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
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
import android.net.ConnectivityManager;
import android.net.LinkAddress;
import android.net.LinkProperties;
import android.net.Network;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.Process;
import android.os.Binder;

import com.anpod.server.Pcm16FrameStream;

import java.io.DataInputStream;
import java.io.File;
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
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;
import java.util.UUID;

/**
 * Android playback receiver adapted from ysbing/AudioShare's server Main.java
 * (LGPL-3.0-or-later). The protocol and AudioTrack streaming path are shared
 * with the app_process implementation; see upstream/PATCHES.md for changes.
 */
public final class ReceiverService extends Service {
    public static final String ACTION_UI_START_NETWORK_AUDIO = "app.anpod.receiver.action.UI_START_NETWORK_AUDIO";
    private static final String ACTION_NOTIFICATION_STOP = "app.anpod.receiver.action.NOTIFICATION_STOP";
    private static final String EXTRA_UI_REQUEST_ID = "uiRequestId";
    private static final String CHANNEL_ID = "receiver-management";
    private static final int NOTIFICATION_ID = 1;
    private static final long ADB_NETWORK_BOOTSTRAP_TIMEOUT_MS = 25_000L;
    private static final String EXTRA_UI_OWNER_ID = "uiOwnerId";
    private static final String EXTRA_UI_GENERATION = "uiGeneration";
    private static final long MAX_RUN_MS = 120_000L;
    private static final Object STATUS_LOCK = new Object();
    private static volatile ReceiverService activeService;
    private static final AtomicLong NEXT_AUDIO_ID = new AtomicLong();
    static final AtomicLong PCM_BYTES_WRITTEN = new AtomicLong();

    static volatile boolean foregroundReady;
    static volatile long startedAtElapsedMs;
    static volatile String startFailure = "";
    static volatile boolean audioActive;
    static volatile String actualMode = "";
    static volatile int actualSampleRate = -1;
    static volatile int actualChannels = -1;
    static volatile int bufferCapacityFrames = -1;
    static volatile int requestedBufferFrames = -1;
    static volatile int acceptedBufferFrames = -1;
    static volatile int effectiveBufferFrames = -1;
    static volatile int startThresholdFrames = -1;
    static volatile int preRollFrames = -1;
    static volatile int underrunCount = -1;
    static volatile int underrunIncrement = -1;
    static volatile long tcpDroppedBacklogFrames = -1;
    static volatile String networkAudioState = "stopped";
    static volatile String networkAudioFailure = "";
    static volatile String networkAudioAddress = "";
    static volatile int networkAudioPort;
    private static long statusAudioId;

    static final class PlaybackMetrics {
        final int sampleRate, channels, jitterTargetFrames, underrunCount;
        final int requestedFrames, acceptedFrames, effectiveFrames, capacityFrames, startThresholdFrames, preRollFrames;
        final long expiredFrames, queuedFrames, pendingFrames, tcpDroppedFrames;
        final Boolean advancing;
        final boolean udpQueueActive;

        PlaybackMetrics(int sampleRate, int channels, int jitterTargetFrames, long expiredFrames,
                long queuedFrames, long pendingFrames, int underrunCount, long tcpDroppedFrames,
                Boolean advancing, int requestedFrames, int acceptedFrames, int effectiveFrames,
                int capacityFrames, int startThresholdFrames, int preRollFrames, boolean udpQueueActive) {
            this.sampleRate = sampleRate; this.channels = channels; this.jitterTargetFrames = jitterTargetFrames;
            this.expiredFrames = expiredFrames; this.queuedFrames = queuedFrames; this.pendingFrames = pendingFrames;
            this.underrunCount = underrunCount; this.tcpDroppedFrames = tcpDroppedFrames; this.advancing = advancing;
            this.requestedFrames = requestedFrames; this.acceptedFrames = acceptedFrames;
            this.effectiveFrames = effectiveFrames; this.capacityFrames = capacityFrames;
            this.startThresholdFrames = startThresholdFrames; this.preRollFrames = preRollFrames;
            this.udpQueueActive = udpQueueActive;
        }

        static PlaybackMetrics unknown() {
            return new PlaybackMetrics(-1, -1, -1, -1, -1, -1, -1, -1,
                    null, -1, -1, -1, -1, -1, -1, false);
        }
    }

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable timeout;
    private static volatile AudioSession currentAudio;
    private AudioRequest pendingAudio;
    private int operationStartId;
    private volatile boolean destroyed;
    private volatile long networkRecoveryDeadlineElapsedMs;
    private volatile P5cReceiverCoordinator networkCoordinator;
    private final Object networkCoordinatorLock = new Object();
    private final AtomicLong networkBootstrapGeneration = new AtomicLong();
    private final ReceiverUiOperation uiOperation = new ReceiverUiOperation();
    private String pendingUiStartRequestId = "";
    private String networkCoordinatorOwnerId = "";
    private String cleanupOwnerId = "";
    private long cleanupGeneration;
    private long currentOperationGeneration;
    private boolean cleanupCoordinatorClosed;
    private boolean cleanupFailed;
    private String cleanupError = "";
    private int latestDeliveredStartId;
    private final UiBinder uiBinder = new UiBinder();

    public final class UiBinder extends Binder {
        public UiSnapshot snapshot() { return getUiSnapshot(); }
        public List<UiInterface> interfaces() { return getUiInterfaces(); }
        public String start(String address, int prefixLength, String requestId) { return startFromUi(address, prefixLength, requestId); }
        public boolean stop(String ownerId) { return stopFromUi(ownerId); }
        public P5cDiscovery.SearchSnapshot startSearch(String expectedOwnerId) throws IOException {
            synchronized (networkCoordinatorLock) {
                return currentUiCoordinator(expectedOwnerId).startSearch();
            }
        }
        public P5cDiscovery.SearchSnapshot startAutomaticSearch(String expectedOwnerId) throws IOException {
            synchronized (networkCoordinatorLock) {
                return currentUiCoordinator(expectedOwnerId).startAutomaticSearch();
            }
        }
        public P5cDiscovery.SearchSnapshot stopSearch(String expectedOwnerId) throws IOException {
            synchronized (networkCoordinatorLock) {
                return currentUiCoordinator(expectedOwnerId).stopSearch();
            }
        }
        public String requestConnection(String candidateId, String expectedOwnerId) throws IOException {
            synchronized (networkCoordinatorLock) {
                return currentUiCoordinator(expectedOwnerId).requestConnection(candidateId);
            }
        }
    }

    public static final class UiSnapshot {
        public final String serviceState, bindingState, connectionMethod, playbackMode, versionName, networkAddress;
        public final String operationId, operationState, error;
        public final int versionCode, networkPort;
        public final boolean searchActive;
        public final List<P5cDiscovery.SearchCandidate> searchCandidates;
        public final String connectionRequestStatus;
        UiSnapshot(String serviceState, String bindingState, String connectionMethod, String playbackMode,
                String versionName, int versionCode, String operationId, String operationState, String error,
                String networkAddress, int networkPort, P5cDiscovery.SearchSnapshot search) {
            this.serviceState = serviceState; this.bindingState = bindingState;
            this.connectionMethod = connectionMethod; this.playbackMode = playbackMode;
            this.versionName = versionName; this.versionCode = versionCode;
            this.operationId = operationId; this.operationState = operationState; this.error = error;
            this.networkAddress = networkAddress; this.networkPort = networkPort;
            this.searchActive = search.active; this.searchCandidates = search.candidates;
            this.connectionRequestStatus = search.lastRequestStatus;
        }
    }

    public static final class UiInterface {
        public final String name, address;
        public final int prefixLength;
        UiInterface(String name, String address, int prefixLength) {
            this.name = name; this.address = address; this.prefixLength = prefixLength;
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        cleanupRetiredUpdateParts();
        activeService = this;
        foregroundReady = false;
        audioActive = false;
        actualMode = "";
        actualSampleRate = -1;
        actualChannels = -1;
        bufferCapacityFrames = -1;
        requestedBufferFrames = -1;
        acceptedBufferFrames = -1;
        effectiveBufferFrames = -1;
        startThresholdFrames = -1;
        preRollFrames = -1;
        underrunCount = -1;
        underrunIncrement = -1;
        tcpDroppedBacklogFrames = -1;
        startedAtElapsedMs = 0;
        startFailure = "";
        synchronized (STATUS_LOCK) { statusAudioId = 0; }
        networkAudioState = "stopped";
        networkAudioFailure = "";
        networkAudioAddress = "";
        networkAudioPort = 0;
        createNotificationChannel();
    }

    private void cleanupRetiredUpdateParts() {
        File directory = new File(getFilesDir(), "receiver-update");
        File[] parts = directory.listFiles((parent, name) -> name.endsWith(".part"));
        if (parts == null) return;
        for (File part : parts) {
            if (part.isFile() && !part.delete())
                android.util.Log.w("AnPodReceiver", "Could not remove retired update staging file: " + part.getName());
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        latestDeliveredStartId = Math.max(latestDeliveredStartId, startId);
        if (intent != null && ACTION_NOTIFICATION_STOP.equals(intent.getAction())) {
            String ownerId = intent.getStringExtra(EXTRA_UI_OWNER_ID);
            long generation = intent.getLongExtra(EXTRA_UI_GENERATION, -1L);
            if (uiOperation.owns(ownerId, generation, currentOperationGeneration))
                requestTermination(ownerId, generation, false, "");
            return START_NOT_STICKY;
        }
        if (intent != null && BootstrapActivity.ACTION_STOP.equals(intent.getAction())) {
            int disposition = uiOperation.bootstrapStopDisposition();
            if (disposition == ReceiverUiOperation.BOOTSTRAP_STOP_OWNER) {
                requestTermination(uiOperation.ownerId(), currentOperationGeneration, false, "");
            } else if (disposition == ReceiverUiOperation.BOOTSTRAP_STOP_IDLE) {
                stopSelf(startId);
            }
            return START_NOT_STICKY;
        }
        boolean uiNetworkAudio = intent != null && ACTION_UI_START_NETWORK_AUDIO.equals(intent.getAction());
        boolean networkAudio = intent != null && (BootstrapActivity.ACTION_START_NETWORK_AUDIO.equals(intent.getAction()) || uiNetworkAudio);
        boolean audio = intent != null && BootstrapActivity.ACTION_START_AUDIO.equals(intent.getAction());

        if (networkAudio && networkCoordinator != null) return START_NOT_STICKY;
        if (!audio && currentAudio != null) return START_NOT_STICKY;

        String ownerId;
        long generation;
        if (uiNetworkAudio) {
            ownerId = intent.getStringExtra(EXTRA_UI_OWNER_ID);
            generation = intent.getLongExtra(EXTRA_UI_GENERATION, -1L);
            String requestId = intent.getStringExtra(EXTRA_UI_REQUEST_ID);
            if (!uiOperation.owns(ownerId) || !ReceiverUiOperation.STARTING.equals(uiOperation.phase())
                    || generation != currentOperationGeneration || generation != networkBootstrapGeneration.get()
                    || !requestIdEqualsPending(requestId)) {
                stopRejectedStartIfIdle();
                return START_NOT_STICKY;
            }
        } else if (audio && currentAudio != null && (ReceiverUiOperation.RUNNING.equals(uiOperation.phase())
                || ReceiverUiOperation.STARTING.equals(uiOperation.phase()))) {
            ownerId = uiOperation.ownerId();
            generation = currentOperationGeneration;
        } else {
            if (!uiOperation.canBegin()) return START_NOT_STICKY;
            ownerId = UUID.randomUUID().toString();
            if (!uiOperation.begin(ownerId)) return START_NOT_STICKY;
            generation = networkBootstrapGeneration.incrementAndGet();
            currentOperationGeneration = generation;
            networkAudioState = "stopped";
            networkAudioFailure = "";
            networkAudioAddress = "";
            networkAudioPort = 0;
        }
        operationStartId = startId;

        // A management retry must not downgrade an active mediaPlayback FGS to
        // shortService or attach a short-service timeout to the audio session.
        try {
            Notification notification = serviceNotification(
                    audio ? "AnPod 音频连接正在运行。" : networkAudio ? "无线接收正在准备，可随时停止。" : "管理检查进行中；当前不播放音频。",
                    ownerId, generation, networkAudio || audio && currentAudio != null);
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
            requestTermination(ownerId, generation, true, startFailure);
            return START_NOT_STICKY;
        }

        if (audio) {
            if (timeout != null) handler.removeCallbacks(timeout);
            uiOperation.running(ownerId);
            startAudio(new AudioRequest(intent.getStringExtra("socketName"), intent.getStringExtra("connectCode"),
                    intent.getStringExtra("playbackMode")));
            return START_NOT_STICKY;
        }

        if (networkAudio) {
            if (uiNetworkAudio) {
                pendingUiStartRequestId = "";
            }
            if (timeout != null) handler.removeCallbacks(timeout);
            timeout = () -> requestTermination(ownerId, generation, true, "BootstrapTimeout");
            handler.postDelayed(timeout, networkBootstrapTimeoutMs(uiNetworkAudio));
            startNetworkBootstrap(intent.getStringExtra("networkAddress"), intent.getIntExtra("networkPrefixLength", 0), generation, ownerId);
            return START_NOT_STICKY;
        }

        if (timeout != null) handler.removeCallbacks(timeout);
        uiOperation.running(ownerId);
        timeout = () -> requestTermination(ownerId, generation, false, "");
        handler.postDelayed(timeout, MAX_RUN_MS);
        return START_NOT_STICKY;
    }

    private boolean requestIdEqualsPending(String requestId) {
        return requestId != null && requestId.equals(pendingUiStartRequestId);
    }

    private Notification serviceNotification(String text, String ownerId, long generation, boolean canStop) {
        int immutable = Build.VERSION.SDK_INT >= 23 ? PendingIntent.FLAG_IMMUTABLE : 0;
        PendingIntent content = mainActivityPendingIntent();
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, CHANNEL_ID) : new Notification.Builder(this);
        builder.setSmallIcon(android.R.drawable.stat_sys_headset)
                .setContentTitle("AnPod Receiver")
                .setContentText(text)
                .setContentIntent(content)
                .setOngoing(true);
        if (canStop && ownerId != null && !ownerId.isEmpty()) {
            Intent stop = new Intent(this, ReceiverService.class).setAction(ACTION_NOTIFICATION_STOP)
                    .setData(Uri.parse("anpod://receiver/stop/" + Uri.encode(ownerId) + "/" + generation))
                    .putExtra(EXTRA_UI_OWNER_ID, ownerId).putExtra(EXTRA_UI_GENERATION, generation);
            PendingIntent action = PendingIntent.getService(this, 2, stop,
                    PendingIntent.FLAG_UPDATE_CURRENT | immutable);
            builder.addAction(android.R.drawable.ic_media_pause, "停止接收", action);
        }
        return builder.build();
    }

    private PendingIntent mainActivityPendingIntent() {
        Intent open = new Intent(this, MainActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        int immutable = Build.VERSION.SDK_INT >= 23 ? PendingIntent.FLAG_IMMUTABLE : 0;
        return PendingIntent.getActivity(this, 1, open, PendingIntent.FLAG_UPDATE_CURRENT | immutable);
    }

    static long networkBootstrapTimeoutMs(boolean uiNetworkAudio) {
        return uiNetworkAudio ? MAX_RUN_MS : ADB_NETWORK_BOOTSTRAP_TIMEOUT_MS;
    }

    @Override
    public void onTimeout(int startId) {
        if (startId == operationStartId) requestTermination(uiOperation.ownerId(), currentOperationGeneration, true, "ShortServiceTimeout");
    }

    @Override
    public void onDestroy() {
        destroyed = true;
        if (activeService == this) activeService = null;
        foregroundReady = false;
        if (pendingAudio != null) pendingAudio.destroy();
        pendingAudio = null;
        AudioSession session = currentAudio;
        currentAudio = null;
        if (session != null) cancelAudio(session, "AudioServiceDestroyed");
        P5cReceiverCoordinator coordinator;
        synchronized (networkCoordinatorLock) {
            coordinator = networkCoordinator;
            networkCoordinator = null;
            networkCoordinatorOwnerId = "";
        }
        if (coordinator != null) coordinator.close();
        if (timeout != null) handler.removeCallbacks(timeout);
        if (session != null) clearAudioMetrics(session.id);
        synchronized (STATUS_LOCK) { if (session != null) startFailure = "AudioServiceDestroyed"; }
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE);
        else stopForeground(true);
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return uiBinder;
    }

    private UiSnapshot getUiSnapshot() {
        String binding = "empty";
        try { binding = new BindingStore(getFilesDir()).status().state; }
        catch (Exception error) { binding = "unavailable"; }
        String method = networkCoordinator != null || !networkAudioAddress.isEmpty() ? "wireless-udp"
                : currentAudio != null ? "usb-or-local" : "unknown";
        String state = ReceiverUiOperation.displayState(networkAudioState, currentAudio != null, audioActive);
        String mode = currentAudio == null ? "" : actualMode;
        return new UiSnapshot(state, binding, method, mode, BuildConfig.VERSION_NAME,
                BuildConfig.VERSION_CODE, uiOperation.ownerId(), uiOperation.phase(), uiOperation.error(),
                networkAudioAddress, networkAudioPort,
                networkCoordinator == null ? new P5cDiscovery.SearchSnapshot(false, Collections.emptyList(), "idle")
                        : networkCoordinator.searchSnapshot());
    }

    private P5cReceiverCoordinator currentUiCoordinator(String expectedOwnerId) throws IOException {
        P5cReceiverCoordinator coordinator = networkCoordinator;
        if (expectedOwnerId == null || expectedOwnerId.isEmpty()
                || !expectedOwnerId.equals(networkCoordinatorOwnerId)
                || !isCurrentOperation(expectedOwnerId, currentOperationGeneration))
            throw new IOException("接收会话已结束，请刷新状态后重试。");
        if (coordinator == null) throw new IOException("请先启动 Receiver 无线 UDP 接收。");
        return coordinator;
    }

    private List<UiInterface> getUiInterfaces() {
        ArrayList<UiInterface> result = new ArrayList<>();
        try {
            Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
            while (interfaces != null && interfaces.hasMoreElements()) {
                NetworkInterface network = interfaces.nextElement();
                if (!network.isUp() || network.isLoopback()) continue;
                for (java.net.InterfaceAddress item : network.getInterfaceAddresses()) {
                    InetAddress address = item.getAddress();
                    if (address instanceof Inet4Address && address.isSiteLocalAddress())
                        result.add(new UiInterface(network.getDisplayName(), address.getHostAddress(), item.getNetworkPrefixLength()));
                }
            }
        } catch (SocketException ignored) { }
        Collections.sort(result, Comparator.comparing((UiInterface item) -> item.name).thenComparing(item -> item.address));
        return result;
    }

    private String startFromUi(String address, int prefixLength, String requestId) {
        if (requestId == null || requestId.isEmpty()) return "启动请求无效，请重试。";
        if (!uiOperation.canBegin() || currentAudio != null || networkCoordinator != null) return "已有接收会话正在运行。";
        try {
            BindingStore.Status binding = new BindingStore(getFilesDir()).status();
            if (!"saved".equals(binding.state)) throw new IOException("请先通过 USB 完成电脑绑定。");
            InetAddress selected = parseIpv4(address);
            if (!isAssignedPrivateAddress(selected, prefixLength)) throw new IOException("所选网络接口已不可用，请重新选择。");
            String ownerId = UUID.randomUUID().toString();
            if (!uiOperation.begin(ownerId)) return "已有接收会话正在运行。";
            long generation = networkBootstrapGeneration.incrementAndGet();
            currentOperationGeneration = generation;
            pendingUiStartRequestId = requestId;
            networkAudioState = "starting";
            networkAudioFailure = "";
            startFailure = "";
            Intent intent = new Intent(this, ReceiverService.class).setAction(ACTION_UI_START_NETWORK_AUDIO)
                    .putExtra("networkAddress", address).putExtra("networkPrefixLength", prefixLength)
                    .putExtra(EXTRA_UI_REQUEST_ID, requestId).putExtra(EXTRA_UI_OWNER_ID, ownerId)
                    .putExtra(EXTRA_UI_GENERATION, generation);
            try {
                if (Build.VERSION.SDK_INT >= 26) startForegroundService(intent); else startService(intent);
            } catch (RuntimeException error) {
                requestTermination(ownerId, generation, true, error.getClass().getSimpleName());
                return error.getClass().getSimpleName();
            }
            return "";
        } catch (Exception error) {
            return error.getMessage() == null ? "无法开始接收。" : error.getMessage();
        }
    }

    private boolean stopFromUi(String ownerId) {
        if (!uiOperation.owns(ownerId) || !(ReceiverUiOperation.STARTING.equals(uiOperation.phase())
                || ReceiverUiOperation.RUNNING.equals(uiOperation.phase()))) return false;
        requestTermination(ownerId, currentOperationGeneration, false, "");
        return true;
    }

    private void requestTermination(String ownerId, long generation, boolean failed, String reason) {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            handler.post(() -> beginTermination(ownerId, generation, failed, reason));
        } else beginTermination(ownerId, generation, failed, reason);
    }

    private void beginTermination(String ownerId, long generation, boolean failed, String reason) {
        synchronized (networkCoordinatorLock) {
            if (!uiOperation.owns(ownerId) || generation != currentOperationGeneration
                    || generation != networkBootstrapGeneration.get() || !uiOperation.stop(ownerId)) return;
        }
        cleanupOwnerId = ownerId;
        cleanupGeneration = networkBootstrapGeneration.incrementAndGet();
        currentOperationGeneration = cleanupGeneration;
        cleanupFailed = failed;
        cleanupError = failed ? (reason == null || reason.isEmpty() ? "ReceiverOperationFailed" : reason) : "";
        cleanupCoordinatorClosed = true;
        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = null;
        networkRecoveryDeadlineElapsedMs = 0;
        if (pendingAudio != null) pendingAudio.destroy();
        pendingAudio = null;

        AudioSession session = currentAudio;
        if (session != null && ownerId.equals(session.ownerId))
            cancelAudio(session, session.request.udpContext != null ? "NetworkSessionStopped" : "UiSessionStopped");
        P5cReceiverCoordinator coordinator = null;
        synchronized (networkCoordinatorLock) {
            if (networkCoordinator != null && ownerId.equals(networkCoordinatorOwnerId)) {
                coordinator = networkCoordinator;
                networkCoordinator = null;
                networkCoordinatorOwnerId = "";
            }
        }
        if (coordinator != null) {
            P5cReceiverCoordinator closingCoordinator = coordinator;
            cleanupCoordinatorClosed = false;
            long terminatingGeneration = cleanupGeneration;
            new Thread(() -> {
                closingCoordinator.close();
                handler.post(() -> {
                    if (!isPendingCleanup(ownerId, terminatingGeneration)) return;
                    cleanupCoordinatorClosed = true;
                    finishTerminationIfReady(ownerId, terminatingGeneration);
                });
            }, "AnPod receiver cleanup").start();
        }
        networkAudioState = "stopping";
        networkAudioAddress = "";
        networkAudioPort = 0;
        finishTerminationIfReady(ownerId, cleanupGeneration);
    }

    private boolean isPendingCleanup(String ownerId, long generation) {
        return ownerId.equals(cleanupOwnerId) && generation == cleanupGeneration
                && generation == currentOperationGeneration && uiOperation.owns(ownerId)
                && ReceiverUiOperation.STOPPING.equals(uiOperation.phase());
    }

    private void finishTerminationIfReady(String ownerId, long generation) {
        if (!isPendingCleanup(ownerId, generation) || !cleanupCoordinatorClosed
                || (currentAudio != null && ownerId.equals(currentAudio.ownerId))) return;
        if (cleanupFailed) uiOperation.completeFailure(ownerId, cleanupError);
        else uiOperation.completeStop(ownerId);
        networkAudioState = cleanupFailed ? "failed" : "stopped";
        networkAudioFailure = cleanupFailed ? cleanupError : "";
        startFailure = networkAudioFailure;
        actualMode = "";
        audioActive = false;
        foregroundReady = false;
        startedAtElapsedMs = 0;
        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = null;
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE); else stopForeground(true);
        int stopId = uiOperation.startedServiceStopId(latestDeliveredStartId);
        if (stopId > 0) stopSelf(stopId);
    }

    private void stopRejectedStartIfIdle() {
        if (!uiOperation.shouldStopRejectedStart()) return;
        int stopId = uiOperation.startedServiceStopId(latestDeliveredStartId);
        if (stopId > 0) stopSelf(stopId);
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
            requestTermination(uiOperation.ownerId(), currentOperationGeneration, true, "InvalidAudioRequest");
            return null;
        }
        AudioSession session = new AudioSession(NEXT_AUDIO_ID.incrementAndGet(), request,
                uiOperation.ownerId(), networkBootstrapGeneration.get());
        currentAudio = session;
        synchronized (STATUS_LOCK) {
            statusAudioId = session.id;
            audioActive = false;
            actualMode = "";
            actualSampleRate = -1;
            actualChannels = -1;
            bufferCapacityFrames = -1;
            requestedBufferFrames = -1;
            acceptedBufferFrames = -1;
            effectiveBufferFrames = -1;
            startThresholdFrames = -1;
            preRollFrames = -1;
            underrunCount = -1;
            underrunIncrement = -1;
            tcpDroppedBacklogFrames = -1;
            startFailure = "";
        }
        if (request.udpContext != null) PCM_BYTES_WRITTEN.set(0);
        session.thread = new Thread(() -> receiveAudio(session), "AnPod audio receiver");
        session.thread.start();
        handler.post(() -> {
            if (currentAudio == session && isCurrentOperation(session.ownerId, session.networkGeneration)) {
                try {
                    Notification notification = serviceNotification("正在播放电脑音频。", session.ownerId,
                            session.networkGeneration, true);
                    if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION_ID, notification,
                            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
                    else startForeground(NOTIFICATION_ID, notification);
                } catch (RuntimeException error) {
                    requestTermination(session.ownerId, session.networkGeneration, true,
                            error.getClass().getSimpleName());
                }
            }
        });
        return session;
    }

    /** In-process hook for an authenticated session coordinator; no Intent or exported API calls it. */
    void startUdpAudio(UdpSessionContext context) {
        String ownerId = networkCoordinatorOwnerId;
        long generation = currentOperationGeneration;
        Runnable start = () -> {
            if (!isCurrentOperation(ownerId, generation)) context.destroy();
            else startAudio(AudioRequest.udp(context));
        };
        if (Looper.myLooper() == Looper.getMainLooper()) start.run();
        else if (!handler.post(start)) context.destroy();
    }

    private void startNetworkBootstrap(String address, int prefixLength, long generation, String ownerId) {
        try {
            if (!isCurrentOperation(ownerId, generation)) return;
            networkAudioState = "starting";
            networkAudioFailure = "";
            startFailure = "";
            networkRecoveryDeadlineElapsedMs = 0;
            InetAddress local = parseIpv4(address);
            if (!isAssignedPrivateAddress(local, prefixLength)) throw new IOException("InvalidReceiverInterface");
            PairingProtocol.BindingMaterial binding = new BindingStore(getFilesDir()).load();
            P5cReceiverCoordinator.PlaybackEndpoint playback = new P5cReceiverCoordinator.PlaybackEndpoint() {
                private AudioSession ownedSession;
                private boolean current() { return isCurrentOperation(ownerId, generation); }

                @Override public boolean supportsPlaybackProgress() { return true; }

                @Override public byte[] playbackProgressPayload() {
                    return ReceiverService.playbackProgressPayload();
                }

                @Override public boolean start(UdpSessionContext context,
                        P5cReceiverCoordinator.StartPermission permission) throws IOException {
                    AudioStartTicket ticket = new AudioStartTicket();
                    boolean posted = handler.post(() -> {
                        synchronized (ticket) {
                            try {
                                if (ticket.cancelled || !current() || currentAudio != null || pendingAudio != null
                                        || !permission.allowed()) {
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
                                networkRecoveryDeadlineElapsedMs = 0;
                                networkAudioFailure = "";
                                networkAudioState = "receiverRunning";
                                AudioSession accepted = startAudio(AudioRequest.udp(context));
                                if (accepted == null || accepted.request.udpContext != context || currentAudio != accepted) {
                                    throw new IOException("NetworkAudioSessionNotAccepted");
                                }
                                ticket.accepted = accepted;
                                ownedSession = accepted;
                                if (timeout != null) handler.removeCallbacks(timeout);
                                timeout = null;
                                foregroundReady = true;
                            } catch (IOException | RuntimeException error) {
                                if (current()) requestTermination(ownerId, generation, true,
                                        error instanceof IOException ? error.getMessage() : error.getClass().getSimpleName());
                                context.destroy();
                            } finally { ticket.finished.countDown(); }
                        }
                    });
                    if (!posted) { context.destroy(); return false; }
                    try {
                        if (!ticket.finished.await(3, TimeUnit.SECONDS)) {
                            ticket.cancelled = true;
                            requestTermination(ownerId, generation, true, "NetworkStartTimedOut");
                            return false;
                        }
                    } catch (InterruptedException interrupted) {
                        Thread.currentThread().interrupt();
                        ticket.cancelled = true;
                        requestTermination(ownerId, generation, true, "NetworkStartInterrupted");
                        return false;
                    }
                    return ticket.accepted != null && currentAudio == ticket.accepted
                            && ticket.accepted.request.udpContext == context && current();
                }

                @Override public void pause() throws IOException {
                    runOnMainForOwner(ownerId, generation, () -> {
                        setUdpAudioPaused(ownedSession, true);
                        networkAudioState = "paused";
                    });
                }

                @Override public void resume() throws IOException {
                    runOnMainForOwner(ownerId, generation, () -> {
                        setUdpAudioPaused(ownedSession, false);
                        networkAudioState = "receiverRunning";
                    });
                }

                @Override public void stop() {
                    try {
                        runOnMainForOwner(ownerId, generation, () -> {
                            AudioSession session = ownedSession;
                            if (session != null && currentAudio == session) cancelAudio(session, "NetworkSessionStopped");
                            networkRecoveryDeadlineElapsedMs = android.os.SystemClock.elapsedRealtime() + 30_000L;
                            networkAudioState = "prepared";
                        });
                    } catch (IOException error) {
                        postOperationError(ownerId, generation, error.getMessage());
                    }
                }

                @Override public boolean streaming() throws IOException {
                    if (!current()) throw new IOException("NetworkSessionStopped");
                    if ("failed".equals(networkAudioState)) throw new IOException(networkAudioFailure.isEmpty()
                            ? "NetworkAudioWorkerFailed" : networkAudioFailure);
                    AudioSession session = ownedSession;
                    if (session == null || currentAudio != session) throw new IOException("NetworkAudioSessionNoLongerOwned");
                    boolean streaming = session.isStreamingAt(System.nanoTime());
                    postOperationState(ownerId, generation, streaming ? "streaming" : "receiverRunning");
                    return streaming;
                }

                @Override public void shutdown() {
                    // The waiter owns coordinator termination and checks this generation on the main thread.
                }
            };
            P5cReceiverCoordinator coordinator = P5cReceiverCoordinator.openOnInterface(binding, playback, local,
                    prefixLength, BuildConfig.VERSION_CODE);
            boolean installed;
            synchronized (networkCoordinatorLock) {
                installed = isCurrentOperation(ownerId, generation);
                if (installed) {
                    networkCoordinator = coordinator;
                    networkCoordinatorOwnerId = ownerId;
                }
            }
            if (!installed) { coordinator.close(); return; }
            networkAudioAddress = coordinator.localAddress().getHostAddress();
            networkAudioPort = coordinator.port();
            coordinator.start();
            coordinator.startDiscovery(discoverySocketBinder(local, prefixLength));
            uiOperation.running(ownerId);
            networkAudioState = "prepared";
            Thread waiter = new Thread(() -> {
                try {
                    while (true) {
                        boolean completed = coordinator.await(250, TimeUnit.MILLISECONDS);
                        if (!isCurrentOperation(ownerId, generation)) return;
                        if (completed) {
                            handler.post(() -> handleCoordinatorEnded(coordinator, ownerId, generation));
                            return;
                        }
                        long recoveryDeadline = networkRecoveryDeadlineElapsedMs;
                        if (recoveryDeadline != 0 && currentAudio == null
                                && android.os.SystemClock.elapsedRealtime() >= recoveryDeadline) {
                            requestTermination(ownerId, generation, true, "RecoveryTimeout");
                            return;
                        }
                    }
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                }
            }, "AnPod P5-C bootstrap timeout");
            waiter.setDaemon(true);
            waiter.start();
        } catch (Exception error) {
            if (!isCurrentOperation(ownerId, generation)) return;
            String reason = error instanceof IOException ? error.getMessage() : error.getClass().getSimpleName();
            recordNetworkFailure(reason);
            requestTermination(ownerId, generation, true, networkAudioFailure);
        }
    }

    private void handleCoordinatorEnded(P5cReceiverCoordinator coordinator, String ownerId, long generation) {
        if (!isCurrentOperation(ownerId, generation) || networkCoordinator != coordinator
                || !ownerId.equals(networkCoordinatorOwnerId)) return;
        String coordinatorFailure = coordinator.failure();
        boolean failed = !coordinatorFailure.isEmpty() || "failed".equals(networkAudioState) || !networkAudioFailure.isEmpty();
        String reason = !coordinatorFailure.isEmpty() ? coordinatorFailure : networkAudioFailure;
        requestTermination(ownerId, generation, failed, reason);
    }

    private interface MainOwnerCommand { void run() throws IOException; }

    private boolean isCurrentOperation(String ownerId, long generation) {
        String phase = uiOperation.phase();
        return !destroyed && uiOperation.owns(ownerId) && currentOperationGeneration == generation
                && networkBootstrapGeneration.get() == generation
                && (ReceiverUiOperation.STARTING.equals(phase) || ReceiverUiOperation.RUNNING.equals(phase));
    }

    private void runOnMainForOwner(String ownerId, long generation, MainOwnerCommand command) throws IOException {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            if (!isCurrentOperation(ownerId, generation)) throw new IOException("NetworkSessionStopped");
            command.run();
            return;
        }
        CountDownLatch completed = new CountDownLatch(1);
        AtomicBoolean cancelled = new AtomicBoolean();
        AtomicReference<IOException> failure = new AtomicReference<>();
        boolean posted = handler.post(() -> {
            try {
                if (cancelled.get() || !isCurrentOperation(ownerId, generation)) throw new IOException("NetworkSessionStopped");
                command.run();
            } catch (IOException error) { failure.set(error); }
            finally { completed.countDown(); }
        });
        if (!posted) throw new IOException("ReceiverServiceUnavailable");
        try {
            if (!completed.await(3, TimeUnit.SECONDS)) {
                cancelled.set(true);
                throw new IOException("ReceiverMainThreadTimedOut");
            }
        } catch (InterruptedException interrupted) {
            cancelled.set(true);
            Thread.currentThread().interrupt();
            throw new IOException("ReceiverMainThreadInterrupted", interrupted);
        }
        if (failure.get() != null) throw failure.get();
    }

    private void postOperationState(String ownerId, long generation, String state) {
        handler.post(() -> {
            if (!isCurrentOperation(ownerId, generation)) return;
            if ("streaming".equals(state) || ("receiverRunning".equals(state) && "streaming".equals(networkAudioState)))
                networkAudioState = state;
        });
    }

    private void postOperationError(String ownerId, long generation, String error) {
        handler.post(() -> {
            if (!isCurrentOperation(ownerId, generation)) return;
            requestTermination(ownerId, generation, true, error == null ? "NetworkAudioWorkerFailed" : error);
        });
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

    private P5cDiscovery.SocketBinder discoverySocketBinder(InetAddress address, int prefixLength) throws IOException {
        NetworkInterface selected = NetworkInterface.getByInetAddress(address);
        ConnectivityManager connectivity = (ConnectivityManager) getSystemService(CONNECTIVITY_SERVICE);
        if (selected == null || connectivity == null) throw new IOException("所选网络接口已不可用，请重新选择。");
        for (Network network : connectivity.getAllNetworks()) {
            LinkProperties properties = connectivity.getLinkProperties(network);
            if (properties == null || !selected.getName().equals(properties.getInterfaceName())) continue;
            for (LinkAddress link : properties.getLinkAddresses()) {
                if (address.equals(link.getAddress()) && link.getPrefixLength() == prefixLength)
                    return network::bindSocket;
            }
        }
        throw new IOException("无法将发现 socket 绑定到所选 Android 网络。");
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

    private static void recordNetworkFailure(String reason) {
        networkAudioFailure = reason == null ? "NetworkStartFailed" : reason;
        networkAudioState = "failed";
        startFailure = networkAudioFailure;
    }

    /** Internal pause hook: drop pending and incoming history without adding a control datagram. */
    private void setUdpAudioPaused(AudioSession session, boolean paused) throws IOException {
        if (session == null || currentAudio != session || session.request.udpContext == null)
            throw new IOException("NetworkAudioSessionNotOwned");
        synchronized (session) {
            ensureRunning(session);
            session.udpPaused = paused;
            if (session.udpQueue != null) session.udpQueue.setPaused(paused);
        }
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
                    udpQueue = new UdpPcmQueue(session.request.udpContext.parameters,
                            session.request.udpContext.playbackMode);
                    udpReceiver = new UdpReceiver(session.request.udpContext, udpQueue);
                    UdpReceiver receiverForThread = udpReceiver;
                    networkThread = new Thread(() -> {
                        Process.setThreadPriority(Process.THREAD_PRIORITY_DEFAULT);
                        receiverForThread.run();
                    }, "AnPod UDP packet receiver");
                    synchronized (session) {
                        ensureRunning(session);
                        session.udpQueue = udpQueue;
                        udpQueue.setPaused(session.udpPaused);
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
                int acceptedBufferFrames = -1;
                int requestedStartFrames = -1;
                boolean startThresholdRequestLimited = false;
                String configuredMode;
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
                    session.audioTrack = track;
                    session.trackWrittenFrames = 0;
                    session.playbackActivity.resetPlaybackHead(track.getPlaybackHeadPosition());
                    capacityFrames = Build.VERSION.SDK_INT >= 24
                            ? track.getBufferCapacityInFrames() : track.getBufferSizeInFrames();
                    if ("lowlatency".equalsIgnoreCase(session.request.playbackMode)) {
                        if (Build.VERSION.SDK_INT >= 24 && capacityFrames > 0) {
                            int desired = Math.max(1,
                                    sampleRate * Pcm16FrameStream.LOW_LATENCY_AUDIO_TRACK_BUFFER_MS / 1000);
                            requestedBufferFrames = Pcm16FrameStream.requestedAudioTrackBufferFrames(
                                    sampleRate, capacityFrames, true);
                            acceptedBufferFrames = track.setBufferSizeInFrames(requestedBufferFrames);
                            if (acceptedBufferFrames < 0) configuredMode = "balanced-fallback";
                            else {
                                configuredMode = acceptedBufferFrames == requestedBufferFrames
                                        && capacityFrames >= desired ? "low-latency" : "low-latency-limited";
                                if (Build.VERSION.SDK_INT >= 31 && acceptedBufferFrames > 0) {
                                    requestedStartFrames = Math.min(acceptedBufferFrames,
                                            Math.max(1, sampleRate * Pcm16FrameStream.LOW_LATENCY_START_THRESHOLD_MS / 1000));
                                    int acceptedStartFrames = track.setStartThresholdInFrames(requestedStartFrames);
                                    startThresholdRequestLimited = acceptedStartFrames < 0
                                            || acceptedStartFrames != requestedStartFrames;
                                } else if (Build.VERSION.SDK_INT >= 31) startThresholdRequestLimited = true;
                            }
                        } else configuredMode = "balanced-fallback";
                    } else if (Build.VERSION.SDK_INT >= 24 && capacityFrames > 0) {
                        int desired = Math.max(1, sampleRate * Pcm16FrameStream.BALANCED_AUDIO_TRACK_BUFFER_MS / 1000);
                        requestedBufferFrames = Pcm16FrameStream.requestedAudioTrackBufferFrames(
                                sampleRate, capacityFrames, false);
                        acceptedBufferFrames = track.setBufferSizeInFrames(requestedBufferFrames);
                        configuredMode = acceptedBufferFrames < 0 ? "balanced-fallback"
                                : capacityFrames >= desired && acceptedBufferFrames == requestedBufferFrames
                                        ? "balanced" : "balanced-limited";
                    } else configuredMode = "balanced-system";
                    int effectiveFrames = track.getBufferSizeInFrames();
                    int thresholdFrames = Build.VERSION.SDK_INT >= 31 ? track.getStartThresholdInFrames() : -1;
                    if (configuredMode.startsWith("low-latency")
                            && (effectiveFrames != requestedBufferFrames
                                    || (requestedStartFrames > 0 && thresholdFrames != requestedStartFrames)
                                    || startThresholdRequestLimited)) {
                        configuredMode = "low-latency-limited";
                    } else if (configuredMode.equals("balanced") && effectiveFrames != requestedBufferFrames) {
                        configuredMode = "balanced-limited";
                    }
                    publishMode(session, configuredMode);
                }

                byte[] buffer = new byte[Math.max(frameBytes, minBuffer + frameBytes - 1)];
                Pcm16FrameStream pcm = new Pcm16FrameStream(channels);
                int effectiveFrames = track.getBufferSizeInFrames();
                int thresholdFrames = Build.VERSION.SDK_INT >= 31 ? track.getStartThresholdInFrames() : -1;
                int preRollFrames = currentMode(session).startsWith("low-latency") && thresholdFrames > 0
                        ? Math.min(effectiveFrames, thresholdFrames) : Math.min(effectiveFrames, minBufferFrames);
                publishMetrics(session, track.getSampleRate(), track.getChannelCount(), capacityFrames, requestedBufferFrames,
                        acceptedBufferFrames, effectiveFrames, thresholdFrames,
                        Build.VERSION.SDK_INT >= 24 ? track.getUnderrunCount() : -1, preRollFrames, currentMode(session));
                if (udpQueue != null) {
                    udpQueue.configureAudioTrack(preRollFrames);
                } else {
                    int backlogLimitFrames = Pcm16FrameStream.backlogLimitFrames(
                            track.getSampleRate(), preRollFrames, currentMode(session).startsWith("low-latency"));
                    pcm.setBacklogLimit(backlogLimitFrames,
                            (track.getSampleRate() * 3 + 999) / 1000);
                    synchronized (STATUS_LOCK) {
                        if (statusAudioId == session.id) tcpDroppedBacklogFrames = 0;
                    }
                }
                int preRollBytes = preRollFrames * frameBytes;
                int preRolled = 0;
                while (preRolled < preRollBytes) {
                    ensureRunning(session);
                    int count = pcm.readFrames(input, buffer, Math.min(buffer.length, preRollBytes - preRolled));
                    if (count < 0) throw new IOException("AudioStreamEndedBeforePlayback");
                    publishTcpBacklogMetrics(session, udpQueue, pcm);
                    writeFrames(session, track, pcm, buffer, count, frameBytes);
                    preRolled += count;
                }
                publishPreRollFrames(session, preRolled / frameBytes);

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
                    session.trackPlaying = true;
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
                    publishTcpBacklogMetrics(session, udpQueue, pcm);
                    writeFrames(session, track, pcm, buffer, count, frameBytes);
                    if (Build.VERSION.SDK_INT >= 24) {
                        int underruns = track.getUnderrunCount();
                        if (udpQueue != null) udpQueue.reportTrackUnderruns(underruns);
                        synchronized (STATUS_LOCK) {
                            if (statusAudioId == session.id) {
                                underrunIncrement = underrunCount < 0 || underruns < underrunCount
                                        ? -1 : underruns - underrunCount;
                                underrunCount = underruns;
                            }
                        }
                    }
                }
            }
        } catch (IOException | RuntimeException error) {
            failure = session.cancelled ? session.stopReason : describeFailure(error);
        } finally {
            session.trackPlaying = false;
            if (session.audioTrack == track) session.audioTrack = null;
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
            handler.post(() -> finishAudio(session, finalFailure));
        }
    }

    private void writeFrames(AudioSession session, AudioTrack track, Pcm16FrameStream pcm, byte[] buffer, int count, int frameBytes) throws IOException {
        pcm.writeFramesForChannels((data, offset, length) -> {
            synchronized (session) {
                ensureRunning(session);
                int written = track.write(data, offset, length, AudioTrack.WRITE_BLOCKING);
                if (written <= 0) throw new IOException("AudioTrack.write failed: " + written);
                session.trackWrittenFrames += written / frameBytes;
                if (session.request.udpContext != null) {
                    PCM_BYTES_WRITTEN.addAndGet(written);
                    session.playbackActivity.noteSuccessfulWrite(System.nanoTime());
                }
                return written;
            }
        }, buffer, count);
        if (count % frameBytes != 0) throw new IOException("PCM frame alignment failed");
    }

    private void finishAudio(AudioSession session, String failure) {
        if (currentAudio != session) return;
        currentAudio = null;
        clearAudioMetrics(session.id);
        String phase = uiOperation.phase();
        if (pendingAudio != null && !destroyed && (ReceiverUiOperation.STARTING.equals(phase)
                || ReceiverUiOperation.RUNNING.equals(phase)) && uiOperation.owns(session.ownerId)) {
            AudioRequest next = pendingAudio;
            pendingAudio = null;
            startAudio(next);
            return;
        }
        if (ReceiverUiOperation.STOPPING.equals(uiOperation.phase()) && uiOperation.owns(session.ownerId)) {
            finishTerminationIfReady(session.ownerId, currentOperationGeneration);
            return;
        }
        if (!uiOperation.owns(session.ownerId) || session.networkGeneration != currentOperationGeneration) return;
        if (session.request.udpContext != null && "NetworkSessionStopped".equals(failure)) {
            networkAudioState = networkCoordinator != null ? "prepared" : "stopped";
            if (networkCoordinator != null) networkRecoveryDeadlineElapsedMs = android.os.SystemClock.elapsedRealtime() + 30_000L;
            else requestTermination(session.ownerId, currentOperationGeneration, false, "");
            return;
        }
        if (session.request.udpContext != null && networkCoordinator != null) {
            recordNetworkFailure(failure.isEmpty() ? "NetworkAudioSessionEnded" : failure);
            uiOperation.reportError(session.ownerId, networkAudioFailure);
            networkRecoveryDeadlineElapsedMs = android.os.SystemClock.elapsedRealtime() + 30_000L;
            return;
        }
        boolean failed = failure != null && !failure.isEmpty() && !"AudioSessionEnded".equals(failure);
        requestTermination(session.ownerId, currentOperationGeneration, failed, failed ? failure : "");
    }

    private static void clearAudioMetrics(long audioId) {
        synchronized (STATUS_LOCK) {
            if (statusAudioId != audioId) return;
            statusAudioId = 0;
            audioActive = false;
            actualMode = "";
            actualSampleRate = -1;
            actualChannels = -1;
            bufferCapacityFrames = -1;
            requestedBufferFrames = -1;
            acceptedBufferFrames = -1;
            effectiveBufferFrames = -1;
            startThresholdFrames = -1;
            preRollFrames = -1;
            underrunCount = -1;
            underrunIncrement = -1;
            tcpDroppedBacklogFrames = -1;
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

    private void publishMetrics(AudioSession session, int rate, int channels, int capacity, int requested, int accepted,
            int effective, int threshold, int undrruns, int preRoll, String mode) {
        synchronized (STATUS_LOCK) {
            if (statusAudioId != session.id) return;
            actualSampleRate = rate;
            actualChannels = channels;
            bufferCapacityFrames = capacity;
            requestedBufferFrames = requested;
            acceptedBufferFrames = accepted;
            effectiveBufferFrames = effective;
            startThresholdFrames = threshold;
            preRollFrames = preRoll;
            underrunCount = undrruns;
            actualMode = mode;
        }
    }

    private void publishPreRollFrames(AudioSession session, int frames) {
        synchronized (STATUS_LOCK) {
            if (statusAudioId == session.id) preRollFrames = frames;
        }
    }

    private static long trackPendingFrames(AudioSession session, AudioTrack track) {
        if (track == null) return -1;
        try {
            long played = session.playbackActivity.playedFrames(track.getPlaybackHeadPosition());
            return Pcm16FrameStream.pendingFrames(session.trackWrittenFrames, played);
        } catch (RuntimeException unavailable) {
            return -1;
        }
    }

    private void publishMode(AudioSession session, String mode) {
        synchronized (STATUS_LOCK) { if (statusAudioId == session.id) actualMode = mode; }
    }

    static PlaybackMetrics playbackMetricsSnapshot() {
        AudioSession session = currentAudio;
        if (session == null) return PlaybackMetrics.unknown();
        UdpPcmQueue queue = session.udpQueue;
        AudioTrack track = session.audioTrack;
        int jitter = queue == null ? -1 : queue.targetFrames();
        long expired = queue == null ? -1 : queue.expiredFrames();
        long queued = queue == null ? -1 : queue.queuedFrames();
        long pending = trackPendingFrames(session, track);
        Boolean advancing = null;
        if (track != null) {
            try { advancing = session.playbackActivity.recentlyAdvanced(System.nanoTime(), track.getPlaybackHeadPosition()); }
            catch (RuntimeException unavailable) { advancing = null; }
        }
        synchronized (STATUS_LOCK) {
            if (currentAudio != session || statusAudioId != session.id || session.audioTrack != track)
                return PlaybackMetrics.unknown();
            return new PlaybackMetrics(actualSampleRate, actualChannels, jitter,
                    expired, queued, pending, underrunCount, tcpDroppedBacklogFrames,
                    advancing, requestedBufferFrames, acceptedBufferFrames, effectiveBufferFrames,
                    bufferCapacityFrames, startThresholdFrames, preRollFrames, queue != null);
        }
    }

    static byte[] playbackProgressPayload() {
        PlaybackMetrics metrics = playbackMetricsSnapshot();
        return P5cControlProtocol.playbackProgressPayload(metrics.sampleRate, metrics.channels,
                metrics.jitterTargetFrames, metrics.expiredFrames, metrics.queuedFrames, metrics.pendingFrames,
                metrics.underrunCount, metrics.tcpDroppedFrames, metrics.advancing, metrics.requestedFrames,
                metrics.acceptedFrames, metrics.effectiveFrames, metrics.capacityFrames,
                metrics.startThresholdFrames, metrics.preRollFrames);
    }

    static boolean udpQueueActive() {
        AudioSession session = currentAudio;
        return session != null && session.udpQueue != null;
    }

    private void publishTcpBacklogMetrics(AudioSession session, UdpPcmQueue udpQueue,
            Pcm16FrameStream pcm) {
        if (udpQueue != null) return;
        synchronized (STATUS_LOCK) {
            if (statusAudioId == session.id) tcpDroppedBacklogFrames = pcm.getDroppedBacklogFrames();
        }
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
        final String ownerId;
        final long networkGeneration;
        volatile Thread thread;
        volatile Thread networkThread;
        volatile LocalSocket socket;
        volatile UdpReceiver udpReceiver;
        volatile UdpPcmQueue udpQueue;
        final UdpPlaybackActivity playbackActivity = new UdpPlaybackActivity();
        volatile AudioTrack audioTrack;
        volatile long trackWrittenFrames;
        volatile boolean trackPlaying;
        volatile boolean udpPaused;
        volatile boolean cancelled;
        volatile String stopReason = "";

        AudioSession(long id, AudioRequest request, String ownerId, long networkGeneration) {
            this.id = id;
            this.request = request;
            this.ownerId = ownerId;
            this.networkGeneration = networkGeneration;
        }

        boolean isStreamingAt(long nowNanos) throws IOException {
            if (cancelled || udpPaused || !trackPlaying) return false;
            UdpPcmQueue queue = udpQueue;
            AudioTrack track = audioTrack;
            if (queue == null || track == null) return false;
            int playbackHead;
            try {
                playbackHead = track.getPlaybackHeadPosition();
            } catch (IllegalStateException error) {
                if (audioTrack != track || cancelled) return false;
                throw new IOException("AudioTrack playback position unavailable", error);
            }
            return playbackActivity.isActive(nowNanos, playbackHead, queue.hasAuthenticatedAudio(),
                    queue.lastAuthenticatedAudioAtNanos(), udpPaused);
        }
    }
}
