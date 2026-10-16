package app.anpod.receiver;

import java.io.IOException;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import java.net.SocketException;
import java.net.SocketAddress;
import java.util.Arrays;

/** Explicit per-session UDP socket; it has no discovery, control, or service-start path. */
final class UdpReceiver implements Runnable, AutoCloseable {
    private static final int RECEIVE_BUFFER_BYTES = 256 * 1024;
    private final DatagramSocket socket;
    private final InetSocketAddress expectedPeer;
    private final UdpPacketDecoder decoder;
    private final UdpPcmQueue queue;
    private volatile boolean closed;

    UdpReceiver(UdpSessionContext context, UdpPcmQueue queue) throws IOException {
        this.queue = queue;
        expectedPeer = context.expectedPeer;
        byte[] key = context.copyKey();
        UdpPacketDecoder createdDecoder;
        try { createdDecoder = new UdpPacketDecoder(context.sessionId, key); }
        finally { Arrays.fill(key, (byte) 0); }
        decoder = createdDecoder;
        DatagramSocket candidate = null;
        try {
            candidate = context.takeSocket();
            if (candidate == null) {
                candidate = new DatagramSocket(null);
                candidate.setReuseAddress(false);
                candidate.bind(context.bindAddress);
            }
            candidate.setReceiveBufferSize(RECEIVE_BUFFER_BYTES);
            socket = candidate;
        } catch (IOException | RuntimeException error) {
            if (candidate != null) candidate.close();
            decoder.destroy();
            throw error;
        }
    }

    InetSocketAddress localAddress() {
        return (InetSocketAddress) socket.getLocalSocketAddress();
    }

    boolean isClosed() { return socket.isClosed(); }

    @Override
    public void run() {
        byte[] bytes = new byte[UdpPacketDecoder.MAX_DATAGRAM_BYTES + 1];
        DatagramPacket datagram = new DatagramPacket(bytes, bytes.length);
        try {
            while (!closed) {
                datagram.setLength(bytes.length);
                socket.receive(datagram);
                processDatagram(bytes, datagram.getLength(), datagram.getSocketAddress());
            }
        } catch (SocketException error) {
            if (!closed) queue.fail(new IOException("UDP receive socket failed", error));
        } catch (IOException error) {
            if (!closed) queue.fail(new IOException("UDP receive failed", error));
        } finally {
            if (!closed) queue.stop();
        }
    }

    ProcessResult processDatagram(byte[] bytes, int length, SocketAddress source) {
        if (!expectedPeer.equals(source)) return ProcessResult.WRONG_PEER;
        try {
            UdpPacketDecoder.Packet packet = decoder.open(bytes, length);
            queue.accept(packet, decoder);
            return packet.type == UdpPacketDecoder.FORMAT ? ProcessResult.FORMAT : ProcessResult.AUDIO;
        } catch (UdpPacketDecoder.ProtocolException error) {
            return resultFor(error.reason);
        }
    }

    UdpPcmQueue queue() { return queue; }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        socket.close();
        queue.stop();
        decoder.destroy();
    }

    private static ProcessResult resultFor(int reason) {
        switch (reason) {
            case UdpPacketDecoder.ProtocolException.AUTHENTICATION: return ProcessResult.AUTHENTICATION;
            case UdpPacketDecoder.ProtocolException.REPLAY: return ProcessResult.REPLAY;
            case UdpPacketDecoder.ProtocolException.IDENTITY: return ProcessResult.IDENTITY;
            case UdpPacketDecoder.ProtocolException.FORMAT: return ProcessResult.FORMAT_REJECTED;
            default: return ProcessResult.HEADER_REJECTED;
        }
    }

    enum ProcessResult {
        FORMAT, AUDIO, WRONG_PEER, IDENTITY, AUTHENTICATION, REPLAY, FORMAT_REJECTED, HEADER_REJECTED
    }
}
