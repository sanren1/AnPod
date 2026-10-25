package app.anpod.receiver;

import com.anpod.server.FloatPcmFrameQueue;

/** Authenticated record-stream audio metadata and the one bounded ingress shared with ReceiverService. */
final class AoaSessionContext implements AutoCloseable {
    final long sessionId;
    final UdpPacketDecoder.AudioFormatV2 formatV2;
    final String playbackMode;
    final AodTelemetrySnapshot.Transport transport;
    final AnpdAudioIngress ingress;
    final UdpPcmQueue pcmQueue;
    final FloatPcmFrameQueue floatQueue;
    private boolean closed;

    AoaSessionContext(long sessionId, byte[] audioKey, UdpPacketDecoder.AudioFormatV2 formatV2) {
        this(sessionId, audioKey, formatV2, AodTelemetrySnapshot.Transport.USB_DIRECT);
    }

    AoaSessionContext(long sessionId, byte[] audioKey, UdpPacketDecoder.AudioFormatV2 formatV2,
            AodTelemetrySnapshot.Transport transport) {
        if (formatV2 == null) throw new IllegalArgumentException("record session requires a locked v2 format");
        if (transport != AodTelemetrySnapshot.Transport.USB_DIRECT
                && transport != AodTelemetrySnapshot.Transport.USB_ADB
                && transport != AodTelemetrySnapshot.Transport.WIFI_ADB)
            throw new IllegalArgumentException("record session requires a record transport");
        formatV2.encode();
        this.sessionId = sessionId;
        this.formatV2 = formatV2;
        this.transport = transport;
        playbackMode = formatV2.mode == 1 ? "lowlatency" : "balanced";
        boolean lowLatency = formatV2.mode == 1;
        if (formatV2.profile == 1) {
            pcmQueue = new UdpPcmQueue(formatV2.sampleRate, formatV2.channels, playbackMode);
            floatQueue = null;
        } else {
            pcmQueue = null;
            floatQueue = new FloatPcmFrameQueue(formatV2.sampleRate, lowLatency,
                    UdpPacketDecoder.MAX_PLAINTEXT_BYTES / formatV2.frameBytes);
        }
        ingress = AnpdAudioIngress.record(sessionId, audioKey, formatV2, pcmQueue, floatQueue);
    }

    synchronized void destroy() {
        if (closed) return;
        closed = true;
        stopQueue();
        ingress.close();
    }

    void stopQueue() { ingress.stopQueue(); }

    @Override public void close() { destroy(); }

    synchronized boolean isClosed() { return closed; }
}
