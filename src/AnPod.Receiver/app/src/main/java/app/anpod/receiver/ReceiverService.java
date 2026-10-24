package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.app.ActivityManager;
import android.content.Intent;
import android.content.BroadcastReceiver;
import android.content.IntentFilter;
import android.hardware.usb.UsbAccessory;
import android.hardware.usb.UsbManager;
import android.content.pm.ServiceInfo;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioTrack;
import android.net.LocalSocket;
import android.net.LocalSocketAddress;
import android.net.wifi.WifiManager;
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
import com.anpod.server.FloatPcmFrameQueue;
import com.anpod.server.FloatPcmDriftController;
import com.anpod.server.Pcm24PackedCodec;

import java.io.DataInputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.security.MessageDigest;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.List;
import java.util.UUID;

/**
 * Android playback receiver adapted from ysbing/AudioShare's server Main.java
 * (LGPL-3.0-or-later). The protocol and AudioTrack streaming path are shared
 * with the app_process implementation; see upstream/PATCHES.md for changes.
 */
public final class ReceiverService extends Service {
    private static final Object OBOE_OUTPUT_LOCK = new Object();
    private static String pendingOboeFallbackReason = "";
    private static OboePlaybackOutput deferredOboeClose;
    public static final String ACTION_UI_START_NETWORK_AUDIO = "app.anpod.receiver.action.UI_START_NETWORK_AUDIO";
    public static final String ACTION_UI_START_AOA_AUDIO = "app.anpod.receiver.action.UI_START_AOA_AUDIO";
    private static final String ACTION_NOTIFICATION_STOP = "app.anpod.receiver.action.NOTIFICATION_STOP";
    static final String ACTION_BACKGROUND_AUTH_DECISION = "app.anpod.receiver.action.BACKGROUND_AUTH_DECISION";
    private static final String EXTRA_UI_REQUEST_ID = "uiRequestId";
    private static final String EXTRA_AOA_ACCESSORY = "aoaAccessory";
    static final String CHANNEL_ID = "receiver-management";
    private static final int NOTIFICATION_ID = 1;
    static final int AUTH_NOTIFICATION_ID = 2;
    private static final long ADB_NETWORK_BOOTSTRAP_TIMEOUT_MS = 25_000L;
    private static final String EXTRA_UI_OWNER_ID = "uiOwnerId";
    private static final String EXTRA_UI_GENERATION = "uiGeneration";
    private static final String EXTRA_BACKGROUND_STANDBY = "backgroundStandby";
    private static final String EXTRA_KEEP_RECEIVE_INTENT = "keepReceiveIntent";
    static final String EXTRA_AUTH_OWNER = "authOwner";
    static final String EXTRA_AUTH_GENERATION = "authGeneration";
    static final String EXTRA_AUTH_SESSION = "authSession";
    static final String EXTRA_AUTH_REQUEST = "authRequest";
    static final String EXTRA_AUTH_PEER = "authPeer";
    static final String EXTRA_AUTH_BINDING = "authBinding";
    static final String EXTRA_AUTH_DEADLINE = "authDeadline";
    static final String EXTRA_AUTH_DECISION = "authDecision";
    private static final long MAX_RUN_MS = 120_000L;
    private static final long AOA_RECOVERY_WINDOW_MS = 30_000L;
    private static final long[] AOA_RECOVERY_DELAYS_MS = {0, 1_000, 2_000, 5_000};
    private static final Object STATUS_LOCK = new Object();
    private static volatile ReceiverService activeService;
    private static final AtomicLong NEXT_AUDIO_ID = new AtomicLong();
    static final AtomicLong PCM_BYTES_WRITTEN = new AtomicLong();
    private static final PcmWaveformTap WAVEFORM = new PcmWaveformTap();

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
    static volatile String operationOwnerId = "";
    static volatile long operationGeneration = -1;
    static volatile String terminalCause = "";
    static volatile String terminalOwnerId = "";
    static volatile long terminalGeneration = -1;
    private static long statusAudioId;


    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable timeout;
    static volatile AudioSession currentAudio;
    private AudioRequest pendingAudio;
    private int operationStartId;
    volatile boolean destroyed;
    private volatile long networkRecoveryDeadlineElapsedMs;
    volatile P5cReceiverCoordinator networkCoordinator;
    final Object networkCoordinatorLock = new Object();
    private final AtomicLong networkBootstrapGeneration = new AtomicLong();
    final ReceiverUiOperation uiOperation = new ReceiverUiOperation();
    private String pendingUiStartRequestId = "";
    private String pendingAoaRequestId = "";
    private UsbAccessory pendingAoaAccessory;
    private boolean pendingUiStandbyStart;
    private boolean pendingUiReceiveIntent;
    String networkCoordinatorOwnerId = "";
    private String cleanupOwnerId = "";
    private long cleanupGeneration;
    long currentOperationGeneration;
    private boolean cleanupCoordinatorClosed;
    private boolean cleanupFailed;
    private String cleanupError = "";
    private int latestDeliveredStartId;
    volatile AoaReceiverCoordinator aoaCoordinator;
    volatile UsbAccessory aoaAccessory;
    volatile String aoaOwnerId = "";
    volatile long aoaOwnerGeneration;
    private volatile Thread aoaBootstrapThread;
    private volatile long aoaRecoveryDeadlineElapsedMs;
    private volatile int aoaRecoveryAttempt;
    private volatile String aoaRecoveryFailure = "";
    private BroadcastReceiver aoaDetachReceiver;
    final ReceiverBackgroundAuthorization authorization = new ReceiverBackgroundAuthorization(this);
    private final ReceiverUiBinder uiBinder = new ReceiverUiBinder(this);
    final ReceiverUiState uiState = new ReceiverUiState(this);
    volatile boolean backgroundStandbyEnabled;
    private volatile boolean receiveIntentEnabled;
    BackgroundStartAuthorization backgroundAuthorization;




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
        backgroundAuthorization = new BackgroundStartAuthorization(
                android.os.SystemClock::elapsedRealtime, new BindingStore(getFilesDir()),
                pending -> handler.post(() -> authorization.update(pending)));
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
        if (intent != null && ACTION_BACKGROUND_AUTH_DECISION.equals(intent.getAction())) {
            authorization.handleDecision(intent, startId);
            return START_NOT_STICKY;
        }
        if (intent != null && ACTION_NOTIFICATION_STOP.equals(intent.getAction())) {
            String ownerId = intent.getStringExtra(EXTRA_UI_OWNER_ID);
            long generation = intent.getLongExtra(EXTRA_UI_GENERATION, -1L);
            if (uiOperation.owns(ownerId, generation, currentOperationGeneration)) {
                recordManualStop(ownerId, generation);
                requestTermination(ownerId, generation, false, "");
            }
            return START_NOT_STICKY;
        }
        if (intent != null && BootstrapActivity.ACTION_STOP.equals(intent.getAction())) {
            // The invisible trampoline starts us with startForegroundService even when the session
            // just ended; satisfy the 5 s contract before tearing the service down.
            try {
                Notification stopping = serviceNotification("正在停止 AnPod 接收…", "", 0, false);
                if (Build.VERSION.SDK_INT >= 34) {
                    startForeground(NOTIFICATION_ID, stopping, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE);
                } else {
                    startForeground(NOTIFICATION_ID, stopping);
                }
            } catch (RuntimeException ignored) {
                // Termination below still runs.
            }
            int disposition = uiOperation.bootstrapStopDisposition();
            if (disposition == ReceiverUiOperation.BOOTSTRAP_STOP_OWNER) {
                String ownerId = uiOperation.ownerId();
                long generation = currentOperationGeneration;
                if (uiOperation.owns(ownerId, generation, networkBootstrapGeneration.get())) {
                    recordManualStop(ownerId, generation);
                    requestTermination(ownerId, generation, false, "");
                }
            } else if (disposition == ReceiverUiOperation.BOOTSTRAP_STOP_IDLE) {
                stopSelf(startId);
            }
            return START_NOT_STICKY;
        }
        boolean uiNetworkAudio = intent != null && ACTION_UI_START_NETWORK_AUDIO.equals(intent.getAction());
        boolean uiAoaAudio = intent != null && ACTION_UI_START_AOA_AUDIO.equals(intent.getAction());
        boolean requestedBackgroundStandby = uiNetworkAudio
                && intent.getBooleanExtra(EXTRA_BACKGROUND_STANDBY, false);
        boolean requestedKeepReceiveIntent = uiNetworkAudio
                && intent.getBooleanExtra(EXTRA_KEEP_RECEIVE_INTENT, false);
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
                    || !requestIdEqualsPending(requestId)
                    || requestedBackgroundStandby != pendingUiStandbyStart
                    || requestedKeepReceiveIntent != pendingUiReceiveIntent) {
                stopRejectedStartIfIdle();
                return START_NOT_STICKY;
            }
        } else if (uiAoaAudio) {
            ownerId = intent.getStringExtra(EXTRA_UI_OWNER_ID);
            generation = intent.getLongExtra(EXTRA_UI_GENERATION, -1L);
            String requestId = intent.getStringExtra(EXTRA_UI_REQUEST_ID);
            UsbAccessory requestedAccessory = usbAccessoryExtra(intent);
            if (!uiOperation.owns(ownerId) || !ReceiverUiOperation.STARTING.equals(uiOperation.phase())
                    || generation != currentOperationGeneration || generation != networkBootstrapGeneration.get()
                    || !requestIdEqualsPendingAoa(requestId) || requestedAccessory == null
                    || pendingAoaAccessory == null
                    || !AoaReceiverCoordinator.isProductAccessory(requestedAccessory)) {
                stopRejectedStartIfIdle();
                return START_NOT_STICKY;
            }
            UsbManager manager = getSystemService(UsbManager.class);
            try {
                if (manager == null || !manager.hasPermission(pendingAoaAccessory)) {
                    requestTermination(ownerId, generation, true, "UsbAccessoryPermissionOrAttachmentLost");
                    return START_NOT_STICKY;
                }
                if (!manager.hasPermission(requestedAccessory)) {
                    requestTermination(ownerId, generation, true, "UsbAccessoryPermissionOrAttachmentLost");
                    return START_NOT_STICKY;
                }
                if (!requestedAccessory.equals(pendingAoaAccessory)) {
                    stopRejectedStartIfIdle();
                    return START_NOT_STICKY;
                }
            } catch (SecurityException permissionLost) {
                requestTermination(ownerId, generation, true, "UsbAccessoryPermissionOrAttachmentLost");
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
            beginHealthOperation(ownerId, generation);
            backgroundStandbyEnabled = false;
            receiveIntentEnabled = false;
            networkAudioState = "stopped";
            networkAudioFailure = "";
            networkAudioAddress = "";
            networkAudioPort = 0;
        }
        backgroundStandbyEnabled = requestedBackgroundStandby;
        if (uiNetworkAudio) receiveIntentEnabled = requestedKeepReceiveIntent || requestedBackgroundStandby;
        operationStartId = startId;

        // A management retry must not downgrade an active mediaPlayback FGS to
        // shortService or attach a short-service timeout to the audio session.
        try {
            Notification notification = serviceNotification(
                    audio ? "AnPod 音频连接正在运行。" : uiAoaAudio
                            ? "正在连接 AnPod USB Direct，可随时停止。" : networkAudio
                            ? receiveIntentEnabled ? "正在等待电脑连接，可随时停止。" : "无线接收正在准备，可随时停止。"
                            : "管理检查进行中；当前不播放音频。",
                    ownerId, generation, networkAudio || uiAoaAudio || audio && currentAudio != null);
            if (Build.VERSION.SDK_INT >= 34) {
                int type = uiAoaAudio
                        ? ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                                | ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
                        : audio
                        ? playbackForegroundServiceType()
                        : receiveIntentEnabled ? ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
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
                pendingUiStandbyStart = false;
                pendingUiReceiveIntent = false;
            }
            if (timeout != null) handler.removeCallbacks(timeout);
            timeout = () -> requestTermination(ownerId, generation, true, "BootstrapTimeout");
            handler.postDelayed(timeout, networkBootstrapTimeoutMs(uiNetworkAudio));
            // Opening the control listener and the discovery sockets binds sockets, which Android
            // forbids on the main thread (NetworkOnMainThreadException). Keep the whole bootstrap on
            // its own thread; it reports state changes through the volatile fields and the handler.
            final String bootstrapAddress = intent.getStringExtra("networkAddress");
            final int bootstrapPrefix = intent.getIntExtra("networkPrefixLength", 0);
            final long bootstrapGeneration = generation;
            final String bootstrapOwner = ownerId;
            final boolean bootstrapStandby = backgroundStandbyEnabled;
            final boolean bootstrapReceiveIntent = receiveIntentEnabled;
            Thread bootstrap = new Thread(() -> startNetworkBootstrap(bootstrapAddress, bootstrapPrefix,
                    bootstrapGeneration, bootstrapOwner, bootstrapStandby, bootstrapReceiveIntent),
                    "AnPod P5-C bootstrap setup");
            bootstrap.setDaemon(true);
            bootstrap.start();
            return START_NOT_STICKY;
        }

        if (uiAoaAudio) {
            pendingAoaRequestId = "";
            pendingAoaAccessory = null;
            if (timeout != null) handler.removeCallbacks(timeout);
            timeout = null;
            uiOperation.running(ownerId);
            UsbAccessory accessory = usbAccessoryExtra(intent);
            aoaAccessory = accessory;
            aoaOwnerId = ownerId;
            aoaOwnerGeneration = generation;
            aoaRecoveryDeadlineElapsedMs = 0;
            aoaRecoveryAttempt = 0;
            aoaRecoveryFailure = "";
            networkAudioState = "starting";
            networkAudioFailure = "";
            if (!registerAoaDetachReceiver(accessory, ownerId, generation)) return START_NOT_STICKY;
            startAoaBootstrap(accessory, ownerId, generation);
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

    private boolean requestIdEqualsPendingAoa(String requestId) {
        return requestId != null && requestId.equals(pendingAoaRequestId);
    }

    private static UsbAccessory usbAccessoryExtra(Intent intent) {
        if (intent == null) return null;
        if (Build.VERSION.SDK_INT >= 33)
            return intent.getParcelableExtra(EXTRA_AOA_ACCESSORY, UsbAccessory.class);
        @SuppressWarnings("deprecation") UsbAccessory accessory =
                (UsbAccessory) intent.getParcelableExtra(EXTRA_AOA_ACCESSORY);
        return accessory;
    }

    boolean notificationPermissionAvailable() {
        return ReceiverNotifications.canPost(this, CHANNEL_ID);
    }

    private void startAoaBootstrap(UsbAccessory accessory, String ownerId, long generation) {
        Thread bootstrap = new Thread(() -> {
            PairingProtocol.BindingMaterial binding = null;
            try {
                if (!isCurrentOperation(ownerId, generation)) throw new IOException("AoaOwnerGenerationExpired");
                binding = new BindingStore(getFilesDir()).load();
                UsbManager manager = getSystemService(UsbManager.class);
                AoaReceiverCoordinator coordinator = AoaReceiverCoordinator.openAccessory(manager, accessory,
                        binding, aoaPlaybackEndpoint(ownerId, generation), ownerId, generation);
                binding = null; // The coordinator now owns and destroys the saved binding material.
                synchronized (networkCoordinatorLock) { aoaCoordinator = coordinator; }
                Thread monitor = new Thread(() -> monitorAoaCoordinator(coordinator, ownerId, generation),
                        "AnPod AOA owner monitor");
                monitor.setDaemon(true);
                monitor.start();
                if (!isCurrentOperation(ownerId, generation)) coordinator.cancel("AoaOwnerGenerationExpired");
            } catch (IOException | PairingProtocol.Failure | RuntimeException error) {
                if (binding != null) binding.close();
                String failure = error instanceof IOException && error.getMessage() != null
                        ? error.getMessage() : error.getClass().getSimpleName();
                if (isCurrentOperation(ownerId, generation)) {
                    if (aoaRecoveryDeadlineElapsedMs > 0) {
                        scheduleAoaRecovery(ownerId, generation, describeAoaFailure(failure));
                    } else requestTermination(ownerId, generation, true, describeAoaFailure(failure));
                }
            } finally {
                synchronized (networkCoordinatorLock) {
                    if (aoaBootstrapThread == Thread.currentThread()) aoaBootstrapThread = null;
                }
                handler.post(() -> {
                    if (ReceiverUiOperation.STOPPING.equals(uiOperation.phase())
                            && uiOperation.owns(ownerId)) finishTerminationIfReady(ownerId, currentOperationGeneration);
                });
            }
        }, "AnPod AOA accessory open");
        bootstrap.setDaemon(true);
        synchronized (networkCoordinatorLock) {
            aoaBootstrapThread = bootstrap;
            bootstrap.start();
        }
    }

    private void monitorAoaCoordinator(AoaReceiverCoordinator coordinator, String ownerId, long generation) {
        try { coordinator.awaitTermination(Long.MAX_VALUE, TimeUnit.DAYS); }
        catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        String failure = coordinator.failureMessage();
        handler.post(() -> {
            if (aoaCoordinator != coordinator || !isCurrentOperation(ownerId, generation)) return;
            boolean restartRequested = failure.isEmpty() && coordinator.restartRequested();
            boolean unexpectedFailure = coordinator.shouldRecoverUnexpectedFailure();
            if ((restartRequested || unexpectedFailure) && aoaAccessory != null) {
                aoaCoordinator = null;
                if (aoaRecoveryDeadlineElapsedMs == 0) {
                    aoaRecoveryDeadlineElapsedMs = android.os.SystemClock.elapsedRealtime()
                            + AOA_RECOVERY_WINDOW_MS;
                    aoaRecoveryAttempt = 0;
                }
                scheduleAoaRecovery(ownerId, generation, failure);
                return;
            }
            requestTermination(ownerId, generation, !failure.isEmpty(), failure);
        });
    }

    void scheduleAoaRecovery(String ownerId, long generation, String failure) {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            handler.post(() -> scheduleAoaRecovery(ownerId, generation, failure));
            return;
        }
        if (!isCurrentOperation(ownerId, generation) || !ownerId.equals(aoaOwnerId)
                || aoaOwnerGeneration != generation) return;
        aoaRecoveryFailure = failure == null ? "" : failure;
        long now = android.os.SystemClock.elapsedRealtime();
        long remaining = aoaRecoveryDeadlineElapsedMs - now;
        if (remaining <= 0) {
            aoaRecoveryDeadlineElapsedMs = 0;
            requestTermination(ownerId, generation, true,
                    aoaRecoveryFailure.isEmpty() ? "USB Direct 恢复窗口已结束" : aoaRecoveryFailure);
            return;
        }
        UsbAccessory accessory = aoaAccessory;
        if (accessory == null || !shouldAttemptAoaRecovery(
                isCurrentOperation(ownerId, generation), isManualStop(ownerId, generation),
                now, aoaRecoveryDeadlineElapsedMs)) {
            aoaRecoveryDeadlineElapsedMs = 0;
            return;
        }
        int delayIndex = Math.min(aoaRecoveryAttempt, AOA_RECOVERY_DELAYS_MS.length - 1);
        long delay = Math.min(AOA_RECOVERY_DELAYS_MS[delayIndex], remaining);
        aoaRecoveryAttempt++;
        handler.postDelayed(() -> {
            long callbackNow = android.os.SystemClock.elapsedRealtime();
            if (!isCurrentOperation(ownerId, generation) || aoaAccessory != accessory) return;
            if (isManualStop(ownerId, generation)) {
                aoaRecoveryDeadlineElapsedMs = 0;
                return;
            }
            if (callbackNow >= aoaRecoveryDeadlineElapsedMs) {
                aoaRecoveryDeadlineElapsedMs = 0;
                requestTermination(ownerId, generation, true,
                        aoaRecoveryFailure.isEmpty() ? "USB Direct 恢复窗口已结束" : aoaRecoveryFailure);
                return;
            }
            startAoaBootstrap(accessory, ownerId, generation);
        }, delay);
    }

    static boolean shouldAttemptAoaRecovery(boolean currentOperation, boolean manualStop,
            long nowElapsedMs, long deadlineElapsedMs) {
        return currentOperation && !manualStop && nowElapsedMs < deadlineElapsedMs;
    }

    void noteAoaPlaybackStarted(String ownerId, long generation) {
        if (!isCurrentOperation(ownerId, generation) || generation != aoaOwnerGeneration
                || !ownerId.equals(aoaOwnerId)) return;
        aoaRecoveryDeadlineElapsedMs = 0;
        aoaRecoveryAttempt = 0;
        aoaRecoveryFailure = "";
    }

    private boolean registerAoaDetachReceiver(UsbAccessory accessory, String ownerId, long generation) {
        unregisterAoaDetachReceiver();
        BroadcastReceiver receiver = new BroadcastReceiver() {
            @Override public void onReceive(android.content.Context context, Intent intent) {
                if (!UsbManager.ACTION_USB_ACCESSORY_DETACHED.equals(intent.getAction())
                        || !isCurrentOperation(ownerId, generation)
                        || !ownerId.equals(aoaOwnerId) || aoaOwnerGeneration != generation
                        || accessory != aoaAccessory) return;
                UsbManager manager = getSystemService(UsbManager.class);
                if (manager == null) {
                    requestTermination(ownerId, generation, true, "UsbAccessoryPermissionOrAttachmentLost");
                    return;
                }
                try {
                    if (!manager.hasPermission(accessory)) {
                        requestTermination(ownerId, generation, true, "UsbAccessoryPermissionOrAttachmentLost");
                        return;
                    }
                    UsbAccessory[] attached = manager.getAccessoryList();
                    if (attached != null) for (UsbAccessory current : attached) {
                        if (!manager.hasPermission(current)) continue;
                        if (accessory.equals(current) && AoaReceiverCoordinator.isProductAccessory(current)) return;
                    }
                } catch (SecurityException permissionLost) {
                    requestTermination(ownerId, generation, true, "UsbAccessoryPermissionOrAttachmentLost");
                    return;
                }
                requestTermination(ownerId, generation, true, "AoaAccessoryDetached");
            }
        };
        try {
            aoaDetachReceiver = receiver;
            IntentFilter filter = new IntentFilter(UsbManager.ACTION_USB_ACCESSORY_DETACHED);
            // USB broadcasts originate outside this app; the handler rechecks UsbManager state before stopping.
            if (Build.VERSION.SDK_INT >= 33) registerReceiver(receiver, filter, RECEIVER_EXPORTED);
            else registerReceiver(receiver, filter);
            return true;
        } catch (RuntimeException error) {
            aoaDetachReceiver = null;
            requestTermination(ownerId, generation, true, "AoaDetachReceiverUnavailable");
            return false;
        }
    }

    private String describeAoaFailure(String failure) {
        if ("AoaSessionCleanupPending".equals(failure)) return getString(R.string.usb_direct_cleanup_pending);
        if ("UsbAccessoryPermissionOrAttachmentLost".equals(failure)
                || "UsbAccessoryChangedDuringOpen".equals(failure)
                || "UsbAccessoryPermissionCheckFailed".equals(failure))
            return getString(R.string.usb_direct_detached_or_permission_lost);
        if ("UsbAccessoryOpenFailed".equals(failure)) return getString(R.string.usb_direct_open_failed);
        if ("AoaDetachReceiverUnavailable".equals(failure)) return getString(R.string.usb_direct_receiver_unavailable);
        return failure;
    }

    private void unregisterAoaDetachReceiver() {
        BroadcastReceiver receiver = aoaDetachReceiver;
        aoaDetachReceiver = null;
        if (receiver != null) try { unregisterReceiver(receiver); }
        catch (IllegalArgumentException ignored) { }
    }

    private boolean applicationIsVisible() {
        ActivityManager manager = getSystemService(ActivityManager.class);
        if (manager == null) return false;
        List<ActivityManager.RunningAppProcessInfo> processes = manager.getRunningAppProcesses();
        if (processes == null) return false;
        for (ActivityManager.RunningAppProcessInfo process : processes) {
            if (process.pid == Process.myPid())
                return process.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_VISIBLE
                        && process.importance != ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND_SERVICE;
        }
        return false;
    }

    private long onceRecoveryDeadline(String ownerId, long generation) {
        return authorization.onceDeadline(ownerId, generation);
    }

    private long newRecoveryDeadline(String ownerId, long generation) {
        return authorization.newDeadline(ownerId, generation);
    }

    private void cancelAuthorizationNotification() {
        authorization.cancel();
    }

    private void setAuthorizationError(String ownerId, long generation, String message) {
        authorization.setError(ownerId, generation, message);
    }


    private int playbackForegroundServiceType() {
        return ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                | (receiveIntentEnabled ? ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE : 0);
    }

    static byte[] unhex(String value) {
        byte[] bytes = new byte[value.length() / 2];
        for (int i = 0; i < bytes.length; i++)
            bytes[i] = (byte) ((Character.digit(value.charAt(i * 2), 16) << 4)
                    | Character.digit(value.charAt(i * 2 + 1), 16));
        return bytes;
    }

    private Notification serviceNotification(String text, String ownerId, long generation, boolean canStop) {
        int immutable = Build.VERSION.SDK_INT >= 23 ? PendingIntent.FLAG_IMMUTABLE : 0;
        PendingIntent stopAction = null;
        if (canStop && ownerId != null && !ownerId.isEmpty()) {
            Intent stop = new Intent(this, ReceiverService.class).setAction(ACTION_NOTIFICATION_STOP)
                    .setData(Uri.parse("anpod://receiver/stop/" + Uri.encode(ownerId) + "/" + generation))
                    .putExtra(EXTRA_UI_OWNER_ID, ownerId).putExtra(EXTRA_UI_GENERATION, generation);
            stopAction = PendingIntent.getService(this, 2, stop,
                    PendingIntent.FLAG_UPDATE_CURRENT | immutable);
        }
        PendingIntent aodAction = null;
        if (getSharedPreferences("anpod_receiver", MODE_PRIVATE).getBoolean("aod_enabled", false)) {
            Intent aod = new Intent(this, AodActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            aodAction = PendingIntent.getActivity(this, 3, aod, PendingIntent.FLAG_UPDATE_CURRENT | immutable);
        }
        return ReceiverNotifications.playback(this, CHANNEL_ID, text, mainActivityPendingIntent(), stopAction, aodAction);
    }

    PendingIntent mainActivityPendingIntent() {
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
        if (backgroundStandbyEnabled) {
            synchronized (BindingStore.storageLock()) { backgroundAuthorization.disable(uiOperation.ownerId(), currentOperationGeneration); }
        }
        cancelAuthorizationNotification();
        backgroundStandbyEnabled = false;
        receiveIntentEnabled = false;
        foregroundReady = false;
        if (pendingAudio != null) pendingAudio.destroy();
        pendingAudio = null;
        AudioSession session = currentAudio;
        currentAudio = null;
        if (session != null) cancelAudio(session, "AudioServiceDestroyed");
        AoaReceiverCoordinator aoa = aoaCoordinator;
        if (aoa != null) aoa.cancel("AudioServiceDestroyed");
        unregisterAoaDetachReceiver();
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




    String startFromUi(String address, int prefixLength, String requestId) {
        return startFromUi(address, prefixLength, requestId, false, false);
    }

    String startFromUi(String address, int prefixLength, String requestId,
            boolean backgroundStandby, boolean keepReceiveIntent) {
        if (requestId == null || requestId.isEmpty()) return "启动请求无效，请重试。";
        if (keepReceiveIntent && !applicationIsVisible()) return "请在 AnPod 界面中点击“连接电脑”。";
        if (backgroundStandby && !applicationIsVisible()) return "请在 Receiver 前台界面中启用后台待机。";
        if (keepReceiveIntent && !notificationPermissionAvailable()) return "请先允许 AnPod 通知，再连接电脑。";
        if (!uiOperation.canBegin() || currentAudio != null || networkCoordinator != null) return "已有接收会话正在运行。";
        try {
            BindingStore.Status binding = new BindingStore(getFilesDir()).status();
            if (!"saved".equals(binding.state)) throw new IOException("请先通过 USB 完成电脑绑定。");
            InetAddress selected = parseIpv4(address);
            findNetworkForInterface(selected, prefixLength);
            String ownerId = UUID.randomUUID().toString();
            long generation;
            synchronized (networkCoordinatorLock) {
                if (!uiOperation.canBegin() || currentAudio != null || pendingAudio != null
                        || networkCoordinator != null || !uiOperation.begin(ownerId))
                    return "已有接收会话正在运行。";
                generation = networkBootstrapGeneration.incrementAndGet();
                currentOperationGeneration = generation;
                beginHealthOperation(ownerId, generation);
                backgroundStandbyEnabled = backgroundStandby;
                receiveIntentEnabled = keepReceiveIntent || backgroundStandby;
                pendingUiStartRequestId = requestId;
                pendingUiStandbyStart = backgroundStandby;
                pendingUiReceiveIntent = keepReceiveIntent || backgroundStandby;
                networkAudioState = "starting";
                networkAudioFailure = "";
                startFailure = "";
            }
            Intent intent = new Intent(this, ReceiverService.class).setAction(ACTION_UI_START_NETWORK_AUDIO)
                    .putExtra("networkAddress", address).putExtra("networkPrefixLength", prefixLength)
                    .putExtra(EXTRA_UI_REQUEST_ID, requestId).putExtra(EXTRA_UI_OWNER_ID, ownerId)
                    .putExtra(EXTRA_UI_GENERATION, generation)
                    .putExtra(EXTRA_BACKGROUND_STANDBY, backgroundStandby)
                    .putExtra(EXTRA_KEEP_RECEIVE_INTENT, keepReceiveIntent || backgroundStandby);
            try {
                if (Build.VERSION.SDK_INT >= 26) startForegroundService(intent); else startService(intent);
            } catch (RuntimeException error) {
                requestTermination(ownerId, generation, true, error.getClass().getSimpleName());
                pendingUiStandbyStart = false;
                pendingUiReceiveIntent = false;
                backgroundStandbyEnabled = false;
                receiveIntentEnabled = false;
                return error.getClass().getSimpleName();
            }
            return "";
        } catch (Exception error) {
            return error.getMessage() == null ? "无法开始接收。" : error.getMessage();
        }
    }

    String startAoaFromUi(UsbAccessory accessory, String requestId) {
        if (requestId == null || requestId.isEmpty()) return getString(R.string.operation_failed_retry);
        if (Build.VERSION.SDK_INT < 24) return getString(R.string.usb_direct_requires_api24);
        if (!applicationIsVisible()) return getString(R.string.usb_direct_foreground_required);
        if (!getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_USB_ACCESSORY))
            return getString(R.string.usb_direct_not_supported);
        if (!AoaReceiverCoordinator.isProductAccessory(accessory)) return getString(R.string.usb_direct_wrong_accessory);
        UsbManager manager = getSystemService(UsbManager.class);
        if (manager == null)
            return getString(R.string.usb_direct_detached_or_permission_lost);
        boolean attached = false;
        try {
            if (!manager.hasPermission(accessory))
                return getString(R.string.usb_direct_detached_or_permission_lost);
            UsbAccessory[] accessories = manager.getAccessoryList();
            if (accessories != null) for (UsbAccessory current : accessories)
                if (manager.hasPermission(current) && accessory.equals(current)
                        && AoaReceiverCoordinator.isProductAccessory(current)) attached = true;
        } catch (SecurityException permissionLost) {
            return getString(R.string.usb_direct_detached_or_permission_lost);
        }
        if (!attached)
            return getString(R.string.usb_direct_detached_or_permission_lost);
        try {
            BindingStore.Status binding = new BindingStore(getFilesDir()).status();
            if (!"saved".equals(binding.state)) return getString(R.string.usb_direct_requires_pairing);
        } catch (IOException | PairingProtocol.Failure error) {
            return getString(R.string.binding_status_temporarily_unavailable);
        }
        String ownerId = UUID.randomUUID().toString();
        long generation;
        synchronized (networkCoordinatorLock) {
            if (!uiOperation.canBegin() || currentAudio != null || pendingAudio != null
                    || networkCoordinator != null || aoaCoordinator != null || aoaBootstrapThread != null
                    || !uiOperation.begin(ownerId)) return getString(R.string.usb_direct_busy);
            generation = networkBootstrapGeneration.incrementAndGet();
            currentOperationGeneration = generation;
            beginHealthOperation(ownerId, generation);
            backgroundStandbyEnabled = false;
            receiveIntentEnabled = false;
            pendingAoaRequestId = requestId;
            pendingAoaAccessory = accessory;
            aoaAccessory = accessory;
            aoaOwnerId = ownerId;
            aoaOwnerGeneration = generation;
            networkAudioState = "starting";
            networkAudioFailure = "";
            networkAudioAddress = "";
            networkAudioPort = 0;
            startFailure = "";
        }
        Intent start = new Intent(this, ReceiverService.class).setAction(ACTION_UI_START_AOA_AUDIO)
                .setData(Uri.parse("anpod://receiver/aoa/" + Uri.encode(ownerId) + "/" + generation))
                .putExtra(EXTRA_UI_REQUEST_ID, requestId)
                .putExtra(EXTRA_UI_OWNER_ID, ownerId)
                .putExtra(EXTRA_UI_GENERATION, generation)
                .putExtra(EXTRA_AOA_ACCESSORY, accessory);
        try {
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(start); else startService(start);
        } catch (RuntimeException error) {
            requestTermination(ownerId, generation, true, error.getClass().getSimpleName());
            return error.getClass().getSimpleName();
        }
        return "";
    }

    boolean stopFromUi(String ownerId) {
        if (!uiOperation.owns(ownerId) || !(ReceiverUiOperation.STARTING.equals(uiOperation.phase())
                || ReceiverUiOperation.RUNNING.equals(uiOperation.phase()))) return false;
        recordManualStop(ownerId, currentOperationGeneration);
        requestTermination(ownerId, currentOperationGeneration, false, "");
        return true;
    }

    boolean stopFromUi(String ownerId, long generation) {
        if (!isCurrentOperation(ownerId, generation)) return false;
        recordManualStop(ownerId, generation);
        requestTermination(ownerId, generation, false, "");
        return true;
    }

    boolean trustBindingUntilRevokedFromUi(String ownerId, long generation) {
        if (!applicationIsVisible() || !isCurrentOperation(ownerId, generation) || !backgroundStandbyEnabled) return false;
        try {
            synchronized (BindingStore.storageLock()) { backgroundAuthorization.trustUntilRevoked(ownerId, generation); }
            return true;
        } catch (IOException error) {
            setAuthorizationError(ownerId, generation, error.getClass().getSimpleName());
            return false;
        }
    }

    boolean revokeBackgroundTrustFromUi(String expectedOwnerId, long expectedGeneration,
            String expectedBindingFingerprint) {
        if (!applicationIsVisible() || expectedOwnerId == null || expectedBindingFingerprint == null
                || expectedBindingFingerprint.length() != 64) return false;
        boolean terminateActiveStandby = false;
        synchronized (networkCoordinatorLock) {
            if (!expectedOwnerId.equals(uiOperation.ownerId())
                    || expectedGeneration != currentOperationGeneration
                    || expectedGeneration != networkBootstrapGeneration.get()) return false;
            boolean activeStandby = backgroundStandbyEnabled && backgroundAuthorization.enabled()
                    && isCurrentOperation(expectedOwnerId, expectedGeneration);
            boolean idle = uiOperation.canBegin() && currentAudio == null && pendingAudio == null
                    && networkCoordinator == null && !backgroundStandbyEnabled;
            if (!activeStandby && !idle) return false;
            synchronized (BindingStore.storageLock()) {
                try {
                    if (activeStandby) {
                        byte[] requested = unhex(expectedBindingFingerprint);
                        try {
                            verifyBackgroundBinding(expectedOwnerId, expectedGeneration, true, requested);
                            backgroundAuthorization.revoke(expectedOwnerId, expectedGeneration);
                        } finally { java.util.Arrays.fill(requested, (byte) 0); }
                        backgroundStandbyEnabled = false;
                        networkRecoveryDeadlineElapsedMs = 0;
                        terminateActiveStandby = true;
                    } else {
                        BindingStore.Status status = new BindingStore(getFilesDir()).status();
                        if (!"saved".equals(status.state)
                                || !sameBindingFingerprint(expectedBindingFingerprint, status.commitment)) return false;
                        byte[] commitment = unhex(status.commitment);
                        try {
                            backgroundAuthorization.inspectBinding(commitment);
                            backgroundAuthorization.revokeBindingGrant(status.commitment);
                        } finally { java.util.Arrays.fill(commitment, (byte) 0); }
                    }
                    cancelAuthorizationNotification();
                } catch (IOException | PairingProtocol.Failure error) {
                    backgroundAuthorization.reportErrorForBinding(expectedBindingFingerprint,
                            describeFailure(error));
                    return false;
                }
            }
        }
        if (terminateActiveStandby)
            requestTermination(expectedOwnerId, expectedGeneration, false, "BackgroundTrustRevoked");
        return true;
    }

    boolean canRevokeBackgroundTrust(String binding, String fingerprint, String grantKind) {
        boolean hasGrant = "untilRevoked".equals(grantKind) || "eightHours".equals(grantKind)
                || "once".equals(grantKind);
        boolean activeStandby = backgroundStandbyEnabled && backgroundAuthorization.enabled()
                && (ReceiverUiOperation.STARTING.equals(uiOperation.phase())
                        || ReceiverUiOperation.RUNNING.equals(uiOperation.phase()));
        boolean idle = uiOperation.canBegin() && currentAudio == null && pendingAudio == null
                && networkCoordinator == null && !backgroundStandbyEnabled;
        return currentOperationGeneration == networkBootstrapGeneration.get()
                && applicationIsVisible() && "saved".equals(binding) && !fingerprint.isEmpty()
                && hasGrant && (activeStandby || idle);
    }

    private String verifyBackgroundBinding(String ownerId, long generation, boolean standbyRequested,
            byte[] requestedCommitment) throws IOException {
        if (!standbyRequested || !backgroundStandbyEnabled || !isCurrentOperation(ownerId, generation)
                || requestedCommitment == null || requestedCommitment.length != 32)
            throw new IOException("Background authorization session is no longer current");
        BindingStore.Status status;
        try { status = new BindingStore(getFilesDir()).status(); }
        catch (PairingProtocol.Failure error) { throw new IOException("AnPod binding is unavailable", error); }
        String requested = BackgroundStartAuthorization.hex(requestedCommitment);
        if (!"saved".equals(status.state) || !sameBindingFingerprint(requested, status.commitment)
                || !requested.equals(backgroundAuthorization.bindingCommitment()))
            throw new IOException("AnPod binding changed during background authorization");
        backgroundAuthorization.inspectBinding(requestedCommitment);
        return status.commitment;
    }

    private static boolean sameBindingFingerprint(String left, String right) {
        if (left == null || right == null || left.length() != 64 || right.length() != 64) return false;
        byte[] leftBytes = unhex(left), rightBytes = unhex(right);
        try { return MessageDigest.isEqual(leftBytes, rightBytes); }
        finally {
            java.util.Arrays.fill(leftBytes, (byte) 0);
            java.util.Arrays.fill(rightBytes, (byte) 0);
        }
    }

    private static synchronized void beginHealthOperation(String ownerId, long generation) {
        operationOwnerId = ownerId;
        operationGeneration = generation;
        terminalCause = "";
        terminalOwnerId = "";
        terminalGeneration = -1;
    }

    private static synchronized void recordManualStop(String ownerId, long generation) {
        terminalCause = "USER_STOP";
        terminalOwnerId = ownerId;
        terminalGeneration = generation;
    }

    private static synchronized boolean isManualStop(String ownerId, long generation) {
        return "USER_STOP".equals(terminalCause) && ownerId.equals(terminalOwnerId)
                && generation == terminalGeneration;
    }

    void requestTermination(String ownerId, long generation, boolean failed, String reason) {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            handler.post(() -> beginTermination(ownerId, generation, failed, reason));
        } else beginTermination(ownerId, generation, failed, reason);
    }

    private void beginTermination(String ownerId, long generation, boolean failed, String reason) {
        if (!failed && (reason == null || reason.isEmpty()) && isManualStop(ownerId, generation)) {
            AoaReceiverCoordinator activeAoa;
            P5cReceiverCoordinator activeNetwork;
            synchronized (networkCoordinatorLock) {
                activeAoa = ownerId.equals(aoaOwnerId) && aoaOwnerGeneration == generation
                        ? aoaCoordinator : null;
                activeNetwork = ownerId.equals(networkCoordinatorOwnerId)
                        && generation == currentOperationGeneration ? networkCoordinator : null;
            }
            if (activeAoa != null) {
                if (activeAoa.requestUserStop()) return;
            }
            if (activeNetwork != null) {
                if (activeNetwork.requestUserStop()) return;
            }
        }
        synchronized (networkCoordinatorLock) {
            if (!uiOperation.owns(ownerId) || generation != currentOperationGeneration
                    || generation != networkBootstrapGeneration.get() || !uiOperation.stop(ownerId)) return;
        }
        if (backgroundStandbyEnabled) {
            synchronized (BindingStore.storageLock()) { backgroundAuthorization.disable(ownerId, generation); }
        }
        backgroundStandbyEnabled = false;
        receiveIntentEnabled = false;
        cancelAuthorizationNotification();
        cleanupOwnerId = ownerId;
        cleanupGeneration = networkBootstrapGeneration.incrementAndGet();
        currentOperationGeneration = cleanupGeneration;
        cleanupFailed = failed;
        cleanupError = failed ? (reason == null || reason.isEmpty() ? "ReceiverOperationFailed" : reason) : "";
        cleanupCoordinatorClosed = true;
        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = null;
        pendingUiStartRequestId = "";
        pendingAoaRequestId = "";
        pendingAoaAccessory = null;
        pendingUiStandbyStart = false;
        pendingUiReceiveIntent = false;
        networkRecoveryDeadlineElapsedMs = 0;
        if (pendingAudio != null) pendingAudio.destroy();
        pendingAudio = null;

        AudioSession session = currentAudio;
        if (session != null && ownerId.equals(session.ownerId))
            cancelAudio(session, session.request.usesPacketIngress() ? "NetworkSessionStopped" : "UiSessionStopped");
        Thread opening = null;
        AoaReceiverCoordinator aoa = null;
        synchronized (networkCoordinatorLock) {
            if (ownerId.equals(aoaOwnerId) && aoaOwnerGeneration == generation) {
                opening = aoaBootstrapThread;
                aoa = aoaCoordinator;
            }
        }
        if (aoa != null) aoa.cancel("AoaOwnerStopped");
        unregisterAoaDetachReceiver();
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
        if (opening != null || aoa != null) {
            Thread openingThread = opening;
            AoaReceiverCoordinator initialCoordinator = aoa;
            cleanupCoordinatorClosed = false;
            long terminatingGeneration = cleanupGeneration;
            new Thread(() -> {
                boolean interrupted = false;
                if (openingThread != null && openingThread != Thread.currentThread()) {
                    while (openingThread.isAlive()) try { openingThread.join(); }
                    catch (InterruptedException error) { interrupted = true; }
                }
                AoaReceiverCoordinator completedCoordinator = initialCoordinator;
                synchronized (networkCoordinatorLock) {
                    if (completedCoordinator == null && ownerId.equals(aoaOwnerId)
                            && aoaOwnerGeneration == generation) completedCoordinator = aoaCoordinator;
                }
                if (completedCoordinator != null) {
                    completedCoordinator.cancel("AoaOwnerCleanup");
                    for (;;) try {
                        completedCoordinator.awaitTermination(Long.MAX_VALUE, TimeUnit.DAYS);
                        break;
                    } catch (InterruptedException error) { interrupted = true; }
                    String aoaCleanupFailure = completedCoordinator.failureMessage();
                    if (!aoaCleanupFailure.isEmpty()) {
                        cleanupFailed = true;
                        cleanupError = cleanupError.isEmpty() ? aoaCleanupFailure
                                : cleanupError + "; " + aoaCleanupFailure;
                    }
                }
                if (interrupted) Thread.currentThread().interrupt();
                AoaReceiverCoordinator cleaned = completedCoordinator;
                handler.post(() -> {
                    if (!isPendingCleanup(ownerId, terminatingGeneration)) return;
                    synchronized (networkCoordinatorLock) {
                        if (ownerId.equals(aoaOwnerId) && aoaOwnerGeneration == generation) {
                            if (cleaned == null || aoaCoordinator == cleaned) aoaCoordinator = null;
                            aoaAccessory = null;
                            aoaOwnerId = "";
                            aoaOwnerGeneration = 0;
                            aoaBootstrapThread = null;
                        }
                    }
                    unregisterAoaDetachReceiver();
                    cleanupCoordinatorClosed = true;
                    finishTerminationIfReady(ownerId, terminatingGeneration);
                });
            }, "AnPod AOA session cleanup").start();
        } else if (ownerId.equals(aoaOwnerId) && aoaOwnerGeneration == generation) {
            synchronized (networkCoordinatorLock) {
                if (ownerId.equals(aoaOwnerId) && aoaOwnerGeneration == generation) {
                    aoaCoordinator = null;
                    aoaAccessory = null;
                    aoaOwnerId = "";
                    aoaOwnerGeneration = 0;
                    aoaBootstrapThread = null;
                }
            }
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

    static void resetAuthorizationForBinding(BindingStore store, String commitment) throws IOException {
        synchronized (BindingStore.storageLock()) {
            ReceiverService service = activeService;
            if (service != null && service.backgroundAuthorization != null)
                service.backgroundAuthorization.resetBinding(commitment);
            else store.clearUntilRevoked(commitment);
        }
    }

    private AudioSession startAudio(AudioRequest request) {
        return startAudio(request, uiOperation.ownerId(), networkBootstrapGeneration.get());
    }

    private AudioSession startAudio(AudioRequest request, String ownerId, long generation) {
        WAVEFORM.reset();
        if (destroyed || !isCurrentOperation(ownerId, generation)) return null;
        AudioSession existing = currentAudio;
        if (existing != null) {
            if (pendingAudio != null) pendingAudio.destroy();
            pendingAudio = request;
            cancelAudio(existing, "AudioSessionReplaced");
            return null;
        }
        if (request.udpContext == null && request.aoaContext == null && (request.socketName == null || request.socketName.isEmpty()
                || request.connectCode == null || request.connectCode.isEmpty())) {
            recordStartFailure("InvalidAudioRequest");
            request.destroy();
            requestTermination(uiOperation.ownerId(), currentOperationGeneration, true, "InvalidAudioRequest");
            return null;
        }
        AudioSession session = new AudioSession(NEXT_AUDIO_ID.incrementAndGet(), request,
                ownerId, generation);
        session.requestedOutputEngine = requestedOutputEnginePreference();
        session.actualOutputEngine = "unknown";
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
        if (request.usesPacketIngress()) PCM_BYTES_WRITTEN.set(0);
        session.thread = new Thread(() -> receiveAudio(session), "AnPod audio receiver");
        session.thread.start();
        handler.post(() -> {
            if (currentAudio == session && isCurrentOperation(session.ownerId, session.networkGeneration)) {
                try {
                    Notification notification = serviceNotification("正在播放电脑音频。", session.ownerId,
                            session.networkGeneration, true);
                    if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION_ID, notification,
                            playbackForegroundServiceType());
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
        WAVEFORM.reset();
        String ownerId = networkCoordinatorOwnerId;
        long generation = currentOperationGeneration;
        Runnable start = () -> {
            if (!isCurrentOperation(ownerId, generation)) context.destroy();
            else startAudio(AudioRequest.udp(context));
        };
        if (Looper.myLooper() == Looper.getMainLooper()) start.run();
        else if (!handler.post(start)) context.destroy();
    }

    /** Starts one already-authenticated AOA session before START is ACKed; the caller retains context ownership. */
    AudioSession startAoaAudio(AoaSessionContext context, String ownerId, long generation) throws IOException {
        if (context == null) throw new NullPointerException("context");
        AudioStartTicket ticket = new AudioStartTicket();
        Runnable start = () -> {
            synchronized (ticket) {
                try {
                    if (ticket.cancelled || !isCurrentOperation(ownerId, generation)
                            || currentAudio != null || pendingAudio != null) {
                        context.stopQueue();
                        return;
                    }
                    AudioSession accepted = startAudio(AudioRequest.aoa(context), ownerId, generation);
                    if (accepted == null || accepted.request.aoaContext != context || currentAudio != accepted)
                        throw new IOException("AoaAudioSessionNotAccepted");
                    ticket.accepted = accepted;
                } catch (IOException | RuntimeException error) {
                    context.stopQueue();
                    if (isCurrentOperation(ownerId, generation))
                        requestTermination(ownerId, generation, true,
                                error instanceof IOException ? error.getMessage() : error.getClass().getSimpleName());
                } finally { ticket.finished.countDown(); }
            }
        };
        if (Looper.myLooper() == Looper.getMainLooper()) start.run();
        else if (!handler.post(start)) {
            context.stopQueue();
            throw new IOException("AoaAudioServiceUnavailable");
        }
        try {
            if (!ticket.finished.await(3, TimeUnit.SECONDS)) {
                AudioSession accepted;
                synchronized (ticket) {
                    accepted = ticket.accepted;
                    if (accepted == null) ticket.cancelled = true;
                }
                if (accepted == null) {
                    context.stopQueue();
                    throw new IOException("AoaAudioSessionStartTimedOut");
                }
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            AudioSession accepted;
            synchronized (ticket) {
                accepted = ticket.accepted;
                if (accepted == null) ticket.cancelled = true;
            }
            if (accepted != null) {
                Runnable cancel = () -> {
                    if (isCurrentOperation(ownerId, generation) && currentAudio == accepted)
                        cancelAudio(accepted, "AoaAudioStartInterrupted");
                };
                if (Looper.myLooper() == Looper.getMainLooper()) cancel.run(); else handler.post(cancel);
            }
            context.stopQueue();
            throw new IOException("AoaAudioSessionStartInterrupted", interrupted);
        }
        if (ticket.accepted == null || currentAudio != ticket.accepted || !isCurrentOperation(ownerId, generation))
            throw new IOException("AoaAudioSessionNotAccepted");
        return ticket.accepted;
    }

    AoaReceiverCoordinator.PlaybackEndpoint aoaPlaybackEndpoint(String ownerId, long generation) {
        return new AoaReceiverCoordinator.PlaybackEndpoint() {
            private AudioSession ownedSession;
            private boolean current() { return isCurrentOperation(ownerId, generation); }

            @Override public boolean isCurrent(String expectedOwner, long expectedGeneration) {
                return ownerId.equals(expectedOwner) && generation == expectedGeneration && current();
            }

            @Override public boolean supportsAudio(UdpPacketDecoder.AudioFormatV2 format) {
                return format != null && format.profile >= 1 && format.profile <= 5;
            }

            @Override public void start(AoaSessionContext context) throws IOException {
                if (!current()) throw new IOException("AoaOwnerStopped");
                AudioSession accepted = startAoaAudio(context, ownerId, generation);
                ownedSession = accepted;
                noteAoaPlaybackStarted(ownerId, generation);
            }

            @Override public void pause() throws IOException {
                runOnMainForOwner(ownerId, generation, () -> setUdpAudioPaused(ownedSession, true));
            }

            @Override public void resume() throws IOException {
                runOnMainForOwner(ownerId, generation, () -> setUdpAudioPaused(ownedSession, false));
            }

            @Override public void stop() {
                AudioSession session = ownedSession;
                if (session == null) return;
                Runnable stop = () -> {
                    if (currentAudio == session && isCurrentOperation(ownerId, generation))
                        cancelAudio(session, "AoaSessionStopped");
                };
                if (Looper.myLooper() == Looper.getMainLooper()) stop.run(); else handler.post(stop);
            }

            @Override public boolean streaming() throws IOException {
                if (!current()) throw new IOException("AoaOwnerStopped");
                AudioSession session = ownedSession;
                if (session == null || currentAudio != session) throw new IOException("AoaAudioSessionNotOwned");
                return session.isStreamingAt(System.nanoTime());
            }

            @Override public boolean supportsPlaybackProgressExtension() { return true; }

            @Override public byte[] playbackProgressPayloadExtendedV2() {
                return ReceiverService.playbackProgressPayloadExtendedV2();
            }

            @Override public byte[] playbackProgressPayloadV2() {
                AudioSession session = ownedSession;
                byte[] legacy = ReceiverService.playbackProgressPayload();
                return P5cControlProtocol.playbackProgressPayloadV2(legacy,
                        session == null ? 0 : session.openedTrackFormatV2,
                        session == null ? 0 : session.openedTrackChannelsV2,
                        session == null ? 0 : session.openedTrackSampleRateV2);
            }
        };
    }

    private void startNetworkBootstrap(String address, int prefixLength, long generation, String ownerId,
            boolean backgroundStandby, boolean keepReceiveIntent) {
        try {
            if (!isCurrentOperation(ownerId, generation)) return;
            InetAddress local = parseIpv4(address);
            Network selectedNetwork = findNetworkForInterface(local, prefixLength);
            PairingProtocol.BindingMaterial binding;
            synchronized (BindingStore.storageLock()) {
                if (!isCurrentOperation(ownerId, generation)) return;
                networkAudioState = "starting";
                networkAudioFailure = "";
                startFailure = "";
                networkRecoveryDeadlineElapsedMs = 0;
                binding = new BindingStore(getFilesDir()).load();
                if (backgroundStandby) {
                    if (!notificationPermissionAvailable()) throw new IOException("Background notifications are unavailable");
                    byte[] commitment = binding.commitment();
                    try { backgroundAuthorization.enable(ownerId, generation, commitment); }
                    finally { java.util.Arrays.fill(commitment, (byte) 0); }
                }
            }
            P5cReceiverCoordinator.PlaybackEndpoint playback = new P5cReceiverCoordinator.PlaybackEndpoint() {
                private AudioSession ownedSession;
                private boolean current() { return isCurrentOperation(ownerId, generation); }

                @Override public boolean supportsPlaybackProgress() { return true; }
                @Override public boolean supportsPlaybackProgressExtension() { return true; }
                @Override public boolean supportsAudioV2() { return true; }
                @Override public boolean supportsAudioFormatV2(UdpPacketDecoder.AudioFormatV2 format) {
                    return ReceiverService.supportsAudioFormatV2(format);
                }
                @Override public int openedTrackFormatV2() {
                    AudioSession session = ownedSession;
                    return session == null ? 0 : session.openedTrackFormatV2;
                }
                @Override public int openedTrackChannelsV2() {
                    AudioSession session = ownedSession;
                    return session == null ? 0 : session.openedTrackChannelsV2;
                }
                @Override public int openedTrackSampleRateV2() {
                    AudioSession session = ownedSession;
                    return session == null ? 0 : session.openedTrackSampleRateV2;
                }

                @Override public boolean requiresBackgroundStartAuthorization() {
                    return backgroundStandby && current();
                }

                @Override public boolean supportsBackgroundTrustManagement() {
                    return backgroundStandby && backgroundStandbyEnabled && current();
                }

                @Override public P5cReceiverCoordinator.BackgroundTrustSnapshot queryBackgroundTrust(
                        byte[] bindingCommitment) throws IOException {
                    synchronized (BindingStore.storageLock()) {
                        try {
                            verifyBackgroundBinding(ownerId, generation, backgroundStandby, bindingCommitment);
                            BackgroundStartAuthorization.GrantSnapshot grant = backgroundAuthorization.grantSnapshot();
                            return new P5cReceiverCoordinator.BackgroundTrustSnapshot(grant.kindCode,
                                    grant.persistenceCode, grant.remainingMs);
                        } catch (IOException error) {
                            backgroundAuthorization.reportError(ownerId, generation, describeFailure(error));
                            throw error;
                        }
                    }
                }

                @Override public boolean revokeBackgroundTrust(byte[] bindingCommitment) throws IOException {
                    synchronized (BindingStore.storageLock()) {
                        try {
                            verifyBackgroundBinding(ownerId, generation, backgroundStandby, bindingCommitment);
                            backgroundAuthorization.revoke(ownerId, generation);
                            backgroundStandbyEnabled = false;
                            networkRecoveryDeadlineElapsedMs = 0;
                            cancelAuthorizationNotification();
                            return true;
                        } catch (IOException error) {
                            backgroundAuthorization.reportError(ownerId, generation, describeFailure(error));
                            throw error;
                        }
                    }
                }

                @Override public int requestBackgroundStartAuthorization(byte[] requestId, long sessionId,
                        InetAddress peer, byte[] bindingCommitment) throws IOException {
                    if (!current() || !notificationPermissionAvailable()) {
                        requestTermination(ownerId, generation, true, "BackgroundNotificationsUnavailable");
                        return P5cControlProtocol.AUTH_EXPIRED;
                    }
                    synchronized (BindingStore.storageLock()) {
                        return backgroundAuthorization.request(ownerId, generation, requestId, sessionId,
                                peer, bindingCommitment);
                    }
                }

                @Override public int backgroundStartAuthorizationStatus(byte[] requestId, long sessionId,
                        InetAddress peer, byte[] bindingCommitment) throws IOException {
                    if (!current() || !notificationPermissionAvailable()) {
                        requestTermination(ownerId, generation, true, "BackgroundNotificationsUnavailable");
                        return P5cControlProtocol.AUTH_EXPIRED;
                    }
                    synchronized (BindingStore.storageLock()) {
                        return backgroundAuthorization.status(ownerId, generation, requestId, sessionId,
                                peer, bindingCommitment);
                    }
                }

                @Override public void endBackgroundStartAuthorization(byte[] requestId, long sessionId,
                        InetAddress peer, byte[] bindingCommitment, boolean playbackHadStarted, boolean keepForRecovery) {
                    synchronized (BindingStore.storageLock()) {
                        backgroundAuthorization.end(ownerId, generation, requestId, sessionId, peer,
                                bindingCommitment, playbackHadStarted, keepForRecovery);
                    }
                    if (keepForRecovery) networkRecoveryDeadlineElapsedMs = onceRecoveryDeadline(ownerId, generation);
                    else if (playbackHadStarted) networkRecoveryDeadlineElapsedMs = 0;
                }

                @Override public void cancelBackgroundStartAuthorization(byte[] requestId, long sessionId,
                        InetAddress peer, byte[] bindingCommitment) {
                    synchronized (BindingStore.storageLock()) {
                        BackgroundStartAuthorization.Token token = backgroundAuthorization.pendingToken();
                        if (token != null && token.matches(ownerId, generation, requestId, sessionId, peer, bindingCommitment))
                            backgroundAuthorization.cancel(token);
                    }
                }

                @Override public void backgroundPlaybackStarted(byte[] requestId, long sessionId,
                        InetAddress peer, byte[] bindingCommitment) {
                    synchronized (BindingStore.storageLock()) {
                        backgroundAuthorization.playbackStarted(ownerId, generation, requestId, sessionId,
                                peer, bindingCommitment);
                    }
                    networkRecoveryDeadlineElapsedMs = 0;
                }

                @Override public byte[] playbackProgressPayload() {
                    return ReceiverService.playbackProgressPayload();
                }

                @Override public byte[] playbackProgressPayloadExtended(boolean audioV2) {
                    return ReceiverService.playbackProgressPayloadExtended(audioV2);
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
                                        playbackForegroundServiceType());
                                else startForeground(NOTIFICATION_ID, notification);
                                networkRecoveryDeadlineElapsedMs = onceRecoveryDeadline(ownerId, generation);
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
                            networkRecoveryDeadlineElapsedMs = receiveIntentEnabled
                                    ? onceRecoveryDeadline(ownerId, generation)
                                    : newRecoveryDeadline(ownerId, generation);
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

                @Override public boolean keepStandbyAfterPlaybackStop() { return receiveIntentEnabled; }
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
            byte[] discoverySecret = null;
            try (PairingProtocol.BindingMaterial material = new BindingStore(getFilesDir()).load()) {
                discoverySecret = material.discoverySecret();
            } catch (java.io.IOException | PairingProtocol.Failure ignored) {
            }
            coordinator.startDiscovery(selectedNetwork::bindSocket, discoverySecret);
            if (discoverySecret != null) java.util.Arrays.fill(discoverySecret, (byte) 0);
            if (keepReceiveIntent) coordinator.startAutomaticSearch();
            uiOperation.running(ownerId);
            networkAudioState = "prepared";
            if (keepReceiveIntent && timeout != null) {
                handler.removeCallbacks(timeout);
                timeout = null;
            }
            Thread waiter = new Thread(() -> {
                long nextStandbyCheckAt = 0;
                long nextNetworkCheckAt = 0;
                long nextSearchMaintenanceAt = 0;
                try {
                    while (true) {
                        boolean completed = coordinator.await(250, TimeUnit.MILLISECONDS);
                        if (!isCurrentOperation(ownerId, generation)) return;
                        if (completed) {
                            handler.post(() -> handleCoordinatorEnded(coordinator, ownerId, generation));
                            return;
                        }
                        long now = android.os.SystemClock.elapsedRealtime();
                        if (keepReceiveIntent && now >= nextSearchMaintenanceAt) {
                            nextSearchMaintenanceAt = now + 1_000L;
                            try {
                                P5cDiscovery.SearchSnapshot search = coordinator.maintainAutomaticSearch();
                                if ("searching".equals(search.lastRequestStatus)) networkAudioFailure = "";
                            } catch (IOException searchError) {
                                networkAudioFailure = searchError.getMessage() == null
                                        ? searchError.getClass().getSimpleName() : searchError.getMessage();
                            }
                        }
                        if (now >= nextNetworkCheckAt && (keepReceiveIntent
                                || (currentAudio == null && "prepared".equals(networkAudioState)))) {
                            nextNetworkCheckAt = now + 1_000L;
                            try {
                                if (!isNetworkForInterface(selectedNetwork, local, prefixLength)) {
                                    requestTermination(ownerId, generation, true, "ReceiveInterfaceChanged");
                                    return;
                                }
                            } catch (IOException unavailable) {
                                requestTermination(ownerId, generation, true, "ReceiveInterfaceUnavailable");
                                return;
                            }
                        }
                        if (keepReceiveIntent && now >= nextStandbyCheckAt) {
                            nextStandbyCheckAt = now + 1_000L;
                            if (backgroundStandby) {
                                long trustDeadline;
                                boolean expiredTrust;
                                try {
                                    synchronized (BindingStore.storageLock()) {
                                        trustDeadline = backgroundAuthorization.eightHourDeadlineElapsedMs(ownerId, generation);
                                        expiredTrust = trustDeadline != 0 && backgroundAuthorization.expireEightHourGrant(
                                                ownerId, generation, trustDeadline);
                                    }
                                } catch (IOException storageError) {
                                    requestTermination(ownerId, generation, true, "BackgroundGrantStorageFailed");
                                    return;
                                }
                                if (expiredTrust) {
                                    requestTermination(ownerId, generation, false, "BackgroundTrustExpired");
                                    return;
                                }
                            }
                            if (!notificationPermissionAvailable()) {
                                requestTermination(ownerId, generation, true, "ReceiveNotificationsUnavailable");
                                return;
                            }
                        }
                        long recoveryDeadline = networkRecoveryDeadlineElapsedMs;
                        if (recoveryDeadline != 0 && currentAudio == null
                                && android.os.SystemClock.elapsedRealtime() >= recoveryDeadline) {
                            if (keepReceiveIntent) {
                                networkRecoveryDeadlineElapsedMs = 0;
                                networkAudioState = "prepared";
                            } else {
                                requestTermination(ownerId, generation, true, "RecoveryTimeout");
                                return;
                            }
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

    boolean isCurrentOperation(String ownerId, long generation) {
        String phase = uiOperation.phase();
        return !destroyed && uiOperation.owns(ownerId) && currentOperationGeneration == generation
                && networkBootstrapGeneration.get() == generation
                && (ReceiverUiOperation.STARTING.equals(phase) || ReceiverUiOperation.RUNNING.equals(phase));
    }

    boolean isAoaSession() {
        return aoaOwnerId != null && !aoaOwnerId.isEmpty();
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

    static final class AvailableNetwork {
        final Network network;
        final String interfaceName;
        final int interfaceIndex;
        final InetAddress address;
        final int prefixLength;

        AvailableNetwork(Network network, String interfaceName, int interfaceIndex, InetAddress address, int prefixLength) {
            this.network = network;
            this.interfaceName = interfaceName;
            this.interfaceIndex = interfaceIndex;
            this.address = address;
            this.prefixLength = prefixLength;
        }
    }

    static List<AvailableNetwork> availableNetworks(android.content.Context context) throws SocketException {
        ConnectivityManager connectivity = context.getSystemService(ConnectivityManager.class);
        if (connectivity == null) return Collections.emptyList();
        ArrayList<AvailableNetwork> result = new ArrayList<>();
        for (Network network : connectivity.getAllNetworks()) {
            LinkProperties properties = connectivity.getLinkProperties(network);
            if (properties == null || properties.getInterfaceName() == null
                    || properties.getInterfaceName().isEmpty()) continue;
            NetworkInterface networkInterface = NetworkInterface.getByName(properties.getInterfaceName());
            if (networkInterface == null || !networkInterface.isUp() || networkInterface.isLoopback()) continue;
            for (LinkAddress link : properties.getLinkAddresses()) {
                InetAddress address = link.getAddress();
                int prefixLength = link.getPrefixLength();
                if (!isSelectableNetworkAddress(properties, address, prefixLength)
                        || !isAssignedAddress(networkInterface, address, prefixLength)) continue;
                boolean alreadyListed = false;
                for (AvailableNetwork existing : result)
                    if (address.equals(existing.address) && prefixLength == existing.prefixLength) {
                        alreadyListed = true;
                        break;
                    }
                if (!alreadyListed) result.add(new AvailableNetwork(network, properties.getInterfaceName(),
                        networkInterface.getIndex(), address, prefixLength));
            }
        }
        return result;
    }

    private static boolean isAssignedAddress(NetworkInterface networkInterface, InetAddress address,
            int prefixLength) {
        for (java.net.InterfaceAddress item : networkInterface.getInterfaceAddresses())
            if (address.equals(item.getAddress()) && prefixLength == item.getNetworkPrefixLength()) return true;
        return false;
    }

    private static boolean isSelectableNetworkAddress(LinkProperties properties, InetAddress address,
            int prefixLength) {
        if (properties == null || properties.getInterfaceName() == null
                || properties.getInterfaceName().isEmpty()
                || !(address instanceof Inet4Address) || !address.isSiteLocalAddress()
                || prefixLength < 1 || prefixLength > 32) return false;
        for (LinkAddress link : properties.getLinkAddresses())
            if (address.equals(link.getAddress()) && prefixLength == link.getPrefixLength()) return true;
        return false;
    }

    private Network findNetworkForInterface(InetAddress address, int prefixLength) throws IOException {
        if (!(address instanceof Inet4Address) || !address.isSiteLocalAddress()
                || prefixLength < 1 || prefixLength > 32)
            throw new IOException("所选 Android 网络已不可用或发生变化。");
        try {
            for (AvailableNetwork available : availableNetworks(this))
                if (address.equals(available.address) && prefixLength == available.prefixLength)
                    return available.network;
        } catch (SocketException unavailable) {
            throw new IOException("所选 Android 网络已不可用或发生变化。", unavailable);
        }
        throw new IOException("所选 Android 网络已不可用或发生变化。");
    }

    boolean isNetworkForInterface(Network network, InetAddress address, int prefixLength)
            throws IOException {
        try {
            for (AvailableNetwork available : availableNetworks(this))
                if (network.equals(available.network) && address.equals(available.address)
                        && prefixLength == available.prefixLength) return true;
            return false;
        } catch (SocketException unavailable) {
            throw new IOException("Connectivity service unavailable", unavailable);
        }
    }

    boolean clearNetworkSelectionFailure(String address, int prefixLength) {
        if (currentAudio != null || networkCoordinator != null || !uiOperation.canBegin()) return false;
        String ownerId = uiOperation.ownerId();
        String error = uiOperation.error();
        if (!isNetworkSelectionFailure(error)) return false;
        try { findNetworkForInterface(parseIpv4(address), prefixLength); }
        catch (IOException unavailable) { return false; }
        synchronized (networkCoordinatorLock) {
            if (currentAudio != null || networkCoordinator != null || !uiOperation.canBegin()
                    || !uiOperation.clearFailure(ownerId, error)) return false;
            networkAudioState = "stopped";
            networkAudioFailure = "";
            startFailure = "";
            return true;
        }
    }

    private static boolean isNetworkSelectionFailure(String error) {
        return "InvalidReceiverInterface".equals(error)
                || "ReceiveInterfaceChanged".equals(error)
                || "ReceiveInterfaceUnavailable".equals(error)
                || "所选 Android 网络已不可用或发生变化。".equals(error)
                || "所选网络接口已不可用，请重新选择。".equals(error);
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
        if (session == null || currentAudio != session || !session.request.usesPacketIngress())
            throw new IOException("NetworkAudioSessionNotOwned");
        PlaybackOutput output;
        boolean shouldPause = false;
        boolean shouldResume = false;
        Exception pauseRequestFailure = null;
        synchronized (session) {
            ensureRunning(session);
            AudioFocusState.Action action = session.focusState.setUserPaused(paused);
            if (session.udpQueue != null) session.udpQueue.setPaused(!session.focusState.canPlay());
            if (session.floatQueue != null) session.floatQueue.setPaused(!session.focusState.canPlay());
            output = session.playbackOutput;
            if (action == AudioFocusState.Action.PAUSE && output != null
                    && (session.trackPlaying || !session.trackReady)) {
                shouldPause = true;
                if (session.playbackOutput == output) {
                    try { output.requestPause(); }
                    catch (IOException | RuntimeException error) { pauseRequestFailure = error; }
                }
            } else if (!paused && session.trackReady && !session.trackPlaying
                    && output != null && session.focusState.canPlay()) {
                shouldResume = true;
            }
            session.notifyAll();
        }
        try {
            if (shouldPause) {
                if (pauseRequestFailure instanceof IOException) throw (IOException) pauseRequestFailure;
                if (pauseRequestFailure instanceof RuntimeException) throw (RuntimeException) pauseRequestFailure;
                pauseAndFlushOutput(session, output);
            } else if (shouldResume) {
                synchronized (session) {
                    if (session.playbackOutput == output && session.focusState.canPlay())
                        {
                            String failure = resumeFocusedOutput(session, output);
                            if (!failure.isEmpty()) throw new IOException(failure);
                        }
                }
            }
        } catch (IOException | RuntimeException error) {
            if (!noteOboeRuntimeFailureIfActive(session, output, error)) return;
            session.focusState.cancel();
            IOException failure = new IOException("Audio pause/resume failed: " + describeFailure(error), error);
            cancelAudio(session, failure.getMessage());
            throw failure;
        }
    }

    private void receiveFloatAudio(AudioSession session) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
        AudioTrack track = null;
        PlaybackOutput output = null;
        AudioManager manager = null;
        AudioFocusRequest focusRequest = null;
        AudioManager.OnAudioFocusChangeListener focusListener = null;
        FloatPcmFrameQueue queue = null;
        UdpReceiver receiver = null;
        Thread networkThread = null;
        WifiManager.WifiLock wifiLock = null;
        String failure = "";
        try {
            UdpPacketDecoder.AudioFormatV2 sourceFormat = session.request.formatV2();
            if (!supportsAudioFormatV2(sourceFormat))
                throw new IOException("Unsupported audio format for playback");
            ensureRunning(session);
            boolean lowLatency = "lowlatency".equalsIgnoreCase(session.request.playbackMode);
            if (session.request.aoaContext != null) {
                queue = session.request.aoaContext.floatQueue;
                if (queue == null) throw new IOException("AOA Float queue missing");
                synchronized (session) {
                    ensureRunning(session);
                    queue.setPaused(!session.focusState.canPlay());
                }
            } else {
                WifiManager wifi = getSystemService(WifiManager.class);
                if (wifi == null) throw new IOException("WiFi service unavailable");
                int wifiMode = Build.VERSION.SDK_INT >= 29
                        ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF;
                wifiLock = wifi.createWifiLock(wifiMode, "AnPod:UdpPlayback");
                wifiLock.acquire();
                queue = new FloatPcmFrameQueue(sourceFormat.sampleRate, lowLatency,
                        UdpPacketDecoder.MAX_PLAINTEXT_BYTES / sourceFormat.frameBytes);
                receiver = new UdpReceiver(session.request.udpContext, queue);
                UdpReceiver receiverForThread = receiver;
                networkThread = new Thread(() -> {
                    Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
                    receiverForThread.run();
                }, "AnPod Float32 UDP packet receiver");
                synchronized (session) {
                    ensureRunning(session);
                    session.floatQueue = queue;
                    queue.setPaused(!session.focusState.canPlay());
                    session.udpReceiver = receiver;
                    session.networkThread = networkThread;
                    networkThread.start();
                }
            }

            output = openOboeOutputIfRequested(session, sourceFormat.sampleRate, sourceFormat.channels,
                    lowLatency, Math.max(1, sourceFormat.sampleRate / 200));
            OpenedV2Track openedTrack = null;
            if (output == null) {
                openedTrack = openV2AudioTrack(sourceFormat.sampleRate, AudioFormat.CHANNEL_OUT_STEREO,
                        sourceFormat.sampleFormat == 3);
                output = new AudioTrackPlaybackOutput(openedTrack.track, openedTrack.formatCode, openedTrack.frameBytes);
                recordOutputOpened(session, output);
            }
            track = output.audioTrack();
            synchronized (session) {
                ensureRunning(session);
                session.playbackOutput = output;
                session.audioTrack = track;
                session.trackWrittenFrames = 0;
                if (track != null) session.playbackActivity.resetPlaybackHead(track.getPlaybackHeadPosition());
                session.openedTrackFormatV2 = output.formatCode();
                session.openedTrackChannelsV2 = output.channels();
                session.openedTrackSampleRateV2 = output.sampleRate();
            }

            int capacityFrames;
            int requestedBufferFrames;
            int acceptedBufferFrames;
            int effectiveFrames;
            int thresholdFrames;
            String configuredMode;
            if (track != null) {
                capacityFrames = output.capacityFrames();
                requestedBufferFrames = Pcm16FrameStream.requestedAudioTrackBufferFrames(
                        sourceFormat.sampleRate, Math.max(1, capacityFrames), lowLatency);
                acceptedBufferFrames = Build.VERSION.SDK_INT >= 24
                        ? track.setBufferSizeInFrames(requestedBufferFrames) : -1;
                effectiveFrames = output.effectiveBufferFrames();
                int requestedStartFrames = 0;
                boolean thresholdLimited = false;
                thresholdFrames = -1;
                if (Build.VERSION.SDK_INT >= 31 && lowLatency && acceptedBufferFrames >= 0) {
                    requestedStartFrames = Math.min(acceptedBufferFrames, Math.max(1,
                            sourceFormat.sampleRate * Pcm16FrameStream.LOW_LATENCY_START_THRESHOLD_MS / 1000));
                    int acceptedStartFrames = track.setStartThresholdInFrames(requestedStartFrames);
                    thresholdLimited = acceptedStartFrames < 0 || acceptedStartFrames != requestedStartFrames;
                    thresholdFrames = output.startThresholdFrames();
                }
                configuredMode = acceptedBufferFrames < 0 ? "balanced-fallback"
                        : acceptedBufferFrames == requestedBufferFrames && !thresholdLimited
                                ? (lowLatency ? "low-latency" : "balanced")
                                : (lowLatency ? "low-latency-limited" : "balanced-limited");
                if (lowLatency && requestedStartFrames > 0 && thresholdFrames != requestedStartFrames)
                    configuredMode = "low-latency-limited";
            } else {
                int budgetMs = lowLatency ? Pcm16FrameStream.LOW_LATENCY_AUDIO_TRACK_BUFFER_MS
                        : Pcm16FrameStream.BALANCED_AUDIO_TRACK_BUFFER_MS;
                requestedBufferFrames = sourceFormat.sampleRate * budgetMs / 1000;
                capacityFrames = output.capacityFrames();
                acceptedBufferFrames = output.effectiveBufferFrames();
                effectiveFrames = output.effectiveBufferFrames();
                thresholdFrames = -1;
                configuredMode = lowLatency ? "low-latency" : "balanced";
            }
            publishMode(session, configuredMode);
            int minBufferFrames = openedTrack == null ? Math.max(1, sourceFormat.sampleRate / 200)
                    : Math.max(1, (openedTrack.minBufferBytes + openedTrack.frameBytes - 1)
                            / openedTrack.frameBytes);
            int preRollFrames = lowLatency && thresholdFrames > 0
                    ? Math.min(effectiveFrames, thresholdFrames) : Math.min(effectiveFrames, minBufferFrames);
            if (output instanceof OboePlaybackOutput) {
                preRollFrames = Math.min(preRollFrames, output.ringCapacityFrames());
                if (preRollFrames < output.framesPerBurst())
                    throw new IOException("Oboe ring cannot hold one callback burst of pre-roll");
                requestedBufferFrames = output.info()[11];
                acceptedBufferFrames = output.effectiveBufferFrames();
                effectiveFrames = output.effectiveBufferFrames();
            }
            publishMetrics(session, output.sampleRate(), output.channels(), capacityFrames,
                    requestedBufferFrames, acceptedBufferFrames, effectiveFrames, thresholdFrames,
                    output.underrunCount(), preRollFrames, configuredMode);
            session.oboeXrunCount = output.oboeXrunCount();

            int chunkFrames = Math.max(1, sourceFormat.sampleRate / 200);
            float[] samples = new float[(chunkFrames + 1) * sourceFormat.channels];
            byte[] packed = openedTrack != null && openedTrack.formatCode == 2
                    ? new byte[(chunkFrames + 1) * openedTrack.frameBytes] : null;
            manager = getSystemService(AudioManager.class);
            final AudioSession focusSession = session;
            final PlaybackOutput focusOutput = output;
            focusListener = change -> handler.post(() ->
                    handleAudioFocusChange(focusSession, focusOutput, change));
            boolean focusRequested = false;
            while (true) {
                V2PreRollProgress preRoll = new V2PreRollProgress(session.focusState.writeGeneration());
                while (preRoll.completedFrames() < preRollFrames) {
                    ensureRunning(session);
                    long blockGeneration = session.focusState.writeGeneration();
                    preRoll.observeGeneration(blockGeneration);
                    int count = readFloatFrames(queue, samples,
                            Math.min(chunkFrames, preRollFrames - preRoll.completedFrames()));
                    if (count < 0) throw new IOException("FloatAudioStreamEndedBeforePlayback");
                    boolean complete = writeV2Frames(session, output, samples, count, packed,
                            blockGeneration);
                    long currentGeneration = session.focusState.writeGeneration();
                    preRoll.recordWrite(blockGeneration, currentGeneration, count, complete);
                }
                publishPreRollFrames(session, preRoll.completedFrames());
                synchronized (session) {
                    ensureRunning(session);
                    if (session.focusState.writeGeneration() != preRoll.generation()) continue;
                    if (!focusRequested) {
                        if (Build.VERSION.SDK_INT >= 26) {
                            focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                                    .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                                    .setWillPauseWhenDucked(true).setOnAudioFocusChangeListener(focusListener).build();
                            if (manager.requestAudioFocus(focusRequest) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
                                throw new IOException("Audio focus denied");
                        } else if (manager.requestAudioFocus(focusListener, AudioManager.STREAM_MUSIC,
                                AudioManager.AUDIOFOCUS_GAIN) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                            throw new IOException("Audio focus denied");
                        }
                        focusRequested = true;
                    }
                    ensureRunning(session);
                    session.trackReady = true;
                    if (session.focusState.canPlay()) {
                        output.play();
                        markOutputEpoch(session);
                        if (track != null && track.getPlayState() != AudioTrack.PLAYSTATE_PLAYING)
                            throw new IOException("AudioTrack did not start");
                        session.trackPlaying = true;
                        session.focusState.markTrackPlaying();
                    }
                    synchronized (STATUS_LOCK) {
                        if (statusAudioId == session.id) audioActive = session.focusState.canPlay();
                    }
                }
                break;
            }

            FloatPcmDriftController drift = new FloatPcmDriftController(sourceFormat.sampleRate,
                    queue.initialTargetFrames());
            session.floatDriftController = drift;
            long driftNow = System.nanoTime();
            drift.reset(driftNow);
            long initialPlaybackHead = output.playbackHeadPosition();
            if (initialPlaybackHead >= 0) session.driftPlaybackHead.reset((int) initialPlaybackHead, driftNow);
            long observedDriftResetGeneration = session.driftResetGeneration;
            long observedConcealedFrames = queue.concealedFrames();
            long observedDroppedFrames = queue.droppedFrames();
            long observedExpiredFrames = queue.expiredFrames();
            long observedOversizedGapSkips = queue.oversizedGapSkips();
            int observedUnderruns = output.discontinuityCount();
            session.oboeXrunCount = output.oboeXrunCount();

            while (true) {
                ensureRunning(session);
                awaitFocusResume(session);
                long blockGeneration = session.focusState.writeGeneration();
                int count = readFloatFrames(queue, samples, chunkFrames);
                if (count < 0) {
                    failure = "FloatAudioStreamEnded";
                    break;
                }
                if (!session.focusState.acceptsWrite(blockGeneration)) continue;
                long nowNanos = System.nanoTime();
                long playbackPosition = output.playbackHeadPosition();
                long playedFrames = playbackPosition < 0 ? 0
                        : session.driftPlaybackHead.update((int) playbackPosition, nowNanos);
                long concealedFrames = queue.concealedFrames();
                long droppedFrames = queue.droppedFrames();
                long expiredFrames = queue.expiredFrames();
                long oversizedGapSkips = queue.oversizedGapSkips();
                int underrunsBefore = output.discontinuityCount();
                session.oboeXrunCount = output.oboeXrunCount();
                boolean discontinuity = concealedFrames != observedConcealedFrames
                        || droppedFrames != observedDroppedFrames
                        || expiredFrames != observedExpiredFrames
                        || oversizedGapSkips != observedOversizedGapSkips
                        || (observedUnderruns >= 0 && underrunsBefore >= 0
                                && observedUnderruns != underrunsBefore)
                        || observedDriftResetGeneration != session.driftResetGeneration;
                boolean playing = session.trackPlaying
                        && (playbackPosition < 0 || session.driftPlaybackHead.isAdvancing(nowNanos))
                        && concealedFrames == observedConcealedFrames;
                long pendingFrames = output.pendingFrames(session.trackWrittenFrames, playedFrames);
                long waterlineFrames = (long) queue.queuedFrames() + count + pendingFrames;
                int adjustment = drift.observe(nowNanos, waterlineFrames, playing, discontinuity);
                int outputFrames = count;
                if (adjustment != 0 && (adjustment > 0 ? count >= 3 : count >= 2)) {
                    outputFrames = queue.adjustDrift(samples, count, chunkFrames + 1, adjustment);
                    drift.commit(adjustment);
                }
                writeV2Frames(session, output, samples, outputFrames, packed, blockGeneration);
                observedDriftResetGeneration = session.driftResetGeneration;
                observedConcealedFrames = concealedFrames;
                observedDroppedFrames = droppedFrames;
                observedExpiredFrames = expiredFrames;
                observedOversizedGapSkips = oversizedGapSkips;
                observedUnderruns = underrunsBefore;
                int reportedUnderruns = output.underrunCount();
                if (reportedUnderruns >= 0) {
                    synchronized (STATUS_LOCK) {
                        if (statusAudioId == session.id) {
                            underrunIncrement = underrunCount < 0 || reportedUnderruns < underrunCount
                                    ? -1 : reportedUnderruns - underrunCount;
                            underrunCount = reportedUnderruns;
                        }
                    }
                }
            }
        } catch (IOException | RuntimeException error) {
            noteOboeRuntimeFailure(session, error);
            failure = session.cancelled ? session.stopReason : describeFailure(error);
        } finally {
            session.trackPlaying = false;
            String trackReleaseFailure = "";
            synchronized (session) {
                if (output != null) {
                    try { output.close(); }
                    catch (IOException | RuntimeException error) {
                        noteOboeRuntimeFailure(session, error);
                        try { output.close(); }
                        catch (IOException | RuntimeException retryError) {
                            error.addSuppressed(retryError);
                            noteOboeRuntimeFailure(session, retryError);
                            retainDeferredOboeClose(output);
                            trackReleaseFailure = "Audio output close failed: " + describeFailure(error);
                        }
                    }
                }
                if (session.audioTrack == track) session.audioTrack = null;
                if (session.playbackOutput == output) session.playbackOutput = null;
                session.openedTrackFormatV2 = 0;
                session.openedTrackChannelsV2 = 0;
                session.openedTrackSampleRateV2 = 0;
            }
            String focusReleaseFailure = abandonAudioFocus(manager, focusRequest, focusListener);
            if (!trackReleaseFailure.isEmpty()) failure = appendFailure(failure, trackReleaseFailure);
            if (!focusReleaseFailure.isEmpty()) failure = appendFailure(failure, focusReleaseFailure);
            if (receiver != null) receiver.close();
            if (queue != null) queue.stop();
            if (wifiLock != null) {
                try { if (wifiLock.isHeld()) wifiLock.release(); }
                catch (RuntimeException error) { failure = appendFailure(failure, "WiFiLockRelease: " + describeFailure(error)); }
            }
            joinNetworkThread(networkThread);
            session.request.destroy();
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) {
                    audioActive = false;
                    if (failure.isEmpty()) failure = session.stopReason.isEmpty() ? "AudioSessionEnded" : session.stopReason;
                }
            }
            synchronized (session) {
                if (session.udpReceiver == receiver) session.udpReceiver = null;
                if (session.floatQueue == queue) session.floatQueue = null;
                if (session.networkThread == networkThread) session.networkThread = null;
            }
            String finalFailure = failure;
            handler.post(() -> finishAudio(session, finalFailure));
        }
    }

    static boolean supportsPcm24AudioFormatV2(UdpPacketDecoder.AudioFormatV2 format) {
        return format != null && (format.profile == 2 && format.sampleRate == 48_000
                || format.profile == 3 && format.sampleRate == 96_000)
                && format.channels == 2 && format.sampleFormat == 2 && format.validBits == 24
                && format.bytesPerSample == Pcm24PackedCodec.BYTES_PER_SAMPLE && format.frameBytes == 6;
    }

    static boolean supportsFloat32AudioFormatV2(UdpPacketDecoder.AudioFormatV2 format) {
        return format != null && (format.profile == 4 && format.sampleRate == 48_000
                || format.profile == 5 && format.sampleRate == 96_000)
                && format.channels == 2 && format.sampleFormat == 3 && format.validBits == 32
                && format.bytesPerSample == Float.BYTES && format.frameBytes == 8;
    }

    static boolean supportsAudioFormatV2(UdpPacketDecoder.AudioFormatV2 format) {
        return supportsPcm24AudioFormatV2(format) || supportsFloat32AudioFormatV2(format);
    }

    String requestedOutputEnginePreference() {
        String value = getSharedPreferences("anpod_receiver", MODE_PRIVATE)
                .getString("output_engine", "oboeExperimental");
        return "oboeExperimental".equals(value) ? "oboeExperimental" : "audioTrack";
    }

    PlaybackOutput openOboeOutputIfRequested(AudioSession session, int sampleRate, int channels,
            boolean lowLatency, int maxWriteFrames) {
        if (!"oboeExperimental".equals(session.requestedOutputEngine)) return null;
        boolean previousClosed = retryDeferredOboeClose();
        String nextSessionReason;
        synchronized (OBOE_OUTPUT_LOCK) {
            nextSessionReason = pendingOboeFallbackReason;
            pendingOboeFallbackReason = "";
        }
        if (!nextSessionReason.isEmpty()) {
            session.outputFallbackReason = nextSessionReason;
            return null;
        }
        if (!previousClosed) {
            session.outputFallbackReason = "Previous Oboe stream has not closed; using AudioTrack";
            return null;
        }
        int budgetMs = lowLatency ? Pcm16FrameStream.LOW_LATENCY_AUDIO_TRACK_BUFFER_MS
                : Pcm16FrameStream.BALANCED_AUDIO_TRACK_BUFFER_MS;
        int totalBudgetFrames = sampleRate * budgetMs / 1000;
        try {
            PlaybackOutput output = OboePlaybackOutput.open(sampleRate, channels, totalBudgetFrames, maxWriteFrames);
            session.playbackOutput = output;
            session.actualOutputEngine = output.engine();
            session.outputEpoch = 1;
            session.outputFallbackReason = "";
            return output;
        } catch (IOException | RuntimeException | UnsatisfiedLinkError error) {
            session.outputFallbackReason = describeFailure(error);
            return null;
        }
    }

    private static boolean retryDeferredOboeClose() {
        OboePlaybackOutput deferred;
        synchronized (OBOE_OUTPUT_LOCK) { deferred = deferredOboeClose; }
        if (deferred == null) return true;
        try {
            deferred.close();
            synchronized (OBOE_OUTPUT_LOCK) {
                if (deferredOboeClose == deferred) deferredOboeClose = null;
            }
            return true;
        } catch (IOException | RuntimeException error) {
            return false;
        }
    }

    static void retainDeferredOboeClose(PlaybackOutput output) {
        if (!(output instanceof OboePlaybackOutput)) return;
        synchronized (OBOE_OUTPUT_LOCK) {
            if (deferredOboeClose == null) deferredOboeClose = (OboePlaybackOutput) output;
        }
    }

    static void noteOboeRuntimeFailure(AudioSession session, Throwable error) {
        if (!(error instanceof OboeOutputException)
                || !"oboeExperimental".equals(session.actualOutputEngine)) return;
        String reason = describeFailure((Exception) error);
        session.outputFallbackReason = reason;
        synchronized (OBOE_OUTPUT_LOCK) {
            if (pendingOboeFallbackReason.isEmpty()) pendingOboeFallbackReason = reason;
        }
    }

    private static boolean noteOboeRuntimeFailureIfActive(AudioSession session, PlaybackOutput output,
            Throwable error) {
        synchronized (session) {
            if (currentAudio != session || session.cancelled || session.playbackOutput != output) return false;
            noteOboeRuntimeFailure(session, error);
            return true;
        }
    }

    static String pendingOboeFallbackReasonForUi() {
        synchronized (OBOE_OUTPUT_LOCK) { return pendingOboeFallbackReason; }
    }

    static void recordOutputOpened(AudioSession session, PlaybackOutput output) {
        session.playbackOutput = output;
        session.audioTrack = output.audioTrack();
        session.actualOutputEngine = output.engine();
        session.outputEpoch = 1;
    }

    static OpenedV2Track openV2AudioTrack(int sampleRate, int channelMask) throws IOException {
        return openV2AudioTrack(sampleRate, channelMask, false);
    }

    static OpenedV2Track openV2AudioTrack(int sampleRate, int channelMask, boolean preferFloat)
            throws IOException {
        return chooseV2AudioTrack(Build.VERSION.SDK_INT, sampleRate, channelMask, preferFloat,
                ReceiverService::buildV2AudioTrack);
    }

    interface V2TrackOpener {
        OpenedV2Track open(int sampleRate, int channelMask, int encoding, int formatCode) throws IOException;
    }

    static OpenedV2Track chooseV2AudioTrack(int sdk, int sampleRate, int channelMask,
            V2TrackOpener opener) throws IOException {
        return chooseV2AudioTrack(sdk, sampleRate, channelMask, false, opener);
    }

    static OpenedV2Track chooseV2AudioTrack(int sdk, int sampleRate, int channelMask,
            boolean preferFloat, V2TrackOpener opener) throws IOException {
        if (preferFloat) {
            try { return opener.open(sampleRate, channelMask, AudioFormat.ENCODING_PCM_FLOAT, 3); }
            catch (IOException | RuntimeException error) {
                throw error instanceof IOException ? (IOException) error
                        : new IOException("Float32 AudioTrack could not be opened", error);
            }
        }
        IOException packed24Failure = null;
        if (sdk >= 31) {
            try { return opener.open(sampleRate, channelMask, AudioFormat.ENCODING_PCM_24BIT_PACKED, 2); }
            catch (IOException | RuntimeException error) {
                packed24Failure = error instanceof IOException
                        ? (IOException) error : new IOException("Packed PCM24 AudioTrack could not be opened", error);
            }
        }
        try { return opener.open(sampleRate, channelMask, AudioFormat.ENCODING_PCM_FLOAT, 3); }
        catch (IOException | RuntimeException floatFailure) {
            IOException failure = new IOException("Neither packed PCM24 nor Float32 AudioTrack is supported", floatFailure);
            if (packed24Failure != null) failure.addSuppressed(packed24Failure);
            throw failure;
        }
    }

    private static OpenedV2Track buildV2AudioTrack(int sampleRate, int channelMask, int encoding, int formatCode)
            throws IOException {
        int minBufferBytes;
        try { minBufferBytes = AudioTrack.getMinBufferSize(sampleRate, channelMask, encoding); }
        catch (RuntimeException error) { throw new IOException("AudioTrack format probe failed", error); }
        if (minBufferBytes <= 0) throw new IOException("AudioTrack format is unsupported: " + encoding);
        AudioTrack track = null;
        try {
            AudioTrack.Builder builder = new AudioTrack.Builder()
                    .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                    .setAudioFormat(new AudioFormat.Builder().setEncoding(encoding)
                            .setSampleRate(sampleRate).setChannelMask(channelMask).build())
                    .setBufferSizeInBytes(minBufferBytes * 2).setTransferMode(AudioTrack.MODE_STREAM);
            if (Build.VERSION.SDK_INT >= 26) builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY);
            track = builder.build();
            if (track.getState() != AudioTrack.STATE_INITIALIZED || track.getAudioFormat() != encoding)
                throw new IOException("AudioTrack initialized with a different format");
            int channels = track.getChannelCount();
            if (channels != 2) throw new IOException("AudioTrack did not open stereo output");
            if (track.getSampleRate() != sampleRate)
                throw new IOException("AudioTrack opened a different sample rate");
            int frameBytes = channels * (encoding == AudioFormat.ENCODING_PCM_FLOAT ? 4 : 3);
            return new OpenedV2Track(track, formatCode, minBufferBytes, frameBytes);
        } catch (IOException | RuntimeException error) {
            releaseTrack(track);
            if (error instanceof IOException) throw (IOException) error;
            throw new IOException("AudioTrack format could not be opened: " + encoding, error);
        }
    }

    static final class OpenedV2Track {
        final AudioTrack track;
        final int formatCode;
        final int minBufferBytes;
        final int frameBytes;
        OpenedV2Track(AudioTrack track, int formatCode, int minBufferBytes, int frameBytes) {
            this.track = track;
            this.formatCode = formatCode;
            this.minBufferBytes = minBufferBytes;
            this.frameBytes = frameBytes;
        }
    }

    private static int readFloatFrames(FloatPcmFrameQueue queue, float[] samples, int maxFrames) throws IOException {
        try { return queue.readFrames(samples, 0, maxFrames); }
        catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new IOException("Float32 playback worker interrupted", interrupted);
        }
    }

    boolean writeV2Frames(AudioSession session, AudioTrack track, OpenedV2Track openedTrack,
            float[] samples, int frameCount, byte[] packed, long generation) throws IOException {
        if (frameCount <= 0) return true;
        return writeV2WithGeneration(session.focusState, generation, frameCount, (offsetFrames, remainingFrames) -> {
            ensureRunning(session);
            int writtenFrames;
            int writtenBytes;
            if (openedTrack.formatCode == 2) {
                Pcm24PackedCodec.encodeFromFloat(samples, offsetFrames * 2, remainingFrames * 2,
                        packed, 0);
                int written = track.write(packed, 0, remainingFrames * openedTrack.frameBytes,
                        AudioTrack.WRITE_BLOCKING);
                if (written <= 0) throw new IOException("AudioTrack packed PCM24 write failed: " + written);
                if (written % openedTrack.frameBytes != 0)
                    throw new IOException("AudioTrack returned a partial packed PCM24 frame");
                writtenBytes = written;
                writtenFrames = written / openedTrack.frameBytes;
            } else {
                int written = track.write(samples, offsetFrames * 2, remainingFrames * 2,
                        AudioTrack.WRITE_BLOCKING);
                if (written <= 0) throw new IOException("AudioTrack Float32 write failed: " + written);
                if (written % 2 != 0) throw new IOException("AudioTrack returned a partial Float32 frame");
                writtenBytes = written * Float.BYTES;
                writtenFrames = written / 2;
            }
            synchronized (session) {
                if (!session.focusState.acceptsWrite(generation)) return writtenFrames;
                ensureRunning(session);
                session.trackWrittenFrames += writtenFrames;
                PCM_BYTES_WRITTEN.addAndGet(writtenBytes);
                session.playbackActivity.noteSuccessfulWrite(System.nanoTime());
            }
            WAVEFORM.accept(samples, offsetFrames * 2, writtenFrames, 2, System.nanoTime());
            return writtenFrames;
        });
    }

    boolean writeV2Frames(AudioSession session, PlaybackOutput output, float[] samples,
            int frameCount, byte[] packed, long generation) throws IOException {
        if (frameCount <= 0) return true;
        if (output instanceof OboePlaybackOutput) {
            int offset = 0;
            while (offset < frameCount) {
                ensureRunning(session);
                if (!session.focusState.acceptsWrite(generation)) return false;
                int written = output.writeFloatFrames(samples, offset, frameCount - offset, packed);
                if (written < 0 || written > frameCount - offset)
                    throw new IOException("Oboe returned an invalid partial frame count: " + written);
                if (written == 0) {
                    java.util.concurrent.locks.LockSupport.parkNanos(250_000L);
                    if (Thread.currentThread().isInterrupted())
                        throw new IOException("Oboe output write interrupted");
                    continue;
                }
                synchronized (session) {
                    if (!session.focusState.acceptsWrite(generation)) return false;
                    ensureRunning(session);
                    session.trackWrittenFrames += written;
                    PCM_BYTES_WRITTEN.addAndGet(written * output.channels() * Float.BYTES);
                    session.playbackActivity.noteSuccessfulWrite(System.nanoTime());
                }
                WAVEFORM.accept(samples, offset * output.channels(), written, output.channels(), System.nanoTime());
                offset += written;
                if (!session.focusState.acceptsWrite(generation)) return false;
            }
            return true;
        }
        return writeV2WithGeneration(session.focusState, generation, frameCount, (offsetFrames, remainingFrames) -> {
            ensureRunning(session);
            int writtenFrames = output.writeFloatFrames(samples, offsetFrames, remainingFrames, packed);
            if (writtenFrames <= 0) throw new IOException("AudioTrack V2 write failed: " + writtenFrames);
            synchronized (session) {
                if (!session.focusState.acceptsWrite(generation)) return writtenFrames;
                ensureRunning(session);
                session.trackWrittenFrames += writtenFrames;
                PCM_BYTES_WRITTEN.addAndGet(writtenFrames * output.channels()
                        * (output.formatCode() == 2 ? 3 : Float.BYTES));
                session.playbackActivity.noteSuccessfulWrite(System.nanoTime());
            }
            WAVEFORM.accept(samples, offsetFrames * output.channels(), writtenFrames,
                    output.channels(), System.nanoTime());
            return writtenFrames;
        });
    }

    interface V2PartialFrameWriter {
        int write(int offsetFrames, int remainingFrames) throws IOException;
    }

    static final class V2PreRollProgress {
        private long generation;
        private int completedFrames;

        V2PreRollProgress(long generation) { this.generation = generation; }

        void observeGeneration(long currentGeneration) {
            if (currentGeneration != generation) {
                generation = currentGeneration;
                completedFrames = 0;
            }
        }

        void recordWrite(long blockGeneration, long currentGeneration, int frames, boolean complete) {
            if (blockGeneration != generation || currentGeneration != generation) {
                generation = currentGeneration;
                completedFrames = 0;
            } else if (complete) {
                completedFrames += frames;
            }
        }

        long generation() { return generation; }
        int completedFrames() { return completedFrames; }
    }

    static boolean writeV2WithGeneration(AudioFocusState focusState, long generation, int frameCount,
            V2PartialFrameWriter writer) throws IOException {
        int offset = 0;
        while (offset < frameCount) {
            if (!focusState.acceptsWrite(generation)) return false;
            int writtenFrames = writer.write(offset, frameCount - offset);
            if (writtenFrames <= 0 || writtenFrames > frameCount - offset)
                throw new IOException("AudioTrack returned an invalid partial frame count");
            offset += writtenFrames;
            if (!focusState.acceptsWrite(generation)) return false;
        }
        return true;
    }

    private static void joinNetworkThread(Thread networkThread) {
        if (networkThread == null || networkThread == Thread.currentThread()) return;
        boolean interrupted = Thread.interrupted();
        while (networkThread.isAlive()) {
            try { networkThread.join(); }
            catch (InterruptedException ignored) { interrupted = true; }
        }
        if (interrupted) Thread.currentThread().interrupt();
    }

    private void receiveAudio(AudioSession session) {
        if (!retryDeferredOboeClose())
            session.outputFallbackReason = "Previous Oboe stream has not closed; retaining AudioTrack output";
        if (session.request.usesFloatV2Worker()) {
            receiveFloatAudio(session);
            return;
        }
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
        AudioTrack track = null;
        PlaybackOutput output = null;
        AudioManager manager = null;
        AudioFocusRequest focusRequest = null;
        AudioManager.OnAudioFocusChangeListener focusListener = null;
        UdpPcmQueue udpQueue = null;
        UdpReceiver udpReceiver = null;
        Thread networkThread = null;
        WifiManager.WifiLock wifiLock = null;
        String failure = "";
        try {
            ensureRunning(session);
            LocalSocket ownedSocket = session.request.usesPacketIngress() ? null : new LocalSocket();
            try (ownedSocket) {
                InputStream input;
                UdpPcmQueue.QueueInputStream udpPcmInput = null;
                if (session.request.usesPacketIngress()) {
                    if (session.request.aoaContext != null) {
                        udpQueue = session.request.aoaContext.pcmQueue;
                        if (udpQueue == null) throw new IOException("AOA PCM16 queue missing");
                        synchronized (session) {
                            ensureRunning(session);
                            udpQueue.setPaused(!session.focusState.canPlay());
                        }
                    } else {
                        WifiManager wifi = getSystemService(WifiManager.class);
                        if (wifi == null) throw new IOException("WiFi service unavailable");
                        int wifiMode = Build.VERSION.SDK_INT >= 29
                                ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF;
                        wifiLock = wifi.createWifiLock(wifiMode, "AnPod:UdpPlayback");
                        wifiLock.acquire();
                        udpQueue = new UdpPcmQueue(session.request.udpContext.parameters,
                                session.request.udpContext.playbackMode);
                        udpReceiver = new UdpReceiver(session.request.udpContext, udpQueue);
                        UdpReceiver receiverForThread = udpReceiver;
                        networkThread = new Thread(() -> {
                            Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);
                            receiverForThread.run();
                        }, "AnPod UDP packet receiver");
                        synchronized (session) {
                            ensureRunning(session);
                            session.udpQueue = udpQueue;
                            udpQueue.setPaused(!session.focusState.canPlay());
                            session.udpReceiver = udpReceiver;
                            session.networkThread = networkThread;
                            networkThread.start();
                        }
                    }
                    udpPcmInput = (UdpPcmQueue.QueueInputStream) udpQueue.asPcmInputStream();
                    input = udpPcmInput;
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
                boolean lowLatency = "lowlatency".equalsIgnoreCase(session.request.playbackMode);
                int maxWriteFrames = Math.max(1, sampleRate / 50);
                output = openOboeOutputIfRequested(session, sampleRate, channels, lowLatency, maxWriteFrames);
                int capacityFrames;
                int requestedBufferFrames = -1;
                int acceptedBufferFrames = -1;
                int requestedStartFrames = -1;
                boolean startThresholdRequestLimited = false;
                String configuredMode;
                int minBufferFrames = Math.max(1, (minBuffer + frameBytes - 1) / frameBytes);
                if (output == null) synchronized (session) {
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
                    output = new AudioTrackPlaybackOutput(track, 1, frameBytes);
                    recordOutputOpened(session, output);
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
                } else {
                    recordOutputOpened(session, output);
                    session.trackWrittenFrames = 0;
                    capacityFrames = output.capacityFrames();
                    int budgetMs = lowLatency ? Pcm16FrameStream.LOW_LATENCY_AUDIO_TRACK_BUFFER_MS
                            : Pcm16FrameStream.BALANCED_AUDIO_TRACK_BUFFER_MS;
                    requestedBufferFrames = sampleRate * budgetMs / 1000;
                    acceptedBufferFrames = output.effectiveBufferFrames();
                    configuredMode = lowLatency ? "low-latency" : "balanced";
                    publishMode(session, configuredMode);
                }
                if (session.request.formatV2() != null) {
                    session.openedTrackFormatV2 = output.formatCode();
                    session.openedTrackChannelsV2 = output.channels();
                    session.openedTrackSampleRateV2 = output.sampleRate();
                }

                int driftReserveFrames = Pcm16FrameStream.driftReserveFrames(sampleRate);
                int driftReserveBytes = driftReserveFrames * frameBytes;
                int sourceBufferBytes = Math.max(frameBytes, minBuffer + frameBytes - 1);
                byte[] buffer = new byte[sourceBufferBytes + driftReserveBytes];
                byte[] adjustedReadScratch = new byte[buffer.length];
                Pcm16FrameStream pcm = new Pcm16FrameStream(channels);
                boolean lowLatencyBacklog = currentMode(session).startsWith("low-latency");
                int backlogTargetFrames = Pcm16FrameStream.backlogTargetFrames(
                        output.sampleRate(), lowLatencyBacklog);
                int driftReadTargetFrames = udpQueue == null ? backlogTargetFrames
                        : Math.max(1, output.sampleRate() / 200);
                int driftReadLimitBytes = Math.min(buffer.length - driftReserveBytes,
                        driftReadTargetFrames * frameBytes);
                driftReadLimitBytes -= driftReadLimitBytes % frameBytes;
                driftReadLimitBytes = Math.max(frameBytes, driftReadLimitBytes);
                int effectiveFrames = output.effectiveBufferFrames();
                int thresholdFrames = output.startThresholdFrames();
                int preRollFrames = currentMode(session).startsWith("low-latency") && thresholdFrames > 0
                        ? Math.min(effectiveFrames, thresholdFrames) : Math.min(effectiveFrames, minBufferFrames);
                if (output instanceof OboePlaybackOutput) {
                    preRollFrames = Math.min(preRollFrames, output.ringCapacityFrames());
                    if (preRollFrames < output.framesPerBurst())
                        throw new IOException("Oboe ring cannot hold one callback burst of pre-roll");
                    requestedBufferFrames = output.info()[11];
                    acceptedBufferFrames = output.effectiveBufferFrames();
                    effectiveFrames = output.effectiveBufferFrames();
                }
                publishMetrics(session, output.sampleRate(), output.channels(), capacityFrames, requestedBufferFrames,
                        acceptedBufferFrames, effectiveFrames, thresholdFrames,
                        output.underrunCount(), preRollFrames, currentMode(session));
                session.oboeXrunCount = output.oboeXrunCount();
                if (udpQueue != null) {
                    udpQueue.configureAudioTrack(preRollFrames);
                } else {
                    int backlogTriggerFrames = Pcm16FrameStream.backlogTriggerFrames(
                            output.sampleRate(), lowLatencyBacklog);
                    pcm.setBacklogLimit(backlogTargetFrames, backlogTriggerFrames,
                            (output.sampleRate() * 3 + 999) / 1000);
                    synchronized (STATUS_LOCK) {
                        if (statusAudioId == session.id) tcpDroppedBacklogFrames = 0;
                    }
                }
                int preRollBytes = preRollFrames * frameBytes;
                int preRolled = 0;
                while (preRolled < preRollBytes) {
                    ensureRunning(session);
                    long blockGeneration = session.focusState.writeGeneration();
                    int count = pcm.readFrames(input, buffer, Math.min(buffer.length, preRollBytes - preRolled));
                    if (count < 0) throw new IOException("AudioStreamEndedBeforePlayback");
                    if (!session.focusState.acceptsWrite(blockGeneration)) { preRolled = 0; continue; }
                    publishTcpBacklogMetrics(session, udpQueue, pcm);
                    if (writeFrames(session, output, pcm, buffer, count, frameBytes, blockGeneration))
                        preRolled += count;
                    else preRolled = 0;
                }
                publishPreRollFrames(session, preRolled / frameBytes);

                manager = getSystemService(AudioManager.class);
                final AudioSession focusSession = session;
                final PlaybackOutput focusOutput = output;
                focusListener = change -> handler.post(() ->
                        handleAudioFocusChange(focusSession, focusOutput, change));
                boolean driftStarted = false;
                synchronized (session) {
                    ensureRunning(session);
                    if (Build.VERSION.SDK_INT >= 26) {
                        focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                                .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                                .setWillPauseWhenDucked(true)
                                .setOnAudioFocusChangeListener(focusListener).build();
                        if (manager.requestAudioFocus(focusRequest) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
                            throw new IOException("Audio focus denied");
                    } else if (manager.requestAudioFocus(focusListener, AudioManager.STREAM_MUSIC,
                            AudioManager.AUDIOFOCUS_GAIN) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                        throw new IOException("Audio focus denied");
                    }
                    ensureRunning(session);
                    session.trackReady = true;
                    if (session.focusState.canPlay()) {
                        long driftStartNanos = System.nanoTime();
                        long playbackHead = output.playbackHeadPosition();
                        if (playbackHead >= 0) session.driftPlaybackHead.reset((int) playbackHead, driftStartNanos);
                        output.play();
                        markOutputEpoch(session);
                        if (track != null && track.getPlayState() != AudioTrack.PLAYSTATE_PLAYING)
                            throw new IOException("AudioTrack did not start");
                        session.trackPlaying = true;
                        session.focusState.markTrackPlaying();
                    }
                    session.driftResetGeneration++;
                    if (session.trackPlaying) {
                        long driftStartNanos = System.nanoTime();
                        session.driftLastOutputDiscontinuities = output.discontinuityCount();
                        pcm.beginDriftCorrection(output.sampleRate(), udpQueue == null
                                ? backlogTargetFrames : udpQueue.targetFrames(), driftStartNanos);
                        driftStarted = true;
                    }
                    synchronized (STATUS_LOCK) {
                        if (statusAudioId == session.id) audioActive = session.focusState.canPlay();
                    }
                }
                long observedDriftResetGeneration = session.driftResetGeneration;
                long observedUdpDropped = udpQueue == null ? 0 : udpQueue.droppedFrames();
                long observedUdpExpired = udpQueue == null ? 0 : udpQueue.expiredFrames();
                long observedUdpOversizedGaps = udpQueue == null ? 0 : udpQueue.plcOversizedGapSkips();
                long observedUdpPayloadGeneration = udpQueue == null ? 0 : udpQueue.payloadGeneration();
                long driftObservedPlayedFrames = 0;
                boolean driftNeedsRealAudio = false;
                while (true) {
                    ensureRunning(session);
                    awaitFocusResume(session);
                    long driftResetGeneration = session.driftResetGeneration;
                    long nowNanos = System.nanoTime();
                    if (driftResetGeneration != observedDriftResetGeneration) {
                        long head = output.playbackHeadPosition();
                        if (head >= 0) driftObservedPlayedFrames = session.driftPlaybackHead.update((int) head, nowNanos);
                        int underruns = output.discontinuityCount();
                        session.driftLastOutputDiscontinuities = underruns;
                        pcm.beginDriftCorrection(output.sampleRate(), udpQueue == null
                                ? backlogTargetFrames : udpQueue.targetFrames(), nowNanos);
                        observedDriftResetGeneration = driftResetGeneration;
                        driftNeedsRealAudio = false;
                        driftStarted = true;
                    }
                    if (!driftStarted) {
                        nowNanos = System.nanoTime();
                        long head = output.playbackHeadPosition();
                        if (head >= 0) session.driftPlaybackHead.reset((int) head, nowNanos);
                        driftObservedPlayedFrames = 0;
                        session.driftLastOutputDiscontinuities = output.discontinuityCount();
                        pcm.beginDriftCorrection(output.sampleRate(), udpQueue == null
                                ? backlogTargetFrames : udpQueue.targetFrames(), nowNanos);
                        driftStarted = true;
                    }

                    long droppedBacklogBefore = pcm.getDroppedBacklogFrames();
                    long udpDroppedBefore = udpQueue == null ? 0 : udpQueue.droppedFrames();
                    long udpExpiredBefore = udpQueue == null ? 0 : udpQueue.expiredFrames();
                    long udpSourceDeliveredBefore = udpPcmInput == null ? 0
                            : udpPcmInput.sourceBytesDelivered();
                    long udpSyntheticDeliveredBefore = udpPcmInput == null ? 0
                            : udpPcmInput.syntheticBytesDelivered();
                    long udpOversizedGapBefore = udpQueue == null ? 0 : udpQueue.plcOversizedGapSkips();
                    long udpPayloadGenerationBefore = udpQueue == null ? 0 : udpQueue.payloadGeneration();
                    boolean udpDiscontinuityBeforeRead = udpQueue != null
                            && (udpDroppedBefore != observedUdpDropped
                                    || udpExpiredBefore != observedUdpExpired
                                    || udpOversizedGapBefore != observedUdpOversizedGaps
                                    || udpPayloadGenerationBefore != observedUdpPayloadGeneration);
                    long readHead = output.playbackHeadPosition();
                    if (readHead >= 0) session.driftPlaybackHead.update((int) readHead, nowNanos);
                    int previousUnderruns = output.discontinuityCount();
                    boolean headAdvancing = readHead >= 0 && session.driftPlaybackHead.isAdvancing(nowNanos);
                    boolean underrunBeforeRead = session.driftLastOutputDiscontinuities >= 0 && previousUnderruns >= 0
                            && previousUnderruns != session.driftLastOutputDiscontinuities;
                    int sourceReadAdjustmentFrames = headAdvancing && !underrunBeforeRead
                            && !driftNeedsRealAudio
                                    ? pcm.driftReadFrameAdjustment(nowNanos) : 0;
                    int sourceReadLimitBytes = driftReadLimitBytes
                            + sourceReadAdjustmentFrames * frameBytes;
                    long blockGeneration = session.focusState.writeGeneration();
                    int count = pcm.readFrames(input, buffer, sourceReadLimitBytes);
                    if (!session.focusState.acceptsWrite(blockGeneration)) continue;
                    if ((sourceReadAdjustmentFrames != 0 || udpQueue != null) && count >= 0) {
                        int totalRead = count;
                        while (totalRead < sourceReadLimitBytes) {
                            ensureRunning(session);
                            int additionalRead = pcm.readFrames(input, adjustedReadScratch,
                                    sourceReadLimitBytes - totalRead);
                            if (additionalRead < 0) break;
                            System.arraycopy(adjustedReadScratch, 0, buffer, totalRead, additionalRead);
                            totalRead += additionalRead;
                        }
                        count = totalRead;
                    }
                    if (count < 0) {
                        failure = "AudioStreamEnded";
                        break;
                    }
                    publishTcpBacklogMetrics(session, udpQueue, pcm);
                    nowNanos = System.nanoTime();
                    long playbackPosition = output.playbackHeadPosition();
                    long playedFrames = playbackPosition < 0 ? driftObservedPlayedFrames
                            : session.driftPlaybackHead.update((int) playbackPosition, nowNanos);
                    int underruns = output.discontinuityCount();
                    int trackUnderruns = output.underrunCount();
                    session.oboeXrunCount = output.oboeXrunCount();
                    boolean playbackAdvancing = playbackPosition >= 0
                            && session.driftPlaybackHead.isAdvancing(nowNanos);
                    boolean headAdvanced = playedFrames > driftObservedPlayedFrames;
                    if (headAdvanced) driftObservedPlayedFrames = playedFrames;
                    boolean underrunChanged = previousUnderruns >= 0 && underruns >= 0
                            && underruns != previousUnderruns;
                    long udpDroppedAfter = udpQueue == null ? udpDroppedBefore : udpQueue.droppedFrames();
                    long udpExpiredAfter = udpQueue == null ? udpExpiredBefore : udpQueue.expiredFrames();
                    long udpOversizedGapAfter = udpQueue == null ? udpOversizedGapBefore
                            : udpQueue.plcOversizedGapSkips();
                    long udpPayloadGenerationAfter = udpQueue == null ? udpPayloadGenerationBefore
                            : udpQueue.payloadGeneration();
                    long udpSourceDelivered = udpPcmInput == null ? 0
                            : udpPcmInput.sourceBytesDelivered() - udpSourceDeliveredBefore;
                    long udpSyntheticDelivered = udpPcmInput == null ? 0
                            : udpPcmInput.syntheticBytesDelivered() - udpSyntheticDeliveredBefore;
                    boolean syntheticUdpBlock = udpSyntheticDelivered > 0;
                    if (syntheticUdpBlock) driftNeedsRealAudio = true;
                    boolean resumeAfterSynthetic = udpQueue != null && driftNeedsRealAudio
                            && !syntheticUdpBlock && udpSourceDelivered > 0;
                    boolean discontinuity = pcm.getDroppedBacklogFrames() != droppedBacklogBefore
                            || (udpQueue != null && (udpDroppedAfter != udpDroppedBefore
                                    || udpExpiredAfter != udpExpiredBefore
                                    || udpOversizedGapAfter != udpOversizedGapBefore
                                    || udpPayloadGenerationAfter != udpPayloadGenerationBefore
                                    || udpDiscontinuityBeforeRead))
                            || underrunChanged
                            || underrunBeforeRead
                            || session.driftResetGeneration != observedDriftResetGeneration;
                    boolean readAdjustmentApplied = sourceReadAdjustmentFrames != 0
                            && count == sourceReadLimitBytes
                            && (udpPcmInput == null
                                    || udpSourceDelivered + udpSyntheticDelivered == count);
                    int outputBytes = count;
                    int driftSourceTargetFrames = udpQueue == null
                            ? backlogTargetFrames : udpQueue.targetFrames();
                    long sourceBufferedFrames = udpQueue == null
                            ? Math.max(0, input.available() / frameBytes) : udpQueue.sourceBufferedFrames();
                    long pendingOutputFrames = output.pendingFrames(session.trackWrittenFrames, playedFrames);
                    if (pendingOutputFrames < 0) {
                        pcm.resetDriftCorrection();
                        driftStarted = false;
                        outputBytes = count;
                    } else {
                        outputBytes = pcm.correctDrift(buffer, count, sourceBufferedFrames,
                                pendingOutputFrames, driftSourceTargetFrames, nowNanos, playbackAdvancing,
                                discontinuity || syntheticUdpBlock || driftNeedsRealAudio,
                                readAdjustmentApplied ? sourceReadAdjustmentFrames : 0, headAdvanced);
                    }
                    if (udpQueue != null) {
                        observedUdpDropped = udpDroppedAfter;
                        observedUdpExpired = udpExpiredAfter;
                        observedUdpOversizedGaps = udpOversizedGapAfter;
                        observedUdpPayloadGeneration = udpPayloadGenerationAfter;
                    }
                    if (resumeAfterSynthetic) driftNeedsRealAudio = false;
                    session.driftLastOutputDiscontinuities = underruns;
                    writeFrames(session, output, pcm, buffer, outputBytes, frameBytes, blockGeneration);
                    if (trackUnderruns >= 0) {
                        int reportedUnderruns = output.underrunCount();
                        if (udpQueue != null) udpQueue.reportTrackUnderruns(reportedUnderruns);
                        synchronized (STATUS_LOCK) {
                            if (statusAudioId == session.id) {
                                underrunIncrement = underrunCount < 0 || reportedUnderruns < underrunCount
                                        ? -1 : reportedUnderruns - underrunCount;
                                underrunCount = reportedUnderruns;
                            }
                        }
                    }
                }
            }
        } catch (IOException | RuntimeException error) {
            noteOboeRuntimeFailure(session, error);
            failure = session.cancelled ? session.stopReason : describeFailure(error);
        } finally {
            session.trackPlaying = false;
            String trackReleaseFailure;
            synchronized (session) {
                trackReleaseFailure = "";
                if (output != null) {
                    try { output.close(); }
                    catch (IOException | RuntimeException error) {
                        noteOboeRuntimeFailure(session, error);
                        try { output.close(); }
                        catch (IOException | RuntimeException retryError) {
                            error.addSuppressed(retryError);
                            noteOboeRuntimeFailure(session, retryError);
                            retainDeferredOboeClose(output);
                            trackReleaseFailure = "Audio output close failed: " + describeFailure(error);
                        }
                    }
                }
                if (session.audioTrack == track) session.audioTrack = null;
                if (session.playbackOutput == output) session.playbackOutput = null;
                if (session.request.formatV2() != null) {
                    session.openedTrackFormatV2 = 0;
                    session.openedTrackChannelsV2 = 0;
                    session.openedTrackSampleRateV2 = 0;
                }
            }
            String focusReleaseFailure = abandonAudioFocus(manager, focusRequest, focusListener);
            if (!trackReleaseFailure.isEmpty()) failure = appendFailure(failure, trackReleaseFailure);
            if (!focusReleaseFailure.isEmpty()) failure = appendFailure(failure, focusReleaseFailure);
            if (udpReceiver != null) udpReceiver.close();
            if (udpQueue != null) udpQueue.stop();
            if (wifiLock != null) {
                try { if (wifiLock.isHeld()) wifiLock.release(); }
                catch (RuntimeException error) { failure = appendFailure(failure, "WiFiLockRelease: " + describeFailure(error)); }
            }
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

    private boolean writeFrames(AudioSession session, PlaybackOutput output, Pcm16FrameStream pcm,
            byte[] buffer, int count, int frameBytes, long writeGeneration) throws IOException {
        boolean[] completed = {true};
        pcm.writeFramesForChannels((data, offset, length) -> {
            ensureRunning(session);
            if (!session.focusState.acceptsWrite(writeGeneration)) {
                completed[0] = false;
                return length;
            }
            if (session.udpQueue == null) session.inputPcmBytes += length;
            int written = output.writePcm16(data, offset, length);
            if (written == 0 && output instanceof OboePlaybackOutput) {
                if (!session.focusState.acceptsWrite(writeGeneration)) {
                    completed[0] = false;
                    return length;
                }
                return 0;
            }
            synchronized (session) {
                if (!session.focusState.acceptsWrite(writeGeneration)) {
                    completed[0] = false;
                    return length;
                }
                ensureRunning(session);
                if (written <= 0) throw new IOException("Audio output write failed: " + written);
                if (written % frameBytes != 0) throw new IOException("Audio output returned a partial PCM frame");
                session.trackWrittenFrames += written / frameBytes;
                if (session.request.usesPacketIngress()) {
                    PCM_BYTES_WRITTEN.addAndGet(written);
                    session.playbackActivity.noteSuccessfulWrite(System.nanoTime());
                }
                WAVEFORM.accept(data, offset, written, frameBytes, System.nanoTime());
                return written;
            }
        }, buffer, count);
        if (count % frameBytes != 0) throw new IOException("PCM frame alignment failed");
        return completed[0] && session.focusState.acceptsWrite(writeGeneration);
    }

    private void handleAudioFocusChange(AudioSession session, PlaybackOutput output, int change) {
        String failure = "";
        boolean pauseOutput = false;
        boolean resumeOutput = false;
        float volume = -1f;
        Exception outputControlFailure = null;
        synchronized (session) {
            if (currentAudio != session || session.playbackOutput != output
                    || session.cancelled || !isCurrentOperation(session.ownerId, session.networkGeneration)) return;
            AudioFocusState.Change focusChange;
            if (change == AudioManager.AUDIOFOCUS_LOSS) focusChange = AudioFocusState.Change.LOSS;
            else if (change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) focusChange = AudioFocusState.Change.LOSS_TRANSIENT;
            else if (change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK)
                focusChange = AudioFocusState.Change.LOSS_TRANSIENT_CAN_DUCK;
            else if (change == AudioManager.AUDIOFOCUS_GAIN) focusChange = AudioFocusState.Change.GAIN;
            else return;

            AudioFocusState.Action action = session.focusState.onFocusChange(focusChange);
            if (action == AudioFocusState.Action.STOP) failure = "AudioFocusLost";
            else if (action == AudioFocusState.Action.PAUSE) {
                if (session.udpQueue != null) session.udpQueue.setPaused(true);
                if (session.floatQueue != null) session.floatQueue.setPaused(true);
                pauseOutput = session.trackPlaying || !session.trackReady;
            } else if (action == AudioFocusState.Action.DUCK) volume = 0.2f;
            else if (action == AudioFocusState.Action.RESTORE_VOLUME
                    || action == AudioFocusState.Action.RESTORE_AND_HOLD
                    || action == AudioFocusState.Action.RESTORE_AND_RESUME) {
                volume = 1.0f;
                if (action == AudioFocusState.Action.RESTORE_AND_RESUME) resumeOutput = true;
                else if (action == AudioFocusState.Action.RESTORE_AND_HOLD) session.trackPlaying = false;
            } else if (action == AudioFocusState.Action.RESUME) resumeOutput = true;
            else if (action == AudioFocusState.Action.HOLD) session.trackPlaying = false;
            if (pauseOutput && session.playbackOutput == output) {
                try { output.requestPause(); }
                catch (IOException | RuntimeException error) { outputControlFailure = error; }
            }
            if (volume >= 0 && session.playbackOutput == output) {
                try { output.setVolume(volume); }
                catch (IOException | RuntimeException error) {
                    if (outputControlFailure == null) outputControlFailure = error;
                    else outputControlFailure.addSuppressed(error);
                }
            }
            session.notifyAll();
        }
        try {
            if (outputControlFailure instanceof IOException) throw (IOException) outputControlFailure;
            if (outputControlFailure instanceof RuntimeException)
                throw (RuntimeException) outputControlFailure;
            if (pauseOutput) {
                pauseAndFlushOutput(session, output);
            } else if (resumeOutput) {
                synchronized (session) {
                    if (session.playbackOutput == output && session.focusState.canPlay())
                        failure = resumeFocusedOutput(session, output);
                }
            }
        } catch (IOException | RuntimeException error) {
            if (!noteOboeRuntimeFailureIfActive(session, output, error)) return;
            failure = "AudioFocusTransitionFailed: " + describeFailure(error);
        }
        if (!failure.isEmpty() && !"AudioFocusLost".equals(failure)) {
            synchronized (session) { session.focusState.cancel(); }
        }
        if (!failure.isEmpty()) cancelAudio(session, failure);
    }

    /** Called with the session control monitor held. */
    private String resumeFocusedOutput(AudioSession session, PlaybackOutput output) {
        if (!session.focusState.canPlay()) return "";
        UdpPcmQueue queue = session.udpQueue;
        if (queue != null) queue.setPaused(false);
        FloatPcmFrameQueue floatQueue = session.floatQueue;
        if (floatQueue != null) floatQueue.setPaused(false);
        try {
            output.play();
            markOutputEpoch(session);
            resetPlaybackHeadEpoch(session, output.playbackHeadPosition());
            session.trackPlaying = true;
            session.focusState.markTrackPlaying();
            session.driftResetGeneration++;
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) audioActive = true;
            }
            return "";
        } catch (IOException | RuntimeException error) {
            noteOboeRuntimeFailure(session, error);
            return "AudioFocusResumeFailed: " + describeFailure(error);
        }
    }

    /** The generation is invalidated before this control call; it never holds the session monitor while waiting for the worker. */
    private void pauseAndFlushOutput(AudioSession session, PlaybackOutput output) throws IOException {
        output.pauseAndFlush();
        synchronized (session) {
            if (session.playbackOutput != output) return;
            markOutputEpoch(session);
            session.trackWrittenFrames = 0;
            long playbackPosition = output.playbackHeadPosition();
            resetPlaybackHeadEpoch(session, playbackPosition);
            session.driftResetGeneration++;
            PCM_BYTES_WRITTEN.set(0);
            session.trackPlaying = false;
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) audioActive = false;
            }
        }
    }

    private static void awaitFocusResume(AudioSession session) throws IOException {
        synchronized (session) {
            while (!session.focusState.canPlay() && !session.cancelled) {
                try { session.wait(); }
                catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new IOException("Audio focus pause interrupted", interrupted);
                }
            }
            if (session.cancelled) throw new IOException(session.stopReason.isEmpty()
                    ? "AudioSessionStopped" : session.stopReason);
        }
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
        if (session.request.aoaContext != null) {
            if ("AoaSessionStopped".equals(session.stopReason)) return;
            AoaReceiverCoordinator coordinator;
            synchronized (networkCoordinatorLock) {
                coordinator = session.ownerId.equals(aoaOwnerId)
                        && session.networkGeneration == aoaOwnerGeneration ? aoaCoordinator : null;
            }
            if (coordinator != null) {
                if (failure != null && !failure.isEmpty() && !"AudioSessionEnded".equals(failure))
                    coordinator.reportPlaybackFailure(session.request.aoaContext, new IOException(failure));
                return;
            }
            if (aoaRecoveryDeadlineElapsedMs != 0) return;
        }
        if (session.request.udpContext != null && "NetworkSessionStopped".equals(failure)) {
            networkAudioState = networkCoordinator != null ? "prepared" : "stopped";
            if (networkCoordinator != null) {
                networkRecoveryDeadlineElapsedMs = receiveIntentEnabled
                        ? onceRecoveryDeadline(session.ownerId, session.networkGeneration)
                        : newRecoveryDeadline(session.ownerId, session.networkGeneration);
                restoreBackgroundStandbyForeground(session.ownerId, session.networkGeneration);
            }
            else requestTermination(session.ownerId, currentOperationGeneration, false, "");
            return;
        }
        if (session.request.udpContext != null && networkCoordinator != null) {
            recordNetworkFailure(failure.isEmpty() ? "NetworkAudioSessionEnded" : failure);
            uiOperation.reportError(session.ownerId, networkAudioFailure);
            networkRecoveryDeadlineElapsedMs = receiveIntentEnabled
                    ? onceRecoveryDeadline(session.ownerId, session.networkGeneration)
                    : newRecoveryDeadline(session.ownerId, session.networkGeneration);
            restoreBackgroundStandbyForeground(session.ownerId, session.networkGeneration);
            return;
        }
        boolean failed = failure != null && !failure.isEmpty() && !"AudioSessionEnded".equals(failure);
        requestTermination(session.ownerId, currentOperationGeneration, failed, failed ? failure : "");
    }

    private void restoreBackgroundStandbyForeground(String ownerId, long generation) {
        if (!receiveIntentEnabled) return;
        handler.post(() -> {
            if (!receiveIntentEnabled || !isCurrentOperation(ownerId, generation)) return;
            try {
                Notification notification = serviceNotification("正在等待电脑连接，可随时停止。", ownerId, generation, true);
                if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION_ID, notification,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE);
                else startForeground(NOTIFICATION_ID, notification);
                foregroundReady = true;
            } catch (RuntimeException error) {
                requestTermination(ownerId, generation, true, error.getClass().getSimpleName());
            }
        });
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
        String stopFailure = "";
        IOException outputPauseFailure = null;
        LocalSocket socket;
        UdpReceiver receiver;
        UdpPcmQueue queue;
        FloatPcmFrameQueue floatQueue;
        Thread networkThread;
        PlaybackOutput output;
        synchronized (session) {
            if (!session.cancelled) {
                session.cancelled = true;
                session.stopReason = reason;
            }
            session.focusState.cancel();
            session.notifyAll();
            AudioTrack track = session.audioTrack;
            if (track != null) {
                try { track.stop(); }
                catch (RuntimeException error) { stopFailure = "AudioTrackStopFailed: " + describeFailure(error); }
            }
            output = session.playbackOutput;
            if (output instanceof OboePlaybackOutput) {
                // The worker closes and clears this output under the same session monitor.
                // Keep the short mute request in that critical section so stop cannot pause
                // an adapter that the worker has already closed.
                try { output.requestPause(); }
                catch (IOException error) { outputPauseFailure = error; }
            }
            socket = session.socket;
            receiver = session.udpReceiver;
            queue = session.udpQueue;
            floatQueue = session.floatQueue;
            networkThread = session.networkThread;
            if (!stopFailure.isEmpty()) session.stopReason = appendFailure(session.stopReason, stopFailure);
        }
        if (!stopFailure.isEmpty()) {
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) startFailure = session.stopReason;
            }
        }
        synchronized (STATUS_LOCK) { if (statusAudioId == session.id) audioActive = false; }
        if (outputPauseFailure != null) {
            noteOboeRuntimeFailure(session, outputPauseFailure);
            synchronized (STATUS_LOCK) {
                if (statusAudioId == session.id) startFailure = appendFailure(startFailure,
                        "Oboe pause request failed: " + describeFailure(outputPauseFailure));
            }
        }
        Thread thread = session.thread;
        if (thread != null) thread.interrupt();
        if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        if (receiver != null) receiver.close();
        if (queue != null) queue.stop();
        if (floatQueue != null) floatQueue.stop();
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

    private static long outputPendingFrames(AudioSession session, PlaybackOutput output) {
        if (output == null) return -1;
        try {
            long position = output.playbackHeadPosition();
            long played = position < 0 ? 0 : session.playbackActivity.playedFrames((int) position);
            return output.pendingFrames(session.trackWrittenFrames, played);
        } catch (IOException | RuntimeException unavailable) {
            return -1;
        }
    }

    private void publishMode(AudioSession session, String mode) {
        synchronized (STATUS_LOCK) { if (statusAudioId == session.id) actualMode = mode; }
    }

    static PcmWaveformTap.Waveform aodWaveform() {
        return WAVEFORM.waveform(System.nanoTime());
    }

    static PlaybackMetrics playbackMetricsSnapshot() {
        AudioSession session = currentAudio;
        if (session == null) return PlaybackMetrics.unknown();
        UdpPcmQueue queue = session.udpQueue;
        FloatPcmFrameQueue floatQueue = session.floatQueue;
        AudioTrack track = session.audioTrack;
        PlaybackOutput output = session.playbackOutput;
        int jitter = queue == null ? -1 : queue.targetFrames();
        long expired = queue == null ? -1 : queue.expiredFrames();
        long queued = queue != null ? queue.queuedFrames()
                : floatQueue != null ? floatQueue.queuedFrames() : -1;
        long pending = output != null ? outputPendingFrames(session, output) : trackPendingFrames(session, track);
        Boolean advancing = null;
        if (output != null) {
            try {
                long position = output.playbackHeadPosition();
                if (position >= 0) advancing = session.playbackActivity.recentlyAdvanced(System.nanoTime(), (int) position);
            }
            catch (RuntimeException unavailable) { advancing = null; }
            catch (IOException unavailable) { advancing = null; }
        }
        synchronized (STATUS_LOCK) {
            if (currentAudio != session || statusAudioId != session.id || session.playbackOutput != output)
                return PlaybackMetrics.unknown();
            return new PlaybackMetrics(actualSampleRate, actualChannels, jitter,
                    expired, queued, pending, underrunCount, tcpDroppedBacklogFrames,
                    advancing, requestedBufferFrames, acceptedBufferFrames, effectiveBufferFrames,
                    bufferCapacityFrames, startThresholdFrames, preRollFrames, queue != null);
        }
    }

    static byte[] playbackProgressPayload() {
        PlaybackMetrics metrics = playbackMetricsSnapshot();
        return playbackProgressPayload(metrics);
    }

    private static byte[] playbackProgressPayload(PlaybackMetrics metrics) {
        return P5cControlProtocol.playbackProgressPayload(metrics.sampleRate, metrics.channels,
                metrics.jitterTargetFrames, metrics.expiredFrames, metrics.queuedFrames, metrics.pendingFrames,
                metrics.underrunCount, metrics.tcpDroppedFrames, metrics.advancing, metrics.requestedFrames,
                metrics.acceptedFrames, metrics.effectiveFrames, metrics.capacityFrames,
                metrics.startThresholdFrames, metrics.preRollFrames);
    }

    static byte[] playbackProgressPayloadExtended(boolean audioV2) {
        AudioSession session = currentAudio;
        PlaybackMetrics metrics = playbackMetricsSnapshot();
        if (session != currentAudio) session = null;
        byte[] base = playbackProgressPayload(metrics);
        if (audioV2) base = P5cControlProtocol.playbackProgressPayloadV2(base,
                session == null ? 0 : session.openedTrackFormatV2,
                session == null ? 0 : session.openedTrackChannelsV2,
                session == null ? 0 : session.openedTrackSampleRateV2);
        long[] counters = new long[12];
        java.util.Arrays.fill(counters, -1L);
        int outputEpoch = -1;
        int engine = 0;
        int ringPending = -1;
        int streamPending = -1;
        long receiverGeneration = -1;
        if (session != null) {
            receiverGeneration = session.id;
            FloatPcmFrameQueue floatQueue = session.floatQueue;
            UdpPcmQueue pcmQueue = session.udpQueue;
            if (session.request.aoaContext != null) {
                AnpdAudioIngress ingress = session.request.aoaContext.ingress;
                counters[0] = pcmQueue != null ? pcmQueue.acceptedAudioFrames() : ingress.authenticatedAudioFrames();
            } else if (session.request.udpContext != null) {
                UdpReceiver receiver = session.udpReceiver;
                counters[0] = pcmQueue != null ? pcmQueue.acceptedAudioFrames()
                        : receiver == null ? -1 : receiver.authenticatedAudioFrames();
            }
            if (floatQueue != null) {
                counters[1] = floatQueue.droppedFrames();
                counters[2] = floatQueue.expiredFrames();
                counters[3] = floatQueue.concealedFrames();
                counters[4] = floatQueue.insertedDriftFrames();
                counters[5] = floatQueue.deletedDriftFrames();
            } else if (pcmQueue != null) {
                counters[1] = pcmQueue.droppedFrames();
                counters[2] = pcmQueue.expiredFrames();
                counters[3] = pcmQueue.concealedFrames();
            }
            FloatPcmDriftController drift = session.floatDriftController;
            if (drift != null) counters[6] = drift.resetCount();
            PlaybackOutput output = session.playbackOutput;
            if (output != null) {
                outputEpoch = session.outputEpoch;
                engine = "oboeExperimental".equals(session.actualOutputEngine) ? 2
                        : "audioTrack".equals(session.actualOutputEngine) ? 1 : 0;
                counters[7] = session.trackWrittenFrames;
                try {
                    long position = output.playbackHeadPosition();
                    if (position >= 0) counters[8] = session.playbackActivity.playedFrames((int) position);
                    long pending = position < 0 ? -1
                            : output.pendingFrames(session.trackWrittenFrames, counters[8]);
                    if (output instanceof OboePlaybackOutput) {
                        long[] nativeMetrics = ((OboePlaybackOutput) output).diagnosticMetrics();
                        ringPending = (int) nativeMetrics[0];
                        streamPending = (int) nativeMetrics[1];
                        counters[9] = nativeMetrics[3];
                        counters[10] = nativeMetrics[4];
                        counters[11] = nativeMetrics[5];
                        outputEpoch = (int) nativeMetrics[6];
                    } else if (pending >= 0) streamPending = (int) pending;
                } catch (IOException | RuntimeException unavailable) {
                    ringPending = -1;
                    streamPending = -1;
                }
            }
        }
        return P5cControlProtocol.playbackProgressPayloadExtended(base,
                android.os.SystemClock.elapsedRealtimeNanos(), receiverGeneration, outputEpoch, engine,
                counters, ringPending, streamPending);
    }

    static byte[] playbackProgressPayloadExtendedV2() {
        return playbackProgressPayloadExtended(true);
    }

    private static void markOutputEpoch(AudioSession session) { session.outputEpoch++; }

    private static void resetPlaybackHeadEpoch(AudioSession session, long playbackPosition) {
        if (playbackPosition >= 0) {
            int head = (int) playbackPosition;
            session.playbackActivity.resetPlaybackHead(head);
            session.driftPlaybackHead.reset(head, System.nanoTime());
        } else {
            session.playbackActivity.resetPlaybackHeadUnavailable();
            session.driftPlaybackHead.reset();
        }
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

    static String describeFailure(Throwable error) {
        String message = error.getMessage();
        if (message == null || message.isEmpty()) return error.getClass().getSimpleName();
        String description = (error.getClass().getSimpleName() + ": " + message).replace('\r', ' ').replace('\n', ' ');
        return description.substring(0, Math.min(description.length(), 180));
    }

    static String releaseTrack(AudioTrack track) {
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

    static String appendFailure(String previous, String next) {
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

    static final class AudioRequest {
        final String socketName;
        final String connectCode;
        final String playbackMode;
        final UdpSessionContext udpContext;
        final AoaSessionContext aoaContext;

        AudioRequest(String socketName, String connectCode, String playbackMode) {
            this(socketName, connectCode, playbackMode, null, null);
        }

        private AudioRequest(String socketName, String connectCode, String playbackMode,
                UdpSessionContext udpContext, AoaSessionContext aoaContext) {
            this.socketName = socketName;
            this.connectCode = connectCode;
            this.playbackMode = playbackMode;
            this.udpContext = udpContext;
            this.aoaContext = aoaContext;
        }

        static AudioRequest udp(UdpSessionContext context) {
            return new AudioRequest(null, null, context.playbackMode, context, null);
        }

        static AudioRequest aoa(AoaSessionContext context) {
            return new AudioRequest(null, null, context.playbackMode, null, context);
        }

        UdpPacketDecoder.AudioFormatV2 formatV2() {
            return udpContext != null ? udpContext.formatV2 : aoaContext == null ? null : aoaContext.formatV2;
        }

        boolean usesPacketIngress() { return udpContext != null || aoaContext != null; }

        boolean usesFloatV2Worker() {
            UdpPacketDecoder.AudioFormatV2 format = formatV2();
            return format != null && format.profile > 1;
        }

        // AOA context lifetime belongs to its coordinator, which joins transport IO before closing it.
        void destroy() { if (udpContext != null) udpContext.destroy(); }
    }

    private static final class AudioStartTicket {
        final CountDownLatch finished = new CountDownLatch(1);
        volatile boolean cancelled;
        volatile AudioSession accepted;

        AudioStartTicket() {}
    }

    static final class AudioSession {
        final long id;
        final AudioRequest request;
        final String ownerId;
        final long networkGeneration;
        volatile Thread thread;
        volatile Thread networkThread;
        volatile LocalSocket socket;
        volatile UdpReceiver udpReceiver;
        volatile UdpPcmQueue udpQueue;
        volatile FloatPcmFrameQueue floatQueue;
        final UdpPlaybackActivity playbackActivity = new UdpPlaybackActivity();
        final Pcm16FrameStream.PlaybackHeadTracker driftPlaybackHead =
                new Pcm16FrameStream.PlaybackHeadTracker();
        volatile long driftResetGeneration;
        volatile FloatPcmDriftController floatDriftController;
        volatile int outputEpoch;
        volatile int driftLastOutputDiscontinuities = -1;
        volatile int oboeXrunCount = -1;
        volatile AudioTrack audioTrack;
        volatile PlaybackOutput playbackOutput;
        volatile String requestedOutputEngine = "oboeExperimental";
        volatile String actualOutputEngine = "unknown";
        volatile String outputFallbackReason = "";
        volatile int openedTrackFormatV2;
        volatile int openedTrackChannelsV2;
        volatile int openedTrackSampleRateV2;
        boolean trackReady;
        volatile long trackWrittenFrames;
        volatile long inputPcmBytes;
        volatile boolean trackPlaying;
        final AudioFocusState focusState = new AudioFocusState();
        volatile boolean cancelled;
        volatile String stopReason = "";

        AudioSession(long id, AudioRequest request, String ownerId, long networkGeneration) {
            this.id = id;
            this.request = request;
            this.ownerId = ownerId;
            this.networkGeneration = networkGeneration;
            if (request.aoaContext != null) {
                udpQueue = request.aoaContext.pcmQueue;
                floatQueue = request.aoaContext.floatQueue;
            }
        }

        boolean isStreamingAt(long nowNanos) throws IOException {
            if (cancelled || !focusState.canPlay() || !trackPlaying) return false;
            UdpPcmQueue queue = udpQueue;
            FloatPcmFrameQueue floatQueue = this.floatQueue;
            UdpReceiver receiver = udpReceiver;
            PlaybackOutput output = playbackOutput;
            if ((queue == null && floatQueue == null) || output == null) return false;
            long playbackHead;
            try {
                playbackHead = output.playbackHeadPosition();
            } catch (IOException | RuntimeException error) {
                if (!noteOboeRuntimeFailureIfActive(this, output, error)) return false;
                throw new IOException("Audio output progress unavailable", error);
            }
            if (playbackHead < 0) return false;
            boolean hasAudio = queue != null ? queue.hasAuthenticatedAudio()
                    : request.aoaContext != null ? request.aoaContext.ingress.hasAuthenticatedAudio()
                    : receiver != null && receiver.hasAuthenticatedAudio();
            long lastAudioAt = queue != null ? queue.lastAuthenticatedAudioAtNanos()
                    : request.aoaContext != null ? request.aoaContext.ingress.lastAuthenticatedAudioAtNanos()
                    : receiver == null ? 0 : receiver.lastAuthenticatedAudioAtNanos();
            return playbackActivity.isActive(nowNanos, (int) playbackHead, hasAudio,
                    lastAudioAt, focusState.isUserPaused());
        }
    }
}
