package app.anpod.receiver;

import java.io.IOException;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.NetworkInterface;
import java.net.SocketTimeoutException;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.List;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Set;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/** Explicit selected-interface discovery for an already running P5-C endpoint. */
final class P5cDiscovery implements AutoCloseable {
    interface SocketBinder { void bind(DatagramSocket socket) throws IOException; }
    static final int PORT = 49_772;
    private static final byte[] MAGIC = { 'A', 'N', 'P', 'D' };
    private static final int VERSION = 1;
    private static final int PROBE = 1;
    private static final int CANDIDATE = 2;
    private static final int FRAME_BYTES = 26;
    private static final int HOST_PROBE = 3;
    private static final int HOST_CANDIDATE = 4;
    private static final int CONNECT_OFFER = 5;
    private static final int AUTH_VERSION = 3;
    private static final int AUTH_PROBE = 6;
    private static final int AUTH_CANDIDATE = 7;
    private static final int AUTH_FRAME_BYTES = 50;
    private static final long AUTH_REPLY_INTERVAL_MILLIS = 200L;
    private static final long SEARCH_MILLIS = 10_000L;
    private static final SecureRandom RANDOM = new SecureRandom();

    private final DatagramSocket socket;
    private final DatagramSocket responseSocket;
    private final InetAddress localAddress;
    private final int prefixLength;
    private final int controlPort;
    private final byte[] bindingSecret;
    private long lastAuthReplyAt;
    private volatile Thread worker;
    private byte[] searchNonce;
    private long searchDeadline;
    private boolean automaticRequests;
    private final ArrayList<SearchCandidate> candidates = new ArrayList<>();
    private final Set<String> requestedCandidates = new HashSet<>();
    private String lastRequestStatus = "idle";

    static final class SearchCandidate {
        final String id;
        final String address;
        final int offerPort;
        SearchCandidate(InetAddress address, int offerPort) {
            this.address = address.getHostAddress(); this.offerPort = offerPort;
            this.id = this.address + ":" + offerPort;
        }
    }

    static final class SearchSnapshot {
        final boolean active;
        final List<SearchCandidate> candidates;
        final String lastRequestStatus;
        SearchSnapshot(boolean active, List<SearchCandidate> candidates, String lastRequestStatus) {
            this.active = active; this.candidates = candidates; this.lastRequestStatus = lastRequestStatus;
        }
    }

    static P5cDiscovery open(InetAddress localAddress, int prefixLength, int controlPort,
            SocketBinder socketBinder, byte[] bindingSecret) throws IOException {
        return openInternal(localAddress, prefixLength, controlPort, socketBinder, true, false, bindingSecret);
    }

    static P5cDiscovery openLoopbackForTest(InetAddress localAddress, int prefixLength, int controlPort) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        return openInternal(localAddress, prefixLength, controlPort, null, false, true, null);
    }

    private static P5cDiscovery openInternal(InetAddress localAddress, int prefixLength, int controlPort,
            SocketBinder socketBinder, boolean wildcardReceive, boolean allowLoopbackPrefix,
            byte[] bindingSecret) throws IOException {
        if (!(localAddress instanceof Inet4Address) || prefixLength < 1 || prefixLength > 32
                || (!localAddress.isLoopbackAddress() && !localAddress.isSiteLocalAddress()))
            throw new IOException("Discovery requires an explicit private IPv4 address and prefix");
        if (localAddress.isLoopbackAddress() && prefixLength != 32 && !allowLoopbackPrefix)
            throw new IOException("Loopback discovery test interface must use prefix length 32");
        DatagramSocket socket = new DatagramSocket(null);
        DatagramSocket responseSocket = null;
        try {
            responseSocket = new DatagramSocket(null);
            socket.setReuseAddress(false);
            socket.setBroadcast(true);
            socket.setSoTimeout(250);
            if (socketBinder != null) socketBinder.bind(socket);
            socket.bind(new InetSocketAddress(wildcardReceive ? InetAddress.getByName("0.0.0.0") : localAddress, PORT));
            responseSocket.setReuseAddress(false);
            if (socketBinder != null) socketBinder.bind(responseSocket);
            responseSocket.bind(new InetSocketAddress(localAddress, 0));
            return new P5cDiscovery(socket, responseSocket, localAddress, prefixLength, controlPort, bindingSecret);
        } catch (IOException | RuntimeException error) {
            socket.close();
            if (responseSocket != null) responseSocket.close();
            throw error;
        }
    }

    private P5cDiscovery(DatagramSocket socket, DatagramSocket responseSocket,
            InetAddress localAddress, int prefixLength, int controlPort, byte[] bindingSecret) {
        this.socket = socket;
        this.responseSocket = responseSocket;
        this.localAddress = localAddress;
        this.prefixLength = prefixLength;
        this.controlPort = controlPort;
        this.bindingSecret = bindingSecret == null ? null : bindingSecret.clone();
    }

    synchronized void start() {
        if (worker != null) throw new IllegalStateException("P5-C discovery already started");
        worker = new Thread(this::serve, "AnPod P5-C discovery");
        worker.setDaemon(true);
        worker.start();
    }

    synchronized SearchSnapshot startSearch() throws IOException {
        return startSearch(false);
    }

    synchronized SearchSnapshot startAutomaticSearch() throws IOException {
        return startSearch(true);
    }

    private SearchSnapshot startSearch(boolean automatic) throws IOException {
        if (socket.isClosed() || worker == null) throw new IOException("Receiver discovery is unavailable.");
        requireInterface();
        if (prefixLength > 30 && !localAddress.isLoopbackAddress())
            throw new IOException("所选接口不支持 LAN 广播搜索，请选择带有向广播地址的 IPv4 接口。");
        return beginSearch(directedBroadcast(localAddress, prefixLength), automatic);
    }

    synchronized SearchSnapshot startSearchForTest(InetAddress target) throws IOException {
        return startSearchForTest(target, false);
    }

    synchronized SearchSnapshot startSearchForTest(InetAddress target, boolean automatic) throws IOException {
        if (!BuildConfig.DEBUG) throw new IllegalStateException("test hook requires a debug build");
        if (!localAddress.isLoopbackAddress() || target == null || !target.isLoopbackAddress()
                || !isInSelectedPrefix(localAddress, prefixLength, target))
            throw new IOException("Loopback discovery test target is outside the selected interface.");
        requireInterface();
        return beginSearch(target, automatic);
    }

    private SearchSnapshot beginSearch(InetAddress target, boolean automatic) throws IOException {
        searchNonce = new byte[16];
        do { RANDOM.nextBytes(searchNonce); } while (allZero(searchNonce));
        searchDeadline = monotonicMillis() + SEARCH_MILLIS;
        automaticRequests = automatic;
        candidates.clear(); requestedCandidates.clear(); lastRequestStatus = "searching";
        byte[] probe = hostProbe(searchNonce);
        socket.setBroadcast(true);
        socket.send(new DatagramPacket(probe, probe.length, target, PORT));
        return searchSnapshot();
    }

    synchronized byte[] pendingConnectionNonce() throws IOException {
        if (searchNonce == null || monotonicMillis() >= searchDeadline)
            throw new IOException("Receiver connection intent has expired.");
        return searchNonce.clone();
    }

    synchronized void resumeAutomaticSearch(byte[] nonce, long remainingMillis) throws IOException {
        requireInterface();
        if (remainingMillis <= 0) return;
        searchNonce = nonce.clone();
        searchDeadline = monotonicMillis() + remainingMillis;
        automaticRequests = true;
        byte[] probe = hostProbe(searchNonce);
        socket.setBroadcast(true);
        socket.send(new DatagramPacket(probe, probe.length, directedBroadcast(localAddress, prefixLength), PORT));
    }

    synchronized SearchSnapshot stopSearch() {
        automaticRequests = false;
        searchNonce = null; searchDeadline = 0; candidates.clear(); requestedCandidates.clear(); lastRequestStatus = "stopped";
        return searchSnapshot();
    }

    synchronized SearchSnapshot searchSnapshot() {
        if (!socket.isClosed() && !interfaceIsAssigned()) invalidateInterface();
        boolean active = searchNonce != null && monotonicMillis() < searchDeadline;
        if (!active && searchNonce != null) { searchNonce = null; candidates.clear(); requestedCandidates.clear(); lastRequestStatus = "expired"; }
        return new SearchSnapshot(active, new ArrayList<>(candidates), lastRequestStatus);
    }

    synchronized String requestConnection(String candidateId) throws IOException {
        requireInterface();
        if (searchNonce == null || monotonicMillis() >= searchDeadline) {
            searchNonce = null; candidates.clear(); requestedCandidates.clear(); lastRequestStatus = "expired";
            throw new IOException("搜索已结束；请重新搜索后再请求连接。");
        }
        SearchCandidate candidate = null;
        for (SearchCandidate item : candidates) if (item.id.equals(candidateId)) candidate = item;
        if (candidate == null) throw new IOException("所选 PC 候选已失效；请重新搜索。");
        if (requestedCandidates.contains(candidate.id)) throw new IOException("已向所选 PC 发送连接请求；请等待其确认或重新搜索。");
        byte[] offer = hostPacket(CONNECT_OFFER, searchNonce, controlPort);
        socket.send(new DatagramPacket(offer, offer.length,
                new InetSocketAddress(InetAddress.getByName(candidate.address), candidate.offerPort)));
        requestedCandidates.add(candidate.id);
        lastRequestStatus = "sent";
        return lastRequestStatus;
    }

    private void serve() {
        byte[] buffer = new byte[128];
        while (!socket.isClosed()) {
            DatagramPacket request = new DatagramPacket(buffer, buffer.length);
            try {
                socket.receive(request);
                InetAddress source = request.getAddress();
                if (!(source instanceof Inet4Address)
                        || !isInSelectedPrefix(localAddress, prefixLength, source)) continue;
                if (isHostCandidate(request.getData(), request.getOffset(), request.getLength(), source)) {
                    synchronized (this) { acceptCandidate(request.getData(), request.getOffset(), source); }
                } else if (isAuthProbe(request.getData(), request.getOffset(), request.getLength())) {
                    respondAuthenticated(request);
                } else if (isProbe(request.getData(), request.getOffset(), request.getLength())) {
                    byte[] response = response(request.getData(), request.getOffset(), controlPort);
                    responseSocket.send(new DatagramPacket(response, response.length, request.getAddress(), request.getPort()));
                }
            } catch (SocketTimeoutException ignored) {
                if (!interfaceIsAssigned()) {
                    invalidateInterface();
                }
            } catch (IOException error) {
                if (!socket.isClosed()) close();
            }
        }
    }

    private void acceptCandidate(byte[] packet, int offset, InetAddress source) throws IOException {
        if (searchNonce == null || monotonicMillis() >= searchDeadline
                || !Arrays.equals(searchNonce, nonce(packet, offset))) return;
        int port = ((packet[offset + 24] & 0xff) << 8) | (packet[offset + 25] & 0xff);
        if (port != PORT) return;
        SearchCandidate item = new SearchCandidate(source, port);
        boolean known = false;
        for (SearchCandidate candidate : candidates) if (candidate.id.equals(item.id)) { known = true; break; }
        if (!known && candidates.size() < 16) {
            candidates.add(item);
            known = true;
        }
        if (automaticRequests && known && !requestedCandidates.contains(item.id)) requestConnection(item.id);
    }

    private static InetAddress directedBroadcast(InetAddress address, int prefixLength) throws IOException {
        byte[] bytes = address.getAddress().clone();
        for (int bit = prefixLength; bit < 32; bit++) bytes[bit / 8] |= (byte) (1 << (7 - (bit % 8)));
        return InetAddress.getByAddress(bytes);
    }

    static byte[] hostProbe(byte[] nonce) {
        byte[] packet = new byte[24]; System.arraycopy(MAGIC, 0, packet, 0, 4);
        packet[4] = 2; packet[5] = HOST_PROBE; System.arraycopy(nonce, 0, packet, 8, 16); return packet;
    }

    static byte[] hostPacket(int type, byte[] nonce, int port) {
        byte[] packet = new byte[26]; System.arraycopy(MAGIC, 0, packet, 0, 4);
        packet[4] = 2; packet[5] = (byte) type; System.arraycopy(nonce, 0, packet, 8, 16);
        packet[24] = (byte) (port >>> 8); packet[25] = (byte) port; return packet;
    }

    private static byte[] nonce(byte[] packet, int offset) { return Arrays.copyOfRange(packet, offset + 8, offset + 24); }

    private boolean isHostCandidate(byte[] packet, int offset, int length, InetAddress source) {
        return length == 26 && hasHeader(packet, offset, 2, HOST_CANDIDATE)
                && packet[offset + 6] == 0 && packet[offset + 7] == 0 && !allZero(packet, offset + 8, 16)
                && isInSelectedPrefix(localAddress, prefixLength, source);
    }

    private boolean interfaceIsAssigned() {
        try {
            NetworkInterface network = NetworkInterface.getByInetAddress(localAddress);
            if (network == null || !network.isUp()) return false;
            if (localAddress.isLoopbackAddress() && prefixLength == 32) return network.isLoopback();
            for (java.net.InterfaceAddress item : network.getInterfaceAddresses())
                if (localAddress.equals(item.getAddress()) && item.getNetworkPrefixLength() == prefixLength) return true;
        } catch (IOException ignored) { }
        return localAddress.isLoopbackAddress() && prefixLength == 32;
    }

    private void requireInterface() throws IOException {
        if (!interfaceIsAssigned()) {
            invalidateInterface();
            throw new IOException("所选网络接口已不可用，请重新选择。");
        }
    }

    private synchronized void invalidateInterface() {
        candidates.clear(); requestedCandidates.clear(); searchNonce = null; searchDeadline = 0;
        lastRequestStatus = "interface-lost";
        close();
    }

    private static boolean isAuthProbe(byte[] packet, int offset, int length) {
        return length == AUTH_FRAME_BYTES && hasHeader(packet, offset, AUTH_VERSION, AUTH_PROBE)
                && packet[offset + 6] == 0 && packet[offset + 7] == 0
                && !allZero(packet, offset + 8, 16);
    }

    private void respondAuthenticated(DatagramPacket request) throws IOException {
        byte[] secret = bindingSecret;
        if (secret == null) return;
        byte[] packet = request.getData();
        int offset = request.getOffset();
        byte[] nonce = Arrays.copyOfRange(packet, offset + 8, offset + 24);
        byte[] tag = Arrays.copyOfRange(packet, offset + 26, offset + 34);
        if (!MessageDigest.isEqual(tag, DiscoveryAuth.tag(secret))) return;
        byte[] expected = DiscoveryAuth.probeMac(secret, nonce, tag);
        if (!MessageDigest.isEqual(expected, Arrays.copyOfRange(packet, offset + 34, offset + 50))) return;
        long now = monotonicMillis();
        if (now - lastAuthReplyAt < AUTH_REPLY_INTERVAL_MILLIS) return;
        lastAuthReplyAt = now;
        try { Thread.sleep(RANDOM.nextInt(20)); }
        catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); return; }
        byte[] response = authCandidate(secret, nonce);
        responseSocket.send(new DatagramPacket(response, response.length, request.getAddress(), request.getPort()));
    }

    private byte[] authCandidate(byte[] secret, byte[] nonce) {
        byte[] tag = DiscoveryAuth.tag(secret);
        byte[] port = { (byte) (controlPort >>> 8), (byte) controlPort };
        byte[] packet = new byte[AUTH_FRAME_BYTES];
        System.arraycopy(MAGIC, 0, packet, 0, 4);
        packet[4] = AUTH_VERSION;
        packet[5] = AUTH_CANDIDATE;
        System.arraycopy(nonce, 0, packet, 8, 16);
        packet[24] = port[0];
        packet[25] = port[1];
        System.arraycopy(tag, 0, packet, 26, 8);
        System.arraycopy(DiscoveryAuth.candidateMac(secret, nonce, tag, controlPort), 0, packet, 34, 16);
        return packet;
    }

    private static boolean hasHeader(byte[] packet, int offset, int version, int type) {
        return offset >= 0 && packet.length - offset >= 8 && packet[offset] == MAGIC[0]
                && packet[offset + 1] == MAGIC[1] && packet[offset + 2] == MAGIC[2] && packet[offset + 3] == MAGIC[3]
                && (packet[offset + 4] & 0xff) == version && (packet[offset + 5] & 0xff) == type;
    }

    private static boolean allZero(byte[] packet, int offset, int length) {
        for (int index = offset; index < offset + length; index++) if (packet[index] != 0) return false;
        return true;
    }

    private static boolean allZero(byte[] bytes) {
        for (byte value : bytes) if (value != 0) return false;
        return true;
    }

    private static long monotonicMillis() { return System.nanoTime() / 1_000_000L; }

    static boolean isInSelectedPrefix(InetAddress localAddress, int prefixLength, InetAddress source) {
        if (!(localAddress instanceof Inet4Address) || !(source instanceof Inet4Address)
                || prefixLength < 1 || prefixLength > 32) return false;
        byte[] local = localAddress.getAddress();
        byte[] remote = source.getAddress();
        int fullBytes = prefixLength / 8;
        int remainingBits = prefixLength % 8;
        for (int index = 0; index < fullBytes; index++) {
            if (local[index] != remote[index]) return false;
        }
        if (remainingBits == 0) return true;
        int mask = 0xff << (8 - remainingBits);
        return (local[fullBytes] & mask) == (remote[fullBytes] & mask);
    }

    private static boolean isProbe(byte[] packet, int offset, int length) {
        if (length != FRAME_BYTES || offset < 0 || packet.length - offset < FRAME_BYTES) return false;
        return packet[offset] == MAGIC[0] && packet[offset + 1] == MAGIC[1]
                && packet[offset + 2] == MAGIC[2] && packet[offset + 3] == MAGIC[3]
                && (packet[offset + 4] & 0xff) == VERSION && (packet[offset + 5] & 0xff) == PROBE
                && packet[offset + 6] == 0 && packet[offset + 7] == 0
                && packet[offset + 24] == 0 && packet[offset + 25] == 0;
    }

    private static byte[] response(byte[] probe, int offset, int controlPort) {
        byte[] response = new byte[FRAME_BYTES];
        System.arraycopy(MAGIC, 0, response, 0, MAGIC.length);
        response[4] = VERSION;
        response[5] = CANDIDATE;
        System.arraycopy(probe, offset + 8, response, 8, 16);
        response[24] = (byte) (controlPort >>> 8);
        response[25] = (byte) controlPort;
        return response;
    }

    @Override public void close() {
        socket.close();
        responseSocket.close();
        Thread thread = worker;
        if (thread != null && thread != Thread.currentThread()) {
            try { thread.join(1_000); } catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        }
    }
}
