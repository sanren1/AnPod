package app.anpod.receiver;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.DatagramSocket;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
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
    private static final long HEARTBEAT_TIMEOUT_NANOS = TimeUnit.SECONDS.toNanos(5);

    interface PlaybackEndpoint {
        /** Takes ownership of the already authenticated and UDP-peer-bound context on success. */
        boolean start(UdpSessionContext context) throws IOException;
        void pause() throws IOException;
        void resume() throws IOException;
        void stop();
        boolean streaming() throws IOException;
        void shutdown();
    }

    private final ServerSocket listener;
    private final PairingProtocol.BindingMaterial binding;
    private final PlaybackEndpoint playback;
    private final boolean loopbackOnly;
    private final boolean dropStopAckForTest;
    private final int discoveryPrefixLength;
    private final CountDownLatch finished = new CountDownLatch(1);
    private volatile Socket activeSocket;
    private volatile Thread worker;
    private volatile P5cDiscovery discovery;
    private volatile String failure = "";
    private volatile boolean shutdownRequested;

    static P5cReceiverCoordinator openLoopback(PairingProtocol.BindingMaterial existingBinding,
            PlaybackEndpoint playback) throws IOException {
        return open(existingBinding, playback, InetAddress.getByName("127.0.0.1"), true, 32, false);
    }

    static P5cReceiverCoordinator openLoopbackDropStopAckForTest(PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback) throws IOException {
        return open(binding, playback, InetAddress.getByName("127.0.0.1"), false, 32, true);
    }

    /** Explicit internal LAN entry; it never starts discovery or playback by itself. */
    static P5cReceiverCoordinator openOnInterface(PairingProtocol.BindingMaterial existingBinding,
            PlaybackEndpoint playback, InetAddress localAddress, int prefixLength) throws IOException {
        if (!validInterfaceAddress(localAddress, prefixLength))
            throw new IOException("P5-C requires an explicit private IPv4 interface address and prefix");
        if (!isAssignedInterfaceAddress(localAddress, prefixLength))
            throw new IOException("P5-C interface address is not assigned to an active Android interface");
        return open(existingBinding, playback, localAddress, false, prefixLength, false);
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
            boolean dropStopAckForTest) throws IOException {
        ServerSocket listener = new ServerSocket();
        try {
            listener.setReuseAddress(false);
            listener.bind(new InetSocketAddress(localAddress, 0));
            return new P5cReceiverCoordinator(listener, existingBinding, playback, loopbackOnly, prefixLength, dropStopAckForTest);
        } catch (IOException | RuntimeException error) {
            listener.close();
            throw error;
        }
    }

    private P5cReceiverCoordinator(ServerSocket listener, PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback, boolean loopbackOnly, int discoveryPrefixLength, boolean dropStopAckForTest) {
        if (binding == null || playback == null) throw new IllegalArgumentException("P5-C inputs are required");
        this.listener = listener;
        this.binding = binding;
        this.playback = playback;
        this.loopbackOnly = loopbackOnly;
        this.dropStopAckForTest = dropStopAckForTest;
        this.discoveryPrefixLength = discoveryPrefixLength;
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
        if (worker == null) throw new IllegalStateException("P5-C control listener must be started first");
        if (discovery != null) throw new IllegalStateException("P5-C discovery already started");
        P5cDiscovery service = P5cDiscovery.open(listener.getInetAddress(), discoveryPrefixLength, listener.getLocalPort());
        service.start();
        discovery = service;
    }

    boolean await(long timeout, TimeUnit unit) throws InterruptedException {
        return finished.await(timeout, unit);
    }

    String failure() { return failure; }

    private void serve() {
        try {
            while (!listener.isClosed()) {
                try (Socket socket = listener.accept()) {
                    stopDiscovery();
                    activeSocket = socket;
                    socket.setSoTimeout(250);
                    if (loopbackOnly && !socket.getInetAddress().isLoopbackAddress()) throw new IOException("loopback peer required");
                    serveSession(socket);
                    if (shutdownRequested || loopbackOnly) break;
                } catch (IOException | PairingProtocol.Failure | RuntimeException error) {
                    if (shutdownRequested) break;
                    if (listener.isClosed()) break;
                    failure = "session_failed";
                } finally { activeSocket = null; }
                if (loopbackOnly) break;
                if (!listener.isClosed()) startDiscovery();
            }
        } catch (IOException | RuntimeException error) {
            if (!listener.isClosed()) failure = "listener_failed";
        } finally {
            stopDiscovery();
            activeSocket = null;
            if (shutdownRequested) playback.shutdown();
            binding.close();
            try { listener.close(); } catch (IOException ignored) {}
            finished.countDown();
        }
    }

    private void serveSession(Socket socket) throws IOException, PairingProtocol.Failure {
        DataInputStream input = new DataInputStream(socket.getInputStream());
        DataOutputStream output = new DataOutputStream(socket.getOutputStream());
        byte[] hello = new byte[85];
        input.readFully(hello);
        PairingProtocol.ReceiverHandshake handshake = PairingProtocol.ReceiverHandshake.acceptHello(binding.duplicate(), hello);
        try (handshake) {
            output.write(handshake.challenge());
            output.flush();
            byte[] finish = new byte[38];
            input.readFully(finish);
            PairingProtocol.FinishResult complete = handshake.acceptFinish(finish);
            try (PairingProtocol.AuthenticatedSession authenticated = complete.session) {
                PairingProtocol.SessionKeys keys = authenticated.takeKeys();
                try (keys) {
                    long sessionId = authenticated.sessionId;
                    PairingProtocol.AudioParameters parameters = authenticated.parameters;
                    DatagramSocket audioSocket = null;
                    UdpSessionContext context = null;
                    boolean playbackStarted = false;
                    int state = STATE_AUTHENTICATED;
                    long expectedCommandSequence = 0;
                    long ackSequence = 0;
                    byte[] lastCommand = null;
                    byte[] lastAck = null;
                    boolean modeSelected = false;
                    long lastHeartbeatAt = System.nanoTime();
                    try {
                        // Keep the authenticated session/keys owned even if READY cannot be written.
                        output.write(complete.ready);
                        output.flush();
            while (true) {
                checkHeartbeatDeadline(System.nanoTime(), lastHeartbeatAt);
                P5cControlProtocol.Frame command;
                try {
                    command = P5cControlProtocol.read(input, keys.hostToReceiverControl,
                            P5cControlProtocol.HOST_TO_RECEIVER, sessionId);
                } catch (P5cControlProtocol.IdleTimeoutException idle) {
                    continue;
                }
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
                try {
                    switch (command.type) {
                        case P5cControlProtocol.BIND:
                            if (state != STATE_AUTHENTICATED || command.payload.length != 2)
                                throw new IOException("invalid BIND state");
                            int hostPort = P5cControlProtocol.getUnsignedShort(command.payload, 0);
                            if (hostPort == 0) throw new IOException("invalid BIND port");
                            DatagramSocket candidateSocket = new DatagramSocket(null);
                            try {
                                candidateSocket.setReuseAddress(false);
                                candidateSocket.bind(new InetSocketAddress(listener.getInetAddress(), 0));
                                InetSocketAddress expectedPeer = new InetSocketAddress(socket.getInetAddress(), hostPort);
                                UdpSessionContext candidateContext = new UdpSessionContext(candidateSocket, expectedPeer, sessionId,
                                        keys.hostToReceiverAudio, parameters);
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
                                    || (command.payload[0] & 0xff) != parameters.mode)
                                result = RESULT_REJECTED;
                            else modeSelected = true;
                            break;
                        case P5cControlProtocol.START:
                            if (state != STATE_TRANSPORT_CONNECTED || !modeSelected || command.payload.length != 0 || context == null) {
                                result = RESULT_REJECTED;
                            } else if (!playback.start(context)) {
                                playback.stop();
                                result = RESULT_FAILED;
                                context.destroy();
                                context = null;
                                audioSocket = null;
                            } else {
                                playbackStarted = true;
                                context = null;
                                audioSocket = null;
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
                                shutdownRequested = true;
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
                        default:
                            result = RESULT_REJECTED;
                    }
                } catch (IOException | RuntimeException error) {
                    result = RESULT_FAILED;
                    if (command.type == P5cControlProtocol.START && !playbackStarted) playback.stop();
                    if (command.type == P5cControlProtocol.HEARTBEAT) shutdownRequested = true;
                }

                byte[] ackPayload = P5cControlProtocol.ackPayload(command.type, command.sequence,
                        result, state, responsePort);
                byte[] ack = P5cControlProtocol.encode(keys.receiverToHostControl,
                        P5cControlProtocol.RECEIVER_TO_HOST, sessionId, ackSequence++,
                        P5cControlProtocol.ACK, ackPayload);
                if (dropStopAckForTest && command.type == P5cControlProtocol.STOP && result == RESULT_OK)
                    throw new IOException("test dropped the authenticated STOP ACK");
                P5cControlProtocol.write(output, ack);
                lastCommand = command.raw;
                lastAck = ack;
                expectedCommandSequence++;
                if (shutdownRequested) return;
                if (command.type == P5cControlProtocol.STOP && result == RESULT_OK) return;
            }
                    } finally {
                        if (playbackStarted) playback.stop();
                        if (context != null) context.destroy();
                        if (audioSocket != null) audioSocket.close();
                    }
                }
            }
        }
    }

    static void checkHeartbeatDeadline(long now, long lastHeartbeatAt) throws IOException {
        if (now - lastHeartbeatAt > HEARTBEAT_TIMEOUT_NANOS)
            throw new IOException("authenticated heartbeat expired");
    }

    @Override public void close() {
        stopDiscovery();
        try { listener.close(); } catch (IOException ignored) {}
        Socket socket = activeSocket;
        if (socket != null) try { socket.close(); } catch (IOException ignored) {}
        Thread thread = worker;
        if (thread != null && thread != Thread.currentThread()) {
            try { thread.join(2_000); } catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        }
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
