package app.anpod.receiver;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.BindException;
import java.net.DatagramSocket;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ScheduledThreadPoolExecutor;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;

/** Explicitly started, one-session localhost or selected-interface P5-C endpoint. */
final class P5cReceiverCoordinator implements AutoCloseable {
    static final int STATE_AUTHENTICATED = 0;
    static final int STATE_TRANSPORT_CONNECTED = 1;
    static final int STATE_RECEIVER_RUNNING = 2;
    static final int STATE_STREAMING = 3;
    static final int STATE_PAUSED = 4;
    static final int STATE_STOPPED = 5;
    static final int RESULT_OK = 0;
    static final int RESULT_REJECTED = 1;
    static final int RESULT_FAILED = 2;
    static final int RESULT_USER_STOPPED = 3;
    private static final long HEARTBEAT_TIMEOUT_NANOS = TimeUnit.SECONDS.toNanos(5);
    private static final long HANDSHAKE_DEADLINE_NANOS = TimeUnit.SECONDS.toNanos(2);
    private static final int MAX_PREAUTH_CONNECTIONS = 2;
    private static final int MAX_PREAUTH_PER_SOURCE = 1;

    // API 23 has no java.util.function.BooleanSupplier.
    interface StartPermission { boolean allowed(); }

    static final class BackgroundTrustSnapshot {
        final int kind;
        final int persistence;
        final long remainingMs;
        BackgroundTrustSnapshot(int kind, int persistence, long remainingMs) {
            this.kind = kind; this.persistence = persistence; this.remainingMs = remainingMs;
        }
    }

    interface PlaybackEndpoint {
        /** Takes ownership of the already authenticated and UDP-peer-bound context on success. */
        boolean start(UdpSessionContext context, StartPermission permission) throws IOException;
        void pause() throws IOException;
        void resume() throws IOException;
        void stop();
        boolean streaming() throws IOException;
        void shutdown();
        default boolean keepStandbyAfterPlaybackStop() { return false; }
        /** True only when the playback endpoint has a working v2 audio-format consumer. */
        default boolean supportsAudioV2() { return false; }
        /** Endpoint-owned product policy for a V2 descriptor. */
        default boolean supportsAudioFormatV2(UdpPacketDecoder.AudioFormatV2 format) { return false; }
        /** Actual opened AudioTrack format: 0 unknown, 1 PCM16, 2 packed24, 3 Float32. */
        default int openedTrackFormatV2() { return 0; }
        default int openedTrackChannelsV2() { return 0; }
        default int openedTrackSampleRateV2() { return 0; }
        default boolean supportsPlaybackProgress() { return false; }
        default boolean supportsPlaybackProgressExtension() { return false; }
        default byte[] playbackProgressPayload() {
            return P5cControlProtocol.playbackProgressPayload(-1, -1, -1,
                    -1, -1, -1, -1, -1, null, -1, -1, -1, -1, -1, -1);
        }
        default byte[] playbackProgressPayloadV2() {
            return P5cControlProtocol.playbackProgressPayloadV2(playbackProgressPayload(),
                    openedTrackFormatV2(), openedTrackChannelsV2(), openedTrackSampleRateV2());
        }
        default byte[] playbackProgressPayloadExtended(boolean audioV2) { return null; }
        default byte[] playbackProgressPayloadExtended(boolean audioV2, int schema) {
            return schema == 2 ? playbackProgressPayloadExtended(audioV2) : null;
        }
        /** True only for an explicitly started standby endpoint; production defaults to the existing manual flow. */
        default boolean requiresBackgroundStartAuthorization() { return false; }
        default int requestBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) throws IOException { return P5cControlProtocol.AUTH_EXPIRED; }
        default int backgroundStartAuthorizationStatus(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) throws IOException { return P5cControlProtocol.AUTH_EXPIRED; }
        /** keepForRecovery is true only after playback started and the authenticated session was lost unexpectedly. */
        default void endBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment, boolean keepForRecovery) {}
        default void endBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment, boolean playbackHadStarted, boolean keepForRecovery) {
            endBackgroundStartAuthorization(requestId, sessionId, peer, bindingCommitment, keepForRecovery);
        }
        default void cancelBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) {
            endBackgroundStartAuthorization(requestId, sessionId, peer, bindingCommitment, false);
        }
        default void backgroundPlaybackStarted(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) {}
        default boolean supportsBackgroundTrustManagement() { return false; }
        default BackgroundTrustSnapshot queryBackgroundTrust(byte[] bindingCommitment) throws IOException {
            throw new IOException("background trust management is unsupported");
        }
        default boolean revokeBackgroundTrust(byte[] bindingCommitment) throws IOException { return false; }
    }

    private static final class SessionMaterial {
        final int protocolVersion;
        final long sessionId;
        final PairingProtocol.AudioParameters parameters;
        final UdpPacketDecoder.AudioFormatV2 formatV2;
        final PairingProtocol.SessionKeys keys;
        final byte[] ready;

        SessionMaterial(int protocolVersion, long sessionId, PairingProtocol.AudioParameters parameters,
                UdpPacketDecoder.AudioFormatV2 formatV2, PairingProtocol.SessionKeys keys, byte[] ready) {
            this.protocolVersion = protocolVersion;
            this.sessionId = sessionId;
            this.parameters = parameters;
            this.formatV2 = formatV2;
            this.keys = keys;
            this.ready = ready;
        }
    }

    private final ServerSocket listener;
    private final PairingProtocol.BindingMaterial binding;
    private final PlaybackEndpoint playback;
    private final boolean loopbackOnly;
    private final boolean dropStopAckForTest;
    private final int discoveryPrefixLength;
    private final int receiverVersionCode;
    private volatile String capabilityTestMode = "";
    private volatile String backgroundAuthorizationTestMode = "";
    volatile String backgroundTrustTestMode = "";
    private final CountDownLatch finished = new CountDownLatch(1);
    private final Map<Socket, Long> preAuthSockets = new LinkedHashMap<>();
    private final ThreadPoolExecutor preAuthWorkers = new ThreadPoolExecutor(MAX_PREAUTH_CONNECTIONS,
            MAX_PREAUTH_CONNECTIONS, 0, TimeUnit.MILLISECONDS, new ArrayBlockingQueue<>(MAX_PREAUTH_CONNECTIONS),
            runnable -> { Thread thread = new Thread(runnable, "AnPod P5-A preauth"); thread.setDaemon(true); return thread; });
    private final ScheduledThreadPoolExecutor handshakeDeadline = new ScheduledThreadPoolExecutor(1, runnable -> {
        Thread thread = new Thread(runnable, "AnPod P5-A deadline");
        thread.setDaemon(true);
        return thread;
    });
    private volatile Socket activeSocket;
    private volatile Socket authenticatedSocket;
    private volatile boolean terminalCauseNegotiated;
    private volatile Thread worker;
    private volatile P5cDiscovery discovery;
    private P5cDiscovery.SocketBinder discoverySocketBinder;
    private byte[] bindingSecret;
    private volatile String failure = "";
    private volatile boolean shutdownRequested;
    private volatile boolean userStopRequested;
    private boolean loopbackConnectionAccepted;
    private boolean discoveryResumePending;
    private boolean discoveryDisabledForTest;
    private byte[] connectionIntentNonce;
    private byte[] establishedConnectionIntentNonce;
    private long connectionIntentDeadline;
    private volatile boolean connectionIntentRequired;
    private boolean connectionIntentEstablished;

    static P5cReceiverCoordinator openLoopback(PairingProtocol.BindingMaterial existingBinding,
            PlaybackEndpoint playback) throws IOException {
        return open(existingBinding, playback, InetAddress.getByName("127.0.0.1"), true, 32, false, 0);
    }

    static P5cReceiverCoordinator openLoopbackDropStopAckForTest(PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        return open(binding, playback, InetAddress.getByName("127.0.0.1"), false, 32, true, 0);
    }

    /** Explicit internal LAN entry; it never starts discovery or playback by itself. */
    static P5cReceiverCoordinator openOnInterface(PairingProtocol.BindingMaterial existingBinding,
            PlaybackEndpoint playback, InetAddress localAddress, int prefixLength) throws IOException {
        return openOnInterface(existingBinding, playback, localAddress, prefixLength, 0);
    }

    static P5cReceiverCoordinator openOnInterface(PairingProtocol.BindingMaterial existingBinding,
            PlaybackEndpoint playback, InetAddress localAddress, int prefixLength, int versionCode) throws IOException {
        if (!validInterfaceAddress(localAddress, prefixLength))
            throw new IOException("P5-C requires an explicit private IPv4 interface address and prefix");
        if (!isAssignedInterfaceAddress(localAddress, prefixLength))
            throw new IOException("P5-C interface address is not assigned to an active Android interface");
        return open(existingBinding, playback, localAddress, false, prefixLength, false, versionCode);
    }

    static P5cReceiverCoordinator openLoopbackCapabilityTest(PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback, String mode) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        P5cReceiverCoordinator coordinator = open(binding, playback, InetAddress.getByName("127.0.0.1"), true, 32, false, 0);
        coordinator.capabilityTestMode = mode;
        return coordinator;
    }

    static P5cReceiverCoordinator openLoopbackAudioV2Test(PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        // Bound to loopback but permits sequential authenticated probe/formal sessions.
        P5cReceiverCoordinator coordinator = open(binding, playback,
                InetAddress.getByName("127.0.0.1"), false, 32, false, 0);
        coordinator.discoveryDisabledForTest = true;
        return coordinator;
    }

    static P5cReceiverCoordinator openLoopbackBackgroundAuthorizationTest(PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback, String mode) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        P5cReceiverCoordinator coordinator = open(binding, playback, InetAddress.getByName("127.0.0.1"),
                !"persistent-stop".equals(mode), 32, false, 0);
        coordinator.backgroundAuthorizationTestMode = mode;
        return coordinator;
    }

    static P5cReceiverCoordinator openLoopbackIntentTest(PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback, byte[] nonce) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        P5cReceiverCoordinator coordinator = openLoopback(binding, playback);
        coordinator.beginConnectionIntent(nonce);
        return coordinator;
    }

    private static boolean isAssignedInterfaceAddress(InetAddress address, int prefixLength) throws SocketException {
        NetworkInterface network = NetworkInterface.getByInetAddress(address);
        if (network == null || !network.isUp()) return false;
        if (address.isLoopbackAddress()) return network.isLoopback() && prefixLength == 32;
        if (network.isLoopback()) return false;
        for (java.net.InterfaceAddress item : network.getInterfaceAddresses())
            if (address.equals(item.getAddress()) && item.getNetworkPrefixLength() == prefixLength) return true;
        return false;
    }

    static boolean validInterfaceAddress(InetAddress localAddress, int prefixLength) {
        return localAddress instanceof Inet4Address
                && (localAddress.isLoopbackAddress() || localAddress.isSiteLocalAddress())
                && prefixLength >= 1 && prefixLength <= 32
                && (!localAddress.isLoopbackAddress() || prefixLength == 32);
    }

    private static P5cReceiverCoordinator open(PairingProtocol.BindingMaterial existingBinding,
            PlaybackEndpoint playback, InetAddress localAddress, boolean loopbackOnly, int prefixLength,
            boolean dropStopAckForTest, int versionCode) throws IOException {
        ServerSocket listener = new ServerSocket();
        try {
            listener.setReuseAddress(false);
            listener.bind(new InetSocketAddress(localAddress, 0), 4);
            return new P5cReceiverCoordinator(listener, existingBinding, playback, loopbackOnly, prefixLength,
                    dropStopAckForTest, versionCode);
        } catch (IOException | RuntimeException error) {
            listener.close();
            throw error;
        }
    }

    private P5cReceiverCoordinator(ServerSocket listener, PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback, boolean loopbackOnly, int discoveryPrefixLength, boolean dropStopAckForTest,
            int receiverVersionCode) {
        if (binding == null || playback == null) throw new IllegalArgumentException("P5-C inputs are required");
        this.listener = listener;
        this.binding = binding;
        this.playback = playback;
        this.loopbackOnly = loopbackOnly;
        this.dropStopAckForTest = dropStopAckForTest;
        this.discoveryPrefixLength = discoveryPrefixLength;
        this.receiverVersionCode = receiverVersionCode;
    }

    int port() { return listener.getLocalPort(); }
    InetAddress localAddress() { return listener.getInetAddress(); }

    synchronized void start() {
        if (worker != null) throw new IllegalStateException("P5-C listener already started");
        worker = new Thread(this::serve, "AnPod P5-C localhost session");
        worker.setDaemon(true);
        worker.start();
    }

    synchronized void startDiscovery() throws IOException {
        startDiscovery(null);
    }

    synchronized void startDiscovery(P5cDiscovery.SocketBinder socketBinder) throws IOException {
        startDiscovery(socketBinder, bindingSecret);
    }

    synchronized void startDiscovery(P5cDiscovery.SocketBinder socketBinder, byte[] discoverySecret) throws IOException {
        if (discoveryDisabledForTest) return;
        if (worker == null) throw new IllegalStateException("P5-C control listener must be started first");
        if (socketBinder != null) discoverySocketBinder = socketBinder;
        if (discoverySecret != null) bindingSecret = discoverySecret.clone();
        if (authenticatedSocket != null || !preAuthSockets.isEmpty() || discovery != null) return;
        P5cDiscovery service = P5cDiscovery.open(listener.getInetAddress(), discoveryPrefixLength,
                listener.getLocalPort(), discoverySocketBinder, bindingSecret);
        service.start();
        discovery = service;
    }

    P5cDiscovery.SearchSnapshot startSearch() throws IOException {
        P5cDiscovery current = discovery;
        if (current == null) throw new IOException("Receiver 网络接收尚未运行，无法搜索已授权的 PC。");
        return current.startSearch();
    }

    synchronized P5cDiscovery.SearchSnapshot startAutomaticSearch() throws IOException {
        if (authenticatedSocket != null) throw new IOException("Receiver 已有连接操作；不能创建第二个接收意图。");
        P5cDiscovery current = discovery;
        if (current == null) throw new IOException("Receiver 网络接收尚未准备。");
        // Arm rejection before broadcasting, including a failed or expired search startup.
        beginConnectionIntent(null);
        P5cDiscovery.SearchSnapshot result = current.startAutomaticSearch();
        connectionIntentNonce = current.pendingConnectionNonce();
        return result;
    }

    synchronized P5cDiscovery.SearchSnapshot maintainAutomaticSearch() throws IOException {
        P5cDiscovery current = discovery;
        if (current == null) throw new IOException("Receiver 网络接收尚未准备。");
        P5cDiscovery.SearchSnapshot snapshot = current.searchSnapshot();
        if (authenticatedSocket != null || snapshot.active
                || !("idle".equals(snapshot.lastRequestStatus) || "expired".equals(snapshot.lastRequestStatus)))
            return snapshot;
        return startAutomaticSearch();
    }

    private synchronized void beginConnectionIntent(byte[] nonce) {
        boolean retainEstablished = nonce == null && connectionIntentEstablished
                && establishedConnectionIntentNonce != null;
        connectionIntentRequired = true;
        connectionIntentNonce = nonce == null ? null : nonce.clone();
        connectionIntentDeadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10);
        connectionIntentEstablished = retainEstablished;
        if (!retainEstablished) establishedConnectionIntentNonce = null;
    }

    synchronized void cancelPendingConnectionIntent() {
        connectionIntentNonce = null;
        establishedConnectionIntentNonce = null;
        connectionIntentEstablished = false;
        if (discovery != null) discovery.stopSearch();
        for (Socket pending : new HashSet<>(preAuthSockets.keySet()))
            try { pending.close(); } catch (IOException ignored) {}
    }

    private synchronized boolean acceptsConnectionIntent(byte[] nonce) {
        if (connectionIntentRequired && !interfaceIsAssigned()) {
            connectionIntentNonce = null;
            establishedConnectionIntentNonce = null;
            connectionIntentEstablished = false;
            return false;
        }
        return !connectionIntentRequired || (nonce != null && (
                connectionIntentEstablished && establishedConnectionIntentNonce != null
                        && Arrays.equals(nonce, establishedConnectionIntentNonce)
                || connectionIntentNonce != null && Arrays.equals(nonce, connectionIntentNonce)
                        && System.nanoTime() < connectionIntentDeadline));
    }

    private static boolean backgroundAuthorizationGranted(int status) {
        return status == P5cControlProtocol.AUTH_ONCE || status == P5cControlProtocol.AUTH_EIGHT_HOURS
                || status == P5cControlProtocol.AUTH_UNTIL_REVOKED;
    }

    private static boolean validBackgroundAuthorizationStatus(int status) {
        return status >= P5cControlProtocol.AUTH_PENDING && status <= P5cControlProtocol.AUTH_CANCELLED;
    }

    private int queryBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
            byte[] bindingCommitment) throws IOException {
        int status = playback.backgroundStartAuthorizationStatus(requestId.clone(), sessionId, peer,
                bindingCommitment.clone());
        if (!validBackgroundAuthorizationStatus(status)) throw new IOException("invalid background authorization status");
        return status;
    }

    private boolean backgroundStartStillAuthorized(byte[] requestId, long sessionId, InetAddress peer,
            byte[] bindingCommitment) {
        if (requestId == null || bindingCommitment == null) return false;
        try { return backgroundAuthorizationGranted(queryBackgroundStartAuthorization(requestId,
                sessionId, peer, bindingCommitment)); }
        catch (IOException | RuntimeException error) { return false; }
    }

    private boolean acceptsConnectionIntentOrBackgroundGrant(byte[] nonce, boolean backgroundAuthorizationRequired,
            boolean backgroundGranted) {
        if (acceptsConnectionIntent(nonce)) return true;
        return backgroundGranted && connectionIntentRequired && backgroundAuthorizationRequired && interfaceIsAssigned();
    }

    private static boolean allZero(byte[] bytes) {
        int value = 0;
        for (byte item : bytes) value |= item;
        return value == 0;
    }

    private static boolean supportsBackgroundTrustManagementState(int state) {
        return state == STATE_AUTHENTICATED || state == STATE_TRANSPORT_CONNECTED
                || state == STATE_RECEIVER_RUNNING || state == STATE_STREAMING || state == STATE_PAUSED;
    }

    private boolean interfaceIsAssigned() {
        try { return isAssignedInterfaceAddress(listener.getInetAddress(), discoveryPrefixLength); }
        catch (SocketException error) { return false; }
    }

    P5cDiscovery.SearchSnapshot stopSearch() {
        cancelPendingConnectionIntent();
        P5cDiscovery current = discovery;
        return current == null ? new P5cDiscovery.SearchSnapshot(false, java.util.Collections.emptyList(), "stopped")
                : current.stopSearch();
    }

    P5cDiscovery.SearchSnapshot searchSnapshot() {
        P5cDiscovery current = discovery;
        P5cDiscovery.SearchSnapshot result = current == null
                ? new P5cDiscovery.SearchSnapshot(false, java.util.Collections.emptyList(), "idle") : current.searchSnapshot();
        if ("interface-lost".equals(result.lastRequestStatus)) cancelPendingConnectionIntent();
        return result;
    }

    String requestConnection(String candidateId) throws IOException {
        P5cDiscovery current = discovery;
        if (current == null) throw new IOException("Receiver 网络接收已停止。");
        return current.requestConnection(candidateId);
    }

    boolean await(long timeout, TimeUnit unit) throws InterruptedException {
        return finished.await(timeout, unit);
    }

    String failure() { return failure; }

    boolean requestUserStop() {
        // Defer service teardown only while an authenticated host can receive the
        // terminal ACK. A listening or not-yet-negotiated owner must clean up locally.
        if (shutdownRequested || finished.getCount() == 0 || authenticatedSocket == null
                || !terminalCauseNegotiated) return false;
        userStopRequested = true;
        return true;
    }

    private void serve() {
        try {
            while (!listener.isClosed()) {
                Socket socket = listener.accept();
                if (!admitPreAuth(socket)) {
                    try { socket.close(); } catch (IOException ignored) { }
                    continue;
                }
                try {
                    preAuthWorkers.execute(() -> handleConnection(socket));
                } catch (RejectedExecutionException saturated) {
                    synchronized (this) {
                        preAuthSockets.remove(socket);
                        if (activeSocket == socket) {
                            java.util.Iterator<Socket> remaining = preAuthSockets.keySet().iterator();
                            activeSocket = remaining.hasNext() ? remaining.next() : null;
                        }
                    }
                    try { socket.close(); } catch (IOException ignored) { }
                }
            }
        } catch (SocketException error) {
            if (!listener.isClosed()) failure = listenerFailure(error);
        } catch (IOException | RuntimeException error) {
            if (!listener.isClosed()) failure = listenerFailure(error);
        } finally {
            stopDiscovery();
            Socket[] pending;
            synchronized (this) { pending = preAuthSockets.keySet().toArray(new Socket[0]); }
            for (Socket socket : pending)
                try { socket.close(); } catch (IOException ignored) { }
            Socket authenticated = authenticatedSocket;
            if (authenticated != null) try { authenticated.close(); } catch (IOException ignored) { }
            activeSocket = null;
            if (shutdownRequested) playback.shutdown();
            binding.close();
            try { listener.close(); } catch (IOException ignored) {}
            preAuthWorkers.shutdownNow();
            handshakeDeadline.shutdownNow();
            try { preAuthWorkers.awaitTermination(2, TimeUnit.SECONDS); }
            catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
            finished.countDown();
        }
    }

    private synchronized boolean admitPreAuth(Socket socket) {
        if (listener.isClosed() || authenticatedSocket != null || (loopbackOnly && loopbackConnectionAccepted)) return false;
        Socket evict = null;
        int fromSource = 0;
        for (Socket pending : preAuthSockets.keySet()) {
            if (pending.getInetAddress().equals(socket.getInetAddress())) {
                evict = pending;
                fromSource++;
            }
        }
        if (fromSource < MAX_PREAUTH_PER_SOURCE && preAuthSockets.size() >= MAX_PREAUTH_CONNECTIONS)
            evict = preAuthSockets.keySet().iterator().next();
        if (evict != null) {
            preAuthSockets.remove(evict);
            try { evict.close(); } catch (IOException ignored) { }
        }
        preAuthSockets.put(socket, System.nanoTime() + HANDSHAKE_DEADLINE_NANOS);
        if (loopbackOnly) loopbackConnectionAccepted = true;
        activeSocket = socket;
        return true;
    }

    private void handleConnection(Socket socket) {
        boolean ownedSession = false;
        try {
            socket.setSoTimeout(250);
            if (loopbackOnly && !socket.getInetAddress().isLoopbackAddress()) throw new IOException("loopback peer required");
            serveSession(socket, preAuthDeadline(socket));
        } catch (IOException | PairingProtocol.Failure | RuntimeException error) {
            if (!listener.isClosed()) failure = "session_failed";
        } finally {
            synchronized (this) {
                ownedSession = authenticatedSocket == socket;
                if (ownedSession) {
                    authenticatedSocket = null;
                    terminalCauseNegotiated = false;
                    discoveryResumePending = true;
                    // If the host disappeared before the requested authenticated
                    // terminal ACK, finish the manual stop locally after liveness
                    // timeout instead of accepting a reconnect.
                    if (userStopRequested) shutdownRequested = true;
                }
                preAuthSockets.remove(socket);
                if (activeSocket == socket) {
                    java.util.Iterator<Socket> remaining = preAuthSockets.keySet().iterator();
                    activeSocket = remaining.hasNext() ? remaining.next() : null;
                }
            }
            try { socket.close(); } catch (IOException ignored) { }
            if (!listener.isClosed() && !shutdownRequested) {
                try { resumeDiscoveryIfIdle(); }
                catch (IOException | RuntimeException error) {
                    failure = listenerFailure(error);
                    try { listener.close(); } catch (IOException ignored) { }
                }
            }
            if (loopbackOnly || shutdownRequested) try { listener.close(); } catch (IOException ignored) { }
        }
    }

    private synchronized long preAuthDeadline(Socket socket) throws IOException {
        Long deadline = preAuthSockets.get(socket);
        if (deadline == null || socket.isClosed()) throw new IOException("P5-A connection was evicted before authentication");
        return deadline;
    }

    private synchronized boolean claimAuthenticated(Socket socket) {
        if (listener.isClosed() || socket.isClosed() || !preAuthSockets.containsKey(socket) || authenticatedSocket != null)
            return false;
        preAuthSockets.remove(socket);
        authenticatedSocket = socket;
        activeSocket = socket;
        terminalCauseNegotiated = false;
        discoveryResumePending = false;
        stopDiscovery();
        return true;
    }

    private synchronized void resumeDiscoveryIfIdle() throws IOException {
        if (discoveryDisabledForTest) {
            discoveryResumePending = false;
            return;
        }
        if (!discoveryResumePending || listener.isClosed() || shutdownRequested
                || authenticatedSocket != null || !preAuthSockets.isEmpty()) return;
        startDiscovery();
        discoveryResumePending = false;
        if (connectionIntentRequired && !connectionIntentEstablished && connectionIntentNonce != null
                && System.nanoTime() < connectionIntentDeadline)
            discovery.resumeAutomaticSearch(connectionIntentNonce,
                    TimeUnit.NANOSECONDS.toMillis(connectionIntentDeadline - System.nanoTime()));
    }

    private static String listenerFailure(Exception error) {
        if (error instanceof IllegalStateException
                && "P5-C discovery already started".equals(error.getMessage()))
            return "listener_failed:discovery_already_started";
        if (error instanceof BindException) return "listener_failed:bind";
        if (error instanceof SocketException) return "listener_failed:socket";
        if (error instanceof IOException) return "listener_failed:io";
        return "listener_failed:runtime";
    }

    private void serveSession(Socket socket, long deadline) throws IOException, PairingProtocol.Failure {
        DataInputStream input = new DataInputStream(socket.getInputStream());
        DataOutputStream output = new DataOutputStream(socket.getOutputStream());
        long remaining = deadline - System.nanoTime();
        if (remaining <= 0) throw new java.net.SocketTimeoutException("P5-A total handshake deadline expired");
        ScheduledFuture<?> deadlineTask = handshakeDeadline.schedule(() -> {
            try { socket.close(); } catch (IOException ignored) { }
        }, remaining, TimeUnit.NANOSECONDS);
        try {
            serveHandshakeAndSession(socket, input, output, deadline, deadlineTask);
        } finally {
            deadlineTask.cancel(false);
        }
    }

    private void serveHandshakeAndSession(Socket socket, DataInputStream input, DataOutputStream output,
            long deadline, ScheduledFuture<?> deadlineTask) throws IOException, PairingProtocol.Failure {
        SessionMaterial authenticated = acceptSessionHandshake(socket, input, output, deadline);
        socket.setSoTimeout(250);
        try (PairingProtocol.SessionKeys keys = authenticated.keys) {
                    long sessionId = authenticated.sessionId;
                    PairingProtocol.AudioParameters parameters = authenticated.parameters;
                    UdpPacketDecoder.AudioFormatV2 formatV2 = authenticated.formatV2;
                    int audioMode = formatV2 == null ? parameters.mode : formatV2.mode;
                    boolean upgradeProbeOnly = false;
                    if (formatV2 != null && !playback.supportsAudioV2())
                        throw new IOException("Receiver playback endpoint does not support audio v2");
                    DatagramSocket audioSocket = null;
                    UdpSessionContext context = null;
                    boolean playbackStarted = false;
                    int state = STATE_AUTHENTICATED;
                    int negotiatedCapabilities = 0;
                    long expectedCommandSequence = 0;
                    long ackSequence = 0;
                    byte[] lastCommand = null;
                    byte[] lastAck = null;
                    boolean modeSelected = false;
                    byte[] claimedConnectionIntent = null;
                    byte[] backgroundRequestId = null;
                    int backgroundAuthorizationStatus = P5cControlProtocol.AUTH_EXPIRED;
                    boolean backgroundAuthorizationStarted = false;
                    boolean backgroundAuthorizationEnded = false;
                    boolean backgroundTrustRevoked = false;
                    byte[] bindingCommitment = null;
                    final boolean backgroundAuthorizationRequired = playback.requiresBackgroundStartAuthorization();
                    long lastHeartbeatAt = System.nanoTime();
                    try {
                        bindingCommitment = binding.commitment();
                        // Keep the authenticated session/keys owned even if READY cannot be written.
                        output.write(authenticated.ready);
                        output.flush();
                        deadlineTask.cancel(false);
            while (true) {
                checkHeartbeatDeadline(System.nanoTime(), lastHeartbeatAt);
                P5cControlProtocol.Frame command;
                try {
                    command = P5cControlProtocol.read(input, keys.hostToReceiverControl,
                            P5cControlProtocol.HOST_TO_RECEIVER, sessionId);
                } catch (P5cControlProtocol.IdleTimeoutException idle) {
                    continue;
                } catch (java.io.EOFException closed) {
                    if (upgradeProbeOnly && state == STATE_AUTHENTICATED && !playbackStarted) return;
                    throw closed;
                }
                if (upgradeProbeOnly)
                    throw new IOException("v1 audio-v2 capability probe accepts no control commands after its query");
                checkHeartbeatDeadline(System.nanoTime(), lastHeartbeatAt);
                if (expectedCommandSequence > 0 && command.sequence == expectedCommandSequence - 1 && lastCommand != null
                        && Arrays.equals(command.raw, lastCommand)) {
                    P5cControlProtocol.write(output, lastAck);
                    continue;
                }
                if (command.sequence != expectedCommandSequence || expectedCommandSequence == Long.MAX_VALUE)
                    throw new IOException("P5-C replay or sequence gap");

                int result = RESULT_OK;
                int responsePort = 0;
                byte[] responseExtra = new byte[0];
                boolean stopAfterAck = false;
                try {
                    if (userStopRequested) {
                        if (playbackStarted) playback.stop();
                        else if (context != null) context.destroy();
                        playbackStarted = false;
                        state = STATE_STOPPED;
                        result = RESULT_USER_STOPPED;
                        stopAfterAck = true;
                    } else switch (command.type) {
                        case P5cControlProtocol.CAPABILITIES_QUERY:
                            if (state != STATE_AUTHENTICATED || command.payload.length != 8
                                    || command.payload[3] != 0)
                                throw new IOException("invalid capability query");
                            int requestedControl = P5cControlProtocol.getUnsignedShort(command.payload, 0);
                            int requestedAudio = command.payload[2] & 0xff;
                            int requiredCapabilities = getInt(command.payload, 4);
                            if (authenticated.protocolVersion == 1
                                    && (requiredCapabilities & P5cControlProtocol.CAP_AUDIO_V2) != 0) {
                                if (expectedCommandSequence != 0)
                                    throw new IOException("v1 audio-v2 capability probe must be the first control command");
                                upgradeProbeOnly = true;
                            }
                            if ("unsupported".equals(capabilityTestMode)) {
                                byte[] oldAck = P5cControlProtocol.encode(keys.receiverToHostControl,
                                        P5cControlProtocol.RECEIVER_TO_HOST, sessionId, ackSequence++,
                                        P5cControlProtocol.ACK, P5cControlProtocol.ackPayload(command.type,
                                                command.sequence, RESULT_REJECTED, state, 0));
                                P5cControlProtocol.write(output, oldAck);
                                lastCommand = command.raw; lastAck = oldAck; expectedCommandSequence++;
                                continue;
                            }
                            int controlMajor = P5cControlProtocol.MANAGEMENT_CONTROL_MAJOR;
                            int audioMajor = P5cControlProtocol.AUDIO_PROTOCOL_MAJOR;
                            int implemented = P5cControlProtocol.IMPLEMENTED_CAPABILITIES
                                    | (playback.supportsPlaybackProgress() ? P5cControlProtocol.CAP_PLAYBACK_PROGRESS : 0)
                                    | (connectionIntentRequired ? P5cControlProtocol.CAP_CONNECTION_INTENT : 0)
                                    | (backgroundAuthorizationRequired ? P5cControlProtocol.CAP_BACKGROUND_START_AUTH : 0)
                                    | (playback.supportsBackgroundTrustManagement()
                                            ? P5cControlProtocol.CAP_BACKGROUND_TRUST_MANAGEMENT : 0)
                                    | (playback.supportsAudioV2() ? P5cControlProtocol.CAP_AUDIO_V2 : 0)
                                    | (playback.supportsAudioV2() && formatV2 != null
                                            && formatV2.sampleFormat == 3
                                            && playback.supportsAudioFormatV2(formatV2)
                                            ? P5cControlProtocol.CAP_AUDIO_FLOAT32 : 0)
                                    | (playback.supportsPlaybackProgressExtension()
                                            ? P5cControlProtocol.CAP_PROGRESS_EXT
                                                    | P5cControlProtocol.CAP_BUFFER_METRICS : 0);
                            if ("incompatible".equals(capabilityTestMode)) audioMajor++;
                            if ("missing-capability".equals(capabilityTestMode))
                                implemented &= ~P5cControlProtocol.CAP_AUTHENTICATED_STATUS;
                            if ("legacy-terminal-cause".equals(capabilityTestMode))
                                implemented &= ~P5cControlProtocol.CAP_TERMINAL_CAUSE;
                            boolean compatible = requestedControl == controlMajor && requestedAudio == audioMajor
                                    && (formatV2 == null || playback.supportsAudioFormatV2(formatV2))
                                    && (requiredCapabilities & ~implemented) == 0;
                            long echoedQuery = "replay".equals(capabilityTestMode) ? command.sequence - 1 : command.sequence;
                            byte[] info = P5cControlProtocol.capabilitiesInfoPayload(echoedQuery,
                                    controlMajor, audioMajor, compatible ? RESULT_OK : RESULT_REJECTED,
                                    implemented, receiverVersionCode);
                            byte[] response = P5cControlProtocol.encode(keys.receiverToHostControl,
                                    P5cControlProtocol.RECEIVER_TO_HOST, sessionId, ackSequence++,
                                    P5cControlProtocol.CAPABILITIES_INFO, info);
                            P5cControlProtocol.write(output, response);
                            if (compatible) negotiatedCapabilities = implemented;
                            terminalCauseNegotiated = (implemented
                                    & P5cControlProtocol.CAP_TERMINAL_CAUSE) != 0;
                            lastCommand = command.raw; lastAck = response; expectedCommandSequence++;
                            continue;
                        case P5cControlProtocol.CONNECTION_INTENT:
                            if (state != STATE_AUTHENTICATED || command.payload.length != 16
                                    || !connectionIntentRequired || !acceptsConnectionIntent(command.payload)) {
                                result = RESULT_REJECTED;
                            } else claimedConnectionIntent = command.payload.clone();
                            break;
                        case P5cControlProtocol.BACKGROUND_START_REQUEST:
                            responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(command.payload,
                                    P5cControlProtocol.AUTH_EXPIRED);
                            if (backgroundTrustRevoked || state != STATE_AUTHENTICATED || !backgroundAuthorizationRequired
                                    || command.payload.length != 16 || allZero(command.payload)) {
                                result = RESULT_REJECTED;
                            } else if (backgroundRequestId != null) {
                                if (Arrays.equals(backgroundRequestId, command.payload)) {
                                    responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(backgroundRequestId,
                                            backgroundAuthorizationStatus);
                                } else result = RESULT_REJECTED;
                            } else {
                                backgroundRequestId = command.payload.clone();
                                backgroundAuthorizationStatus = playback.requestBackgroundStartAuthorization(
                                        backgroundRequestId.clone(), sessionId, socket.getInetAddress(), bindingCommitment.clone());
                                if (!validBackgroundAuthorizationStatus(backgroundAuthorizationStatus))
                                    throw new IOException("invalid background authorization status");
                                responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(backgroundRequestId,
                                        backgroundAuthorizationStatus);
                            }
                            break;
                        case P5cControlProtocol.BACKGROUND_START_STATUS:
                            responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(command.payload,
                                    P5cControlProtocol.AUTH_EXPIRED);
                            if (state != STATE_AUTHENTICATED || !backgroundAuthorizationRequired
                                    || command.payload.length != 16 || backgroundRequestId == null
                                    || !Arrays.equals(backgroundRequestId, command.payload)) {
                                result = RESULT_REJECTED;
                            } else {
                                backgroundAuthorizationStatus = playback.backgroundStartAuthorizationStatus(
                                        backgroundRequestId.clone(), sessionId, socket.getInetAddress(), bindingCommitment.clone());
                                if (!validBackgroundAuthorizationStatus(backgroundAuthorizationStatus))
                                    throw new IOException("invalid background authorization status");
                                responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(backgroundRequestId,
                                        backgroundAuthorizationStatus);
                            }
                            break;
                        case P5cControlProtocol.BACKGROUND_START_CANCEL:
                            responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(command.payload,
                                    P5cControlProtocol.AUTH_EXPIRED);
                            if (state != STATE_AUTHENTICATED || !backgroundAuthorizationRequired
                                    || command.payload.length != 16 || backgroundRequestId == null
                                    || !Arrays.equals(backgroundRequestId, command.payload) || backgroundAuthorizationStarted) {
                                result = RESULT_REJECTED;
                            } else {
                                playback.cancelBackgroundStartAuthorization(backgroundRequestId.clone(), sessionId,
                                        socket.getInetAddress(), bindingCommitment.clone());
                                backgroundAuthorizationEnded = true;
                                backgroundAuthorizationStatus = P5cControlProtocol.AUTH_CANCELLED;
                                responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(backgroundRequestId,
                                        backgroundAuthorizationStatus);
                            }
                            break;
                        case P5cControlProtocol.BACKGROUND_TRUST_QUERY:
                            if (!supportsBackgroundTrustManagementState(state) || command.payload.length != 32
                                    || !playback.supportsBackgroundTrustManagement()
                                    || !MessageDigest.isEqual(command.payload, bindingCommitment)) {
                                result = RESULT_REJECTED;
                            } else {
                                BackgroundTrustSnapshot trust = playback.queryBackgroundTrust(command.payload.clone());
                                responseExtra = P5cControlProtocol.backgroundTrustAckExtra(command.payload,
                                        trust.kind, trust.persistence, trust.remainingMs);
                            }
                            break;
                        case P5cControlProtocol.BACKGROUND_TRUST_REVOKE:
                            if (!supportsBackgroundTrustManagementState(state) || command.payload.length != 32
                                    || !playback.supportsBackgroundTrustManagement()
                                    || !MessageDigest.isEqual(command.payload, bindingCommitment)) {
                                result = RESULT_REJECTED;
                            } else {
                                backgroundTrustRevoked = true;
                                if (!playback.revokeBackgroundTrust(command.payload.clone())) {
                                    result = RESULT_FAILED;
                                } else {
                                    backgroundAuthorizationEnded = true;
                                    if (playbackStarted) playback.stop();
                                    if (context != null) { context.destroy(); context = null; }
                                    if (audioSocket != null) { audioSocket.close(); audioSocket = null; }
                                    playbackStarted = false;
                                    state = STATE_STOPPED;
                                    responseExtra = P5cControlProtocol.backgroundTrustAckExtra(command.payload, 0, 0, 0);
                                    stopAfterAck = true;
                                }
                            }
                            break;
                        case P5cControlProtocol.BIND:
                            if (formatV2 != null && !playback.supportsAudioFormatV2(formatV2)) {
                                result = RESULT_REJECTED;
                                break;
                            }
                            boolean bindBackgroundGrant = !backgroundAuthorizationRequired
                                    || backgroundRequestId != null && backgroundAuthorizationGranted(
                                        backgroundAuthorizationStatus = queryBackgroundStartAuthorization(backgroundRequestId,
                                            sessionId, socket.getInetAddress(), bindingCommitment));
                            if (backgroundTrustRevoked || !bindBackgroundGrant
                                    || !acceptsConnectionIntentOrBackgroundGrant(claimedConnectionIntent,
                                            backgroundAuthorizationRequired, bindBackgroundGrant)) {
                                result = RESULT_REJECTED;
                                break;
                            }
                            if (state != STATE_AUTHENTICATED || command.payload.length != 2)
                                throw new IOException("invalid BIND state");
                            int hostPort = P5cControlProtocol.getUnsignedShort(command.payload, 0);
                            if (hostPort == 0) throw new IOException("invalid BIND port");
                            DatagramSocket candidateSocket = new DatagramSocket(null);
                            try {
                                candidateSocket.setReuseAddress(false);
                                candidateSocket.bind(new InetSocketAddress(listener.getInetAddress(), 0));
                                InetSocketAddress expectedPeer = new InetSocketAddress(socket.getInetAddress(), hostPort);
                                UdpSessionContext candidateContext = formatV2 == null
                                        ? new UdpSessionContext(candidateSocket, expectedPeer, sessionId,
                                                keys.hostToReceiverAudio, parameters)
                                        : new UdpSessionContext(candidateSocket, expectedPeer, sessionId,
                                                keys.hostToReceiverAudio, formatV2);
                                audioSocket = candidateSocket;
                                context = candidateContext;
                            } catch (IOException | RuntimeException error) {
                                candidateSocket.close();
                                throw error;
                            }
                            responsePort = audioSocket.getLocalPort();
                            state = STATE_TRANSPORT_CONNECTED;
                            break;
                        case P5cControlProtocol.SELECT_MODE:
                            if (state != STATE_TRANSPORT_CONNECTED || command.payload.length != 1
                                    || (command.payload[0] & 0xff) != audioMode)
                                result = RESULT_REJECTED;
                            else modeSelected = true;
                            break;
                        case P5cControlProtocol.START:
                            final byte[] startIntent = claimedConnectionIntent;
                            final byte[] startRequestId = backgroundRequestId == null ? null : backgroundRequestId.clone();
                            final byte[] startBindingCommitment = bindingCommitment == null ? null : bindingCommitment.clone();
                            final InetAddress startPeer = socket.getInetAddress();
                            final boolean startBackgroundGrant = !backgroundAuthorizationRequired
                                    || startRequestId != null && backgroundAuthorizationGranted(
                                        backgroundAuthorizationStatus = queryBackgroundStartAuthorization(startRequestId,
                                            sessionId, startPeer, startBindingCommitment));
                            if (backgroundTrustRevoked || !startBackgroundGrant
                                    || state != STATE_TRANSPORT_CONNECTED || !modeSelected || command.payload.length != 0 || context == null
                                    || !acceptsConnectionIntentOrBackgroundGrant(claimedConnectionIntent,
                                            backgroundAuthorizationRequired, startBackgroundGrant)) {
                                result = RESULT_REJECTED;
                            } else if (!playback.start(context, () -> {
                                boolean stillGranted = !backgroundAuthorizationRequired
                                        || backgroundStartStillAuthorized(startRequestId, sessionId, startPeer,
                                                startBindingCommitment);
                                return stillGranted && acceptsConnectionIntentOrBackgroundGrant(startIntent,
                                        backgroundAuthorizationRequired, stillGranted);
                            })) {
                                playback.stop();
                                result = RESULT_FAILED;
                                context.destroy();
                                context = null;
                                audioSocket = null;
                                state = STATE_AUTHENTICATED;
                                modeSelected = false;
                            } else {
                                playbackStarted = true;
                                backgroundAuthorizationStarted = backgroundRequestId != null;
                                context = null;
                                audioSocket = null;
                                boolean accepted;
                                synchronized (this) {
                                    boolean stillGranted = !backgroundAuthorizationRequired
                                            || backgroundStartStillAuthorized(startRequestId, sessionId, startPeer,
                                                    startBindingCommitment);
                                    accepted = stillGranted && acceptsConnectionIntentOrBackgroundGrant(
                                            claimedConnectionIntent, backgroundAuthorizationRequired, stillGranted);
                                    if (accepted && connectionIntentRequired && claimedConnectionIntent != null) {
                                        connectionIntentEstablished = true;
                                        establishedConnectionIntentNonce = claimedConnectionIntent.clone();
                                    }
                                }
                                if (!accepted) {
                                    playback.stop();
                                    playbackStarted = false;
                                    throw new IOException("Receiver connection intent was cancelled during start.");
                                }
                                if (backgroundAuthorizationRequired)
                                    playback.backgroundPlaybackStarted(startRequestId, sessionId, startPeer, startBindingCommitment);
                                state = STATE_RECEIVER_RUNNING;
                            }
                            break;
                        case P5cControlProtocol.PAUSE:
                            if (command.payload.length != 0)
                                result = RESULT_REJECTED;
                            else if (state == STATE_RECEIVER_RUNNING || state == STATE_STREAMING) {
                                playback.pause(); state = STATE_PAUSED;
                            } else if (state != STATE_PAUSED) result = RESULT_REJECTED;
                            break;
                        case P5cControlProtocol.RESUME:
                            if (command.payload.length != 0) result = RESULT_REJECTED;
                            else if (state == STATE_PAUSED) { playback.resume(); state = STATE_RECEIVER_RUNNING; }
                            else if (state != STATE_RECEIVER_RUNNING && state != STATE_STREAMING) result = RESULT_REJECTED;
                            break;
                        case P5cControlProtocol.STOP:
                            if (command.payload.length != 0) result = RESULT_REJECTED;
                            else {
                                if (playbackStarted) playback.stop();
                                else if (context != null) context.destroy();
                                playbackStarted = false;
                                state = STATE_STOPPED;
                                shutdownRequested = !playback.keepStandbyAfterPlaybackStop();
                            }
                            break;
                        case P5cControlProtocol.HEARTBEAT:
                            if (command.payload.length != 0) result = RESULT_REJECTED;
                            else {
                                lastHeartbeatAt = System.nanoTime();
                                if (playbackStarted && state == STATE_RECEIVER_RUNNING && playback.streaming())
                                    state = STATE_STREAMING;
                                else if (state == STATE_STREAMING && !playback.streaming())
                                    state = STATE_RECEIVER_RUNNING;
                            }
                            break;
                        case P5cControlProtocol.PLAYBACK_PROGRESS_QUERY:
                            if (command.payload.length == 0) responseExtra = formatV2 == null
                                    ? playback.playbackProgressPayload() : playback.playbackProgressPayloadV2();
                            else if (command.payload.length == 1 && command.payload[0] == 2
                                    && (negotiatedCapabilities & P5cControlProtocol.CAP_PROGRESS_EXT) != 0) {
                                responseExtra = playback.playbackProgressPayloadExtended(formatV2 != null);
                                if (responseExtra == null) result = RESULT_FAILED;
                            } else if (command.payload.length == 1 && command.payload[0] == 3
                                    && (negotiatedCapabilities & P5cControlProtocol.CAP_PROGRESS_EXT) != 0
                                    && (negotiatedCapabilities & P5cControlProtocol.CAP_BUFFER_METRICS) != 0) {
                                responseExtra = playback.playbackProgressPayloadExtended(formatV2 != null, 3);
                                if (responseExtra == null) result = RESULT_FAILED;
                            } else result = RESULT_REJECTED;
                            break;
                        default:
                            result = RESULT_REJECTED;
                    }
                } catch (IOException | RuntimeException error) {
                    if (command.type == P5cControlProtocol.CAPABILITIES_QUERY) {
                        if (error instanceof IOException ioError) throw ioError;
                        throw new IOException("invalid capability query", error);
                    }
                    result = RESULT_FAILED;
                    if (command.type == P5cControlProtocol.BACKGROUND_START_REQUEST
                            || command.type == P5cControlProtocol.BACKGROUND_START_STATUS
                            || command.type == P5cControlProtocol.BACKGROUND_START_CANCEL) {
                        if (responseExtra.length != 17) responseExtra = P5cControlProtocol.backgroundAuthorizationAckExtra(
                                command.payload.length == 16 ? command.payload : new byte[16], P5cControlProtocol.AUTH_EXPIRED);
                    }
                    if (command.type == P5cControlProtocol.BACKGROUND_TRUST_QUERY
                            || command.type == P5cControlProtocol.BACKGROUND_TRUST_REVOKE) {
                        if (responseExtra.length != 42 && command.payload.length == 32)
                            responseExtra = P5cControlProtocol.backgroundTrustAckExtra(command.payload,
                                    0, 0, 0);
                    }
                    if (command.type == P5cControlProtocol.START && !playbackStarted) {
                        playback.stop();
                        if (context != null) context.destroy();
                        context = null;
                        if (audioSocket != null) audioSocket.close();
                        audioSocket = null;
                        state = STATE_AUTHENTICATED;
                        modeSelected = false;
                    }
                    if (command.type == P5cControlProtocol.HEARTBEAT) shutdownRequested = true;
                }

                byte[] ackPayload = P5cControlProtocol.ackPayload(command.type, command.sequence,
                        result, state, responsePort, responseExtra);
                if (backgroundAuthorizationTestMode.equals("wrong-id")
                        && command.type == P5cControlProtocol.BACKGROUND_START_REQUEST && responseExtra.length == 17)
                    ackPayload[13] ^= 1;
                if (backgroundTrustTestMode.equals("wrong-binding")
                        && command.type == P5cControlProtocol.BACKGROUND_TRUST_QUERY && responseExtra.length == 42)
                    ackPayload[13] ^= 1;
                byte[] ack = P5cControlProtocol.encode(keys.receiverToHostControl,
                        P5cControlProtocol.RECEIVER_TO_HOST, sessionId, ackSequence++,
                        P5cControlProtocol.ACK, ackPayload);
                if (dropStopAckForTest && command.type == P5cControlProtocol.STOP && result == RESULT_OK)
                    throw new IOException("test dropped the authenticated STOP ACK");
                P5cControlProtocol.write(output, ack);
                lastCommand = command.raw;
                lastAck = ack;
                expectedCommandSequence++;
                if (stopAfterAck) { shutdownRequested = true; return; }
                if (shutdownRequested) return;
                if (command.type == P5cControlProtocol.STOP && result == RESULT_OK) return;
            }
                    } finally {
                        if (backgroundRequestId != null && !backgroundAuthorizationEnded) {
                            playback.endBackgroundStartAuthorization(backgroundRequestId.clone(), sessionId,
                                    socket.getInetAddress(), bindingCommitment.clone(), backgroundAuthorizationStarted,
                                    backgroundAuthorizationStarted && playbackStarted);
                        }
                        if (playbackStarted) playback.stop();
                        if (context != null) context.destroy();
                        if (audioSocket != null) audioSocket.close();
                        if (bindingCommitment != null) Arrays.fill(bindingCommitment, (byte) 0);
                    }
                }
        }

    private SessionMaterial acceptSessionHandshake(Socket socket, DataInputStream input,
            DataOutputStream output, long deadline) throws IOException, PairingProtocol.Failure {
        byte[] prefix = readHandshake(input, socket, 6, deadline);
        if (prefix[0] != 'A' || prefix[1] != 'N' || prefix[2] != 'P' || prefix[3] != '5'
                || (prefix[5] & 0xff) != 1)
            throw new IOException("invalid P5-A HELLO prefix");
        int protocolVersion = prefix[4] & 0xff;
        if (protocolVersion == 1) {
            byte[] hello = readHandshakeTail(input, socket, prefix, 85, deadline);
            PairingProtocol.ReceiverHandshake handshake =
                    PairingProtocol.ReceiverHandshake.acceptHello(binding.duplicate(), hello);
            try (handshake) {
                output.write(handshake.challenge()); // fixed 86-byte v1 CHALLENGE
                output.flush();
                PairingProtocol.FinishResult complete = handshake.acceptFinish(readHandshake(input, socket, 38, deadline));
                if (!claimAuthenticated(socket)) {
                    complete.session.close();
                    throw new IOException("P5-A authenticated session already has an owner");
                }
                try (PairingProtocol.AuthenticatedSession session = complete.session) {
                    return new SessionMaterial(1, session.sessionId, session.parameters, null,
                            session.takeKeys(), complete.ready);
                }
            }
        }
        if (protocolVersion == 2) {
            byte[] lengthBytes = readHandshake(input, socket, 2, deadline);
            int declaredLength = ((lengthBytes[0] & 0xff) << 8) | (lengthBytes[1] & 0xff);
            if (declaredLength != 96)
                throw new IOException("invalid fixed-size P5-A v2 HELLO length");
            byte[] v2Prefix = Arrays.copyOf(prefix, 8);
            System.arraycopy(lengthBytes, 0, v2Prefix, 6, 2);
            byte[] hello = readHandshakeTail(input, socket, v2Prefix, declaredLength, deadline);
            PairingProtocol.V2.ReceiverAwaitFinish handshake =
                    PairingProtocol.V2.Receiver.acceptHello(binding.duplicate(), hello);
            try (handshake) {
                output.write(handshake.challenge); // fixed 104-byte v2 CHALLENGE
                output.flush();
                PairingProtocol.V2.FinishResult complete =
                        handshake.acceptFinish(readHandshake(input, socket, 40, deadline));
                if (!claimAuthenticated(socket)) {
                    complete.session.close();
                    throw new IOException("P5-A authenticated session already has an owner");
                }
                try (PairingProtocol.V2.AuthenticatedSessionV2 session = complete.session) {
                    return new SessionMaterial(2, session.sessionId, null, session.format,
                            session.takeKeys(), complete.ready);
                }
            }
        }
        throw new IOException("unsupported P5-A protocol version");
    }

    private static byte[] readHandshakeTail(DataInputStream input, Socket socket,
            byte[] prefix, int totalLength, long deadline) throws IOException {
        if (prefix == null || prefix.length > totalLength)
            throw new IOException("invalid P5-A handshake prefix length");
        byte[] message = Arrays.copyOf(prefix, totalLength);
        byte[] tail = readHandshake(input, socket, totalLength - prefix.length, deadline);
        System.arraycopy(tail, 0, message, prefix.length, tail.length);
        return message;
    }

    private static byte[] readHandshake(DataInputStream input, Socket socket, int length, long deadline)
            throws IOException {
        byte[] bytes = new byte[length];
        int offset = 0;
        while (offset < bytes.length) {
            long remaining = deadline - System.nanoTime();
            if (remaining <= 0) throw new java.net.SocketTimeoutException("P5-A total handshake deadline expired");
            long timeoutMs = Math.max(1, TimeUnit.NANOSECONDS.toMillis(remaining));
            socket.setSoTimeout((int) Math.min(Integer.MAX_VALUE, timeoutMs));
            int read = input.read(bytes, offset, bytes.length - offset);
            if (read < 0) throw new java.io.EOFException("P5-A handshake peer closed");
            offset += read;
        }
        if (System.nanoTime() >= deadline)
            throw new java.net.SocketTimeoutException("P5-A total handshake deadline expired");
        return bytes;
    }

    private static int getInt(byte[] bytes, int offset) {
        return ((bytes[offset] & 0xff) << 24) | ((bytes[offset + 1] & 0xff) << 16)
                | ((bytes[offset + 2] & 0xff) << 8) | (bytes[offset + 3] & 0xff);
    }

    static void checkHeartbeatDeadline(long now, long lastHeartbeatAt) throws IOException {
        if (now - lastHeartbeatAt > HEARTBEAT_TIMEOUT_NANOS)
            throw new IOException("authenticated heartbeat expired");
    }

    @Override public void close() {
        synchronized (this) {
            connectionIntentNonce = null;
            establishedConnectionIntentNonce = null;
            connectionIntentEstablished = false;
        }
        stopDiscovery();
        try { listener.close(); } catch (IOException ignored) {}
        Socket[] pending;
        synchronized (this) { pending = preAuthSockets.keySet().toArray(new Socket[0]); }
        for (Socket socket : pending)
            try { socket.close(); } catch (IOException ignored) {}
        Socket socket = authenticatedSocket;
        if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        preAuthWorkers.shutdownNow();
        handshakeDeadline.shutdownNow();
        Thread thread = worker;
        if (thread != null && thread != Thread.currentThread()) {
            try { thread.join(2_000); } catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        }
        try { preAuthWorkers.awaitTermination(2, TimeUnit.SECONDS); }
        catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        binding.close();
    }

    private synchronized void stopDiscovery() {
        P5cDiscovery current = discovery;
        if (current != null) {
            discovery = null;
            current.close();
        }
    }
}
