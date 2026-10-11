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
    private final int controlPort;
    private volatile Thread worker;

    static P5cDiscovery open(InetAddress localAddress, int controlPort) throws IOException {
        DatagramSocket socket = new DatagramSocket(null);
        try {
            socket.setReuseAddress(false);
            socket.setBroadcast(true);
            socket.setSoTimeout(250);
            socket.bind(new InetSocketAddress(localAddress, PORT));
            return new P5cDiscovery(socket, controlPort);
        } catch (IOException | RuntimeException error) {
            socket.close();
            throw error;
        }
    }

    private P5cDiscovery(DatagramSocket socket, int controlPort) {
        this.socket = socket;
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
                        || (!source.isLoopbackAddress() && !source.isSiteLocalAddress())) continue;
                if (!isProbe(request.getData(), request.getOffset(), request.getLength())) continue;
                byte[] response = response(request.getData(), request.getOffset(), controlPort);
                socket.send(new DatagramPacket(response, response.length, request.getAddress(), request.getPort()));
            } catch (SocketTimeoutException ignored) {
                // Recheck closure without busy polling.
            } catch (IOException error) {
                if (!socket.isClosed()) close();
            }
        }
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
        Thread thread = worker;
        if (thread != null && thread != Thread.currentThread()) {
            try { thread.join(1_000); } catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        }
    }
}
