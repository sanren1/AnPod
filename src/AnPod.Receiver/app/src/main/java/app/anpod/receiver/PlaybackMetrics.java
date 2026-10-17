package app.anpod.receiver;

    final class PlaybackMetrics {
        final int sampleRate, channels, jitterTargetFrames, underrunCount;
        final int requestedFrames, acceptedFrames, effectiveFrames, capacityFrames, startThresholdFrames, preRollFrames;
        final long expiredFrames, queuedFrames, pendingFrames, tcpDroppedFrames;
        final Boolean advancing;
        final boolean udpQueueActive;

        PlaybackMetrics(int sampleRate, int channels, int jitterTargetFrames, long expiredFrames,
                long queuedFrames, long pendingFrames, int underrunCount, long tcpDroppedFrames,
                Boolean advancing, int requestedFrames, int acceptedFrames, int effectiveFrames,
                int capacityFrames, int startThresholdFrames, int preRollFrames, boolean udpQueueActive) {
            this.sampleRate = sampleRate; this.channels = channels; this.jitterTargetFrames = jitterTargetFrames;
            this.expiredFrames = expiredFrames; this.queuedFrames = queuedFrames; this.pendingFrames = pendingFrames;
            this.underrunCount = underrunCount; this.tcpDroppedFrames = tcpDroppedFrames; this.advancing = advancing;
            this.requestedFrames = requestedFrames; this.acceptedFrames = acceptedFrames;
            this.effectiveFrames = effectiveFrames; this.capacityFrames = capacityFrames;
            this.startThresholdFrames = startThresholdFrames; this.preRollFrames = preRollFrames;
            this.udpQueueActive = udpQueueActive;
        }

        static PlaybackMetrics unknown() {
            return new PlaybackMetrics(-1, -1, -1, -1, -1, -1, -1, -1,
                    null, -1, -1, -1, -1, -1, -1, false);
        }
    }
