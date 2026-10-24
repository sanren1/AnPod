package app.anpod.receiver;

import java.net.InetSocketAddress;
import java.net.DatagramSocket;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.util.Arrays;

/** In-memory input from a future authenticated session coordinator; never built from a discovery packet. */
final class UdpSessionContext {
    final InetSocketAddress bindAddress;
    final InetSocketAddress expectedPeer;
    final long sessionId;
    final String playbackMode;
    final PairingProtocol.AudioParameters parameters;
    final UdpPacketDecoder.AudioFormatV2 formatV2;
    private final byte[] key;
    private DatagramSocket socket;
    private boolean destroyed;

    UdpSessionContext(InetSocketAddress bindAddress, InetSocketAddress expectedPeer,
            long sessionId, byte[] key, String playbackMode) {
        if (bindAddress == null || bindAddress.getAddress() == null || expectedPeer == null
                || expectedPeer.getAddress() == null || bindAddress.getPort() < 0
                || bindAddress.getPort() > 65535 || expectedPeer.getPort() < 1
                || expectedPeer.getPort() > 65535 || key == null || key.length != 32) {
            throw new IllegalArgumentException("Invalid in-memory UDP session context");
        }
        this.bindAddress = bindAddress;
        this.expectedPeer = expectedPeer;
        this.sessionId = sessionId;
        this.key = key.clone();
        this.playbackMode = playbackMode;
        this.parameters = null;
        this.formatV2 = null;
    }

    UdpSessionContext(DatagramSocket socket, InetSocketAddress expectedPeer,
            long sessionId, byte[] key, PairingProtocol.AudioParameters parameters) {
        if (socket == null || socket.isClosed() || !socket.isBound()
                || !(socket.getLocalSocketAddress() instanceof InetSocketAddress)
                || !isPrivateIpv4(socket.getLocalAddress())
                || expectedPeer == null || !isPrivateIpv4(expectedPeer.getAddress())
                || expectedPeer.getPort() < 1 || expectedPeer.getPort() > 65535
                || key == null || key.length != 32 || parameters == null)
            throw new IllegalArgumentException("Invalid authenticated UDP session context");
        this.socket = socket;
        this.bindAddress = (InetSocketAddress) socket.getLocalSocketAddress();
        this.expectedPeer = expectedPeer;
        this.sessionId = sessionId;
        this.key = key.clone();
        this.parameters = parameters;
        this.formatV2 = null;
        this.playbackMode = parameters.mode == 1 ? "lowlatency" : "balanced";
    }

    UdpSessionContext(DatagramSocket socket, InetSocketAddress expectedPeer,
            long sessionId, byte[] key, UdpPacketDecoder.AudioFormatV2 formatV2) {
        if (socket == null || socket.isClosed() || !socket.isBound()
                || !(socket.getLocalSocketAddress() instanceof InetSocketAddress)
                || !isPrivateIpv4(socket.getLocalAddress())
                || expectedPeer == null || !isPrivateIpv4(expectedPeer.getAddress())
                || expectedPeer.getPort() < 1 || expectedPeer.getPort() > 65535
                || key == null || key.length != 32 || formatV2 == null)
            throw new IllegalArgumentException("Invalid authenticated UDP v2 session context");
        formatV2.encode();
        this.socket = socket;
        this.bindAddress = (InetSocketAddress) socket.getLocalSocketAddress();
        this.expectedPeer = expectedPeer;
        this.sessionId = sessionId;
        this.key = key.clone();
        this.parameters = null;
        this.formatV2 = formatV2;
        this.playbackMode = formatV2.mode == 1 ? "lowlatency" : "balanced";
    }

    static boolean isPrivateIpv4(InetAddress address) {
        return address instanceof Inet4Address
                && (address.isLoopbackAddress() || address.isSiteLocalAddress());
    }

    synchronized byte[] copyKey() {
        if (destroyed) throw new IllegalStateException("UDP session context was destroyed");
        return key.clone();
    }

    synchronized DatagramSocket takeSocket() {
        if (destroyed) throw new IllegalStateException("UDP session context was destroyed");
        DatagramSocket result = socket;
        socket = null;
        return result;
    }

    synchronized void destroy() {
        if (destroyed) return;
        destroyed = true;
        Arrays.fill(key, (byte) 0);
        if (socket != null) socket.close();
        socket = null;
    }
}
