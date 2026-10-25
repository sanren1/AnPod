package app.anpod.receiver;

import com.anpod.server.FloatPcmFrameQueue;
import com.anpod.server.Float32LeCodec;
import com.anpod.server.Pcm24PackedCodec;

import java.io.IOException;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicLong;

/** Shared authenticated ANPD decoder and bounded queue ingress for UDP and AOA transports. */
final class AnpdAudioIngress implements AutoCloseable {
    enum Result { FORMAT, AUDIO, IDENTITY, AUTHENTICATION, REPLAY, FORMAT_REJECTED, HEADER_REJECTED }

    private final UdpPacketDecoder decoder;
    private final UdpPacketDecoder.AudioFormatV2 formatV2;
    private final UdpPcmQueue pcmQueue;
    private final FloatPcmFrameQueue floatQueue;
    private final boolean requireFormatBeforeAudio;
    private boolean formatSeen;
    private volatile boolean hasAuthenticatedAudio;
    private volatile long lastAuthenticatedAudioAtNanos;
    private volatile long droppedFloatFrames;
    private final AtomicLong authenticatedAudioPayloadBytes = new AtomicLong();
    private final AtomicLong authenticatedAudioFrames = new AtomicLong();

    private AnpdAudioIngress(long sessionId, byte[] key, UdpPacketDecoder.AudioFormatV2 formatV2,
            UdpPcmQueue pcmQueue, FloatPcmFrameQueue floatQueue, boolean requireFormatBeforeAudio) {
        if (key == null || (pcmQueue == null) == (floatQueue == null)
                || (formatV2 == null && floatQueue != null)
                || (formatV2 != null && (formatV2.profile == 1) != (pcmQueue != null)))
            throw new IllegalArgumentException("ANPD ingress queue does not match its locked format");
        this.formatV2 = formatV2;
        this.pcmQueue = pcmQueue;
        this.floatQueue = floatQueue;
        this.requireFormatBeforeAudio = requireFormatBeforeAudio;
        decoder = formatV2 == null ? new UdpPacketDecoder(sessionId, key)
                : UdpPacketDecoder.newV2(sessionId, key, formatV2);
    }

    static AnpdAudioIngress udp(UdpSessionContext context, UdpPcmQueue pcmQueue,
            FloatPcmFrameQueue floatQueue) {
        if (context == null) throw new NullPointerException("context");
        byte[] key = context.copyKey();
        try { return new AnpdAudioIngress(context.sessionId, key, context.formatV2, pcmQueue, floatQueue, false); }
        finally { Arrays.fill(key, (byte) 0); }
    }

    static AnpdAudioIngress record(long sessionId, byte[] key, UdpPacketDecoder.AudioFormatV2 format,
            UdpPcmQueue pcmQueue, FloatPcmFrameQueue floatQueue) {
        if (format == null) throw new IllegalArgumentException("record stream requires a locked v2 format");
        return new AnpdAudioIngress(sessionId, key, format, pcmQueue, floatQueue, true);
    }

    static AnpdAudioIngress aoa(long sessionId, byte[] key, UdpPacketDecoder.AudioFormatV2 format,
            UdpPcmQueue pcmQueue, FloatPcmFrameQueue floatQueue) {
        return record(sessionId, key, format, pcmQueue, floatQueue);
    }

    Result accept(byte[] bytes, int length) {
        try {
            UdpPacketDecoder.Packet packet = decoder.open(bytes, length);
            if (requireFormatBeforeAudio && packet.type == UdpPacketDecoder.AUDIO && !formatSeen)
                throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
            if (formatV2 == null) {
                pcmQueue.accept(packet, decoder);
            } else if (pcmQueue != null) {
                pcmQueue.acceptV2Pcm16(packet, decoder);
            } else {
                acceptFloat(packet);
            }
            if (packet.type == UdpPacketDecoder.FORMAT) formatSeen = true;
            else authenticatedAudioPayloadBytes.addAndGet(packet.payload.length);
            return packet.type == UdpPacketDecoder.FORMAT ? Result.FORMAT : Result.AUDIO;
        } catch (UdpPacketDecoder.ProtocolException error) {
            return resultFor(error.reason);
        }
    }

    UdpPcmQueue pcmQueue() { return pcmQueue; }
    FloatPcmFrameQueue floatQueue() { return floatQueue; }
    boolean hasAuthenticatedAudio() { return pcmQueue == null ? hasAuthenticatedAudio : pcmQueue.hasAuthenticatedAudio(); }
    long lastAuthenticatedAudioAtNanos() {
        return pcmQueue == null ? lastAuthenticatedAudioAtNanos : pcmQueue.lastAuthenticatedAudioAtNanos();
    }
    long droppedFrames() { return pcmQueue == null ? droppedFloatFrames : pcmQueue.droppedFrames(); }
    long authenticatedAudioPayloadBytes() { return authenticatedAudioPayloadBytes.get(); }
    long authenticatedAudioFrames() { return authenticatedAudioFrames.get(); }

    void fail(IOException error) {
        if (pcmQueue != null) pcmQueue.fail(error); else floatQueue.fail(error);
    }

    void stopQueue() {
        if (pcmQueue != null) pcmQueue.stop(); else floatQueue.stop();
    }

    @Override public void close() { decoder.destroy(); }

    private void acceptFloat(UdpPacketDecoder.Packet packet) throws UdpPacketDecoder.ProtocolException {
        if (packet.type == UdpPacketDecoder.FORMAT) {
            if (packet.sequence != 0) throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
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
        try {
            if (formatV2.sampleFormat == 3)
                Float32LeCodec.decode(packet.payload, 0, samples.length, samples, 0);
            else
                Pcm24PackedCodec.decodeToFloat(packet.payload, 0, samples.length, samples, 0);
        } catch (IllegalArgumentException invalidSample) {
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT, invalidSample);
        }
        decoder.commit(packet.sequence);
        authenticatedAudioFrames.addAndGet(frameCount);
        if (!floatQueue.offer(packet.sampleFrameIndex, samples, frameCount)) droppedFloatFrames += frameCount;
    }

    private static Result resultFor(int reason) {
        switch (reason) {
            case UdpPacketDecoder.ProtocolException.AUTHENTICATION: return Result.AUTHENTICATION;
            case UdpPacketDecoder.ProtocolException.REPLAY: return Result.REPLAY;
            case UdpPacketDecoder.ProtocolException.IDENTITY: return Result.IDENTITY;
            case UdpPacketDecoder.ProtocolException.FORMAT: return Result.FORMAT_REJECTED;
            default: return Result.HEADER_REJECTED;
        }
    }
}
