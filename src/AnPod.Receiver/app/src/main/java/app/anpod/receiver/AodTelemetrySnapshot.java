package app.anpod.receiver;

/** Immutable per-session AOD view published to the Compose UI. */
final class AodTelemetrySnapshot {
    enum DisplayStatus { DISCONNECTED, CONNECTED, RECEIVING, RECONNECTING }

    enum Transport { UNKNOWN, USB_DIRECT, USB_ADB, WIFI_UDP, WIFI_ADB }

    final long sessionGeneration;
    final DisplayStatus status;
    final Transport transport;
    final Long recvAudioPayloadBps;
    final Float receiverBufferMs;
    final Float lossPercent;
    final Float lossPercent10s;
    final Float cumulativeLossPercent;
    /** Unknown source frames in the 60-second loss window, or -1 when its range is unavailable. */
    final long lossUnknownFrames;
    final int outputSampleRate;
    final int outputChannels;
    final int underrunCount;
    final String actualOutputEngine;
    final boolean receivedAudioRecently;
    final long sampledAtElapsedNanos;

    AodTelemetrySnapshot(long sessionGeneration, DisplayStatus status, Transport transport,
            Long recvAudioPayloadBps, Float receiverBufferMs, Float lossPercent,
            int outputSampleRate, int outputChannels, int underrunCount, String actualOutputEngine,
            boolean receivedAudioRecently, long sampledAtElapsedNanos) {
        this(sessionGeneration, status, transport, recvAudioPayloadBps, receiverBufferMs,
                lossPercent, null, null, -1, outputSampleRate, outputChannels, underrunCount,
                actualOutputEngine, receivedAudioRecently, sampledAtElapsedNanos);
    }

    AodTelemetrySnapshot(long sessionGeneration, DisplayStatus status, Transport transport,
            Long recvAudioPayloadBps, Float receiverBufferMs, Float lossPercent,
            Float lossPercent10s, Float cumulativeLossPercent, long lossUnknownFrames,
            int outputSampleRate, int outputChannels, int underrunCount, String actualOutputEngine,
            boolean receivedAudioRecently, long sampledAtElapsedNanos) {
        this.sessionGeneration = sessionGeneration;
        this.status = status;
        this.transport = transport;
        this.recvAudioPayloadBps = recvAudioPayloadBps;
        this.receiverBufferMs = receiverBufferMs;
        this.lossPercent = lossPercent;
        this.lossPercent10s = lossPercent10s;
        this.cumulativeLossPercent = cumulativeLossPercent;
        this.lossUnknownFrames = lossUnknownFrames;
        this.outputSampleRate = outputSampleRate;
        this.outputChannels = outputChannels;
        this.underrunCount = underrunCount;
        this.actualOutputEngine = actualOutputEngine;
        this.receivedAudioRecently = receivedAudioRecently;
        this.sampledAtElapsedNanos = sampledAtElapsedNanos;
    }

    boolean hasAudioFormatData() {
        return recvAudioPayloadBps != null;
    }
}
