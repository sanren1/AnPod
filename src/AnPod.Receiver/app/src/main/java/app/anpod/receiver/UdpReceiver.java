package app.anpod.receiver;

import java.io.IOException;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import java.net.SocketException;
import java.net.SocketAddress;
import java.util.Arrays;
import com.anpod.server.FloatPcmFrameQueue;
import com.anpod.server.Pcm24PackedCodec;

/** Explicit per-session UDP socket; it has no discovery, control, or service-start path. */
final class UdpReceiver implements Runnable, AutoCloseable {
    private static final int RECEIVE_BUFFER_BYTES = 256 * 1024;
    private final DatagramSocket socket;
    private final InetSocketAddress expectedPeer;
    private final UdpPacketDecoder decoder;
    private final UdpPcmQueue queue;
    private final FloatPcmFrameQueue floatQueue;
    private final UdpPacketDecoder.AudioFormatV2 formatV2;
    private volatile boolean hasAuthenticatedAudio;
    private volatile long lastAuthenticatedAudioAtNanos;
    private volatile long droppedFloatFrames;
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
                || (floatQueue != null && context.formatV2 == null)
                || (queue != null && context.formatV2 != null))
            throw new IllegalArgumentException("UDP receiver queue must match its session format");
        this.queue = queue;
        this.floatQueue = floatQueue;
        formatV2 = context.formatV2;
        expectedPeer = context.expectedPeer;
        byte[] key = context.copyKey();
        UdpPacketDecoder createdDecoder;
        try {
            createdDecoder = floatQueue != null
                    ? UdpPacketDecoder.newV2(context.sessionId, key, context.formatV2)
                    : new UdpPacketDecoder(context.sessionId, key);
        }
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
            if (!closed) fail(new IOException("UDP receive socket failed", error));
        } catch (IOException error) {
            if (!closed) fail(new IOException("UDP receive failed", error));
        } finally {
            if (!closed) stopQueue();
        }
    }

    ProcessResult processDatagram(byte[] bytes, int length, SocketAddress source) {
        if (!expectedPeer.equals(source)) return ProcessResult.WRONG_PEER;
        try {
            UdpPacketDecoder.Packet packet = decoder.open(bytes, length);
            if (floatQueue == null) queue.accept(packet, decoder);
            else acceptFloat(packet);
            return packet.type == UdpPacketDecoder.FORMAT ? ProcessResult.FORMAT : ProcessResult.AUDIO;
        } catch (UdpPacketDecoder.ProtocolException error) {
            return resultFor(error.reason);
        }
    }

    UdpPcmQueue queue() { return queue; }
    boolean hasAuthenticatedAudio() { return floatQueue == null ? queue.hasAuthenticatedAudio() : hasAuthenticatedAudio; }
    long lastAuthenticatedAudioAtNanos() {
        return floatQueue == null ? queue.lastAuthenticatedAudioAtNanos() : lastAuthenticatedAudioAtNanos;
    }
    long droppedFrames() { return floatQueue == null ? queue.droppedFrames() : droppedFloatFrames; }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        socket.close();
        stopQueue();
        decoder.destroy();
    }

    private void acceptFloat(UdpPacketDecoder.Packet packet) throws UdpPacketDecoder.ProtocolException {
        if (packet.type == UdpPacketDecoder.FORMAT) {
            decoder.commit(packet.sequence);
            return;
        }
        if (packet.type != UdpPacketDecoder.AUDIO || packet.sequence == 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
        int frameCount = packet.payload.length / formatV2.frameBytes;
        long frameEnd = packet.sampleFrameIndex + frameCount;
        if (Long.compareUnsigned(frameEnd, packet.sampleFrameIndex) <= 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
        hasAuthenticatedAudio = true;
        lastAuthenticatedAudioAtNanos = System.nanoTime();
        float[] samples = new float[frameCount * formatV2.channels];
        Pcm24PackedCodec.decodeToFloat(packet.payload, 0, samples.length, samples, 0);
        decoder.commit(packet.sequence);
        if (!floatQueue.offer(packet.sampleFrameIndex, samples, frameCount)) droppedFloatFrames += frameCount;
    }

    private void fail(IOException error) {
        if (queue != null) queue.fail(error);
        else floatQueue.fail(error);
    }

    private void stopQueue() {
        if (queue != null) queue.stop();
        else floatQueue.stop();
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
