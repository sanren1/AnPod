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
    final Float udpLossPercent;
    final int outputSampleRate;
    final int outputChannels;
    final int underrunCount;
    final String actualOutputEngine;
    final boolean receivedAudioRecently;
    final long sampledAtElapsedNanos;

    AodTelemetrySnapshot(long sessionGeneration, DisplayStatus status, Transport transport,
            Long recvAudioPayloadBps, Float receiverBufferMs, Float udpLossPercent,
            int outputSampleRate, int outputChannels, int underrunCount, String actualOutputEngine,
            boolean receivedAudioRecently, long sampledAtElapsedNanos) {
        this.sessionGeneration = sessionGeneration;
        this.status = status;
        this.transport = transport;
        this.recvAudioPayloadBps = recvAudioPayloadBps;
        this.receiverBufferMs = receiverBufferMs;
        this.udpLossPercent = udpLossPercent;
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
