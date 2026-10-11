package app.anpod.receiver;

import java.net.InetSocketAddress;
import java.util.Arrays;

/** In-memory input from a future authenticated session coordinator; never built from a discovery packet. */
final class UdpSessionContext {
    final InetSocketAddress bindAddress;
    final InetSocketAddress expectedPeer;
    final long sessionId;
    final String playbackMode;
    private final byte[] key;
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
    }

    synchronized byte[] copyKey() {
        if (destroyed) throw new IllegalStateException("UDP session context was destroyed");
        return key.clone();
    }

    synchronized void destroy() {
        if (destroyed) return;
        destroyed = true;
        Arrays.fill(key, (byte) 0);
    }
}
