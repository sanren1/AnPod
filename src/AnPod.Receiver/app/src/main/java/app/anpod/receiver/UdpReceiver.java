package app.anpod.receiver;

import java.io.IOException;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import java.net.SocketException;
import java.net.SocketAddress;
import com.anpod.server.FloatPcmFrameQueue;

/** Explicit per-session UDP socket; it has no discovery, control, or service-start path. */
final class UdpReceiver implements Runnable, AutoCloseable {
    private static final int RECEIVE_BUFFER_BYTES = 256 * 1024;
    private final DatagramSocket socket;
    private final InetSocketAddress expectedPeer;
    private final UdpPcmQueue queue;
    private final FloatPcmFrameQueue floatQueue;
    private final AnpdAudioIngress ingress;
    private volatile boolean closed;

    UdpReceiver(UdpSessionContext context, UdpPcmQueue queue) throws IOException {
        this(context, queue, null);
    }

    UdpReceiver(UdpSessionContext context, FloatPcmFrameQueue queue) throws IOException {
        this(context, null, queue);
    }

    private UdpReceiver(UdpSessionContext context, UdpPcmQueue queue, FloatPcmFrameQueue floatQueue)
            throws IOException {
        if (context == null || (queue == null) == (floatQueue == null)
                || (context.formatV2 == null && floatQueue != null)
                || (context.formatV2 != null && (context.formatV2.profile == 1) != (queue != null)))
            throw new IllegalArgumentException("UDP receiver queue must match its session format");
        this.queue = queue;
        this.floatQueue = floatQueue;
        ingress = AnpdAudioIngress.udp(context, queue, floatQueue);
        expectedPeer = context.expectedPeer;
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
            ingress.close();
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
            if (!closed) fail(new IOException("UDP receive socket failed", error));
        } catch (IOException error) {
            if (!closed) fail(new IOException("UDP receive failed", error));
        } finally {
            if (!closed) stopQueue();
        }
    }

    ProcessResult processDatagram(byte[] bytes, int length, SocketAddress source) {
        if (!expectedPeer.equals(source)) return ProcessResult.WRONG_PEER;
        return resultFor(ingress.accept(bytes, length));
    }

    UdpPcmQueue queue() { return queue; }
    boolean hasAuthenticatedAudio() { return ingress.hasAuthenticatedAudio(); }
    long lastAuthenticatedAudioAtNanos() { return ingress.lastAuthenticatedAudioAtNanos(); }
    long droppedFrames() { return ingress.droppedFrames(); }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        socket.close();
        ingress.stopQueue();
        ingress.close();
    }

    private void fail(IOException error) {
        ingress.fail(error);
    }

    private void stopQueue() {
        ingress.stopQueue();
    }

    private static ProcessResult resultFor(AnpdAudioIngress.Result result) {
        switch (result) {
            case FORMAT: return ProcessResult.FORMAT;
            case AUDIO: return ProcessResult.AUDIO;
            case IDENTITY: return ProcessResult.IDENTITY;
            case AUTHENTICATION: return ProcessResult.AUTHENTICATION;
            case REPLAY: return ProcessResult.REPLAY;
            case FORMAT_REJECTED: return ProcessResult.FORMAT_REJECTED;
            default: return ProcessResult.HEADER_REJECTED;
        }
    }

    enum ProcessResult {
        FORMAT, AUDIO, WRONG_PEER, IDENTITY, AUTHENTICATION, REPLAY, FORMAT_REJECTED, HEADER_REJECTED
    }
}
