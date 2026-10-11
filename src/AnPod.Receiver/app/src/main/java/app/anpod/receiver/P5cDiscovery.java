package app.anpod.receiver;

import java.io.IOException;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.SocketTimeoutException;

/** Explicit, stateless candidate discovery for an already running P5-C endpoint. */
final class P5cDiscovery implements AutoCloseable {
    static final int PORT = 49_772;
    private static final byte[] MAGIC = { 'A', 'N', 'P', 'D' };
    private static final int VERSION = 1;
    private static final int PROBE = 1;
    private static final int CANDIDATE = 2;
    private static final int FRAME_BYTES = 26;

    private final DatagramSocket socket;
    private final DatagramSocket responseSocket;
    private final InetAddress localAddress;
    private final int prefixLength;
    private final int controlPort;
    private volatile Thread worker;

    static P5cDiscovery open(InetAddress localAddress, int prefixLength, int controlPort) throws IOException {
        if (!(localAddress instanceof Inet4Address) || prefixLength < 1 || prefixLength > 32
                || (!localAddress.isLoopbackAddress() && !localAddress.isSiteLocalAddress()))
            throw new IOException("Discovery requires an explicit private IPv4 address and prefix");
        if (localAddress.isLoopbackAddress() && prefixLength != 32)
            throw new IOException("Loopback discovery test interface must use prefix length 32");
        DatagramSocket socket = new DatagramSocket(null);
        DatagramSocket responseSocket = null;
        try {
            responseSocket = new DatagramSocket(null);
            socket.setReuseAddress(false);
            socket.setBroadcast(true);
            socket.setSoTimeout(250);
            socket.bind(new InetSocketAddress(PORT));
            responseSocket.setReuseAddress(false);
            responseSocket.bind(new InetSocketAddress(localAddress, 0));
            return new P5cDiscovery(socket, responseSocket, localAddress, prefixLength, controlPort);
        } catch (IOException | RuntimeException error) {
            socket.close();
            if (responseSocket != null) responseSocket.close();
            throw error;
        }
    }

    private P5cDiscovery(DatagramSocket socket, DatagramSocket responseSocket,
            InetAddress localAddress, int prefixLength, int controlPort) {
        this.socket = socket;
        this.responseSocket = responseSocket;
        this.localAddress = localAddress;
        this.prefixLength = prefixLength;
        this.controlPort = controlPort;
    }

    synchronized void start() {
        if (worker != null) throw new IllegalStateException("P5-C discovery already started");
        worker = new Thread(this::serve, "AnPod P5-C discovery");
        worker.setDaemon(true);
        worker.start();
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
                if (!isProbe(request.getData(), request.getOffset(), request.getLength())) continue;
                byte[] response = response(request.getData(), request.getOffset(), controlPort);
                responseSocket.send(new DatagramPacket(response, response.length, request.getAddress(), request.getPort()));
            } catch (SocketTimeoutException ignored) {
                // Recheck closure without busy polling.
            } catch (IOException error) {
                if (!socket.isClosed()) close();
            }
        }
    }

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
