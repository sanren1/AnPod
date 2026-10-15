package app.anpod.receiver;

import com.anpod.server.Pcm16FrameStream;

import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.util.ArrayList;

/** Short-lock packet queue and byte-stream adapter for the existing AudioTrack worker. */
final class UdpPcmQueue {
    static final int CAPACITY_PACKETS = 8;
    private static final int LOW_LATENCY_INITIAL_MS = 5;
    private static final int BALANCED_INITIAL_MS = 15;
    private static final int LOW_LATENCY_MAX_MS = 20;
    private static final int BALANCED_MAX_MS = 40;
    private static final int STEP_MS = 5;
    private static final int FADE_MS = 3;
    private static final long STABLE_DECAY_NS = 1_000_000_000L;

    private final ArrayList<AudioPacket> packets = new ArrayList<>(CAPACITY_PACKETS);
    private final PairingProtocol.AudioParameters expectedParameters;
    private final boolean lowLatency;
    private UdpPacketDecoder.Format format;
    private volatile boolean hasAuthenticatedAudio;
    private volatile long lastAuthenticatedAudioAtNanos;
    private boolean paused;
    private boolean stopped;
    private IOException failure;
    private boolean hasPlayoutFrame;
    private long playoutFrame;
    private long payloadGeneration;
    private long droppedFrames;
    private long expiredFrames;
    private long queuedFrames;
    private QueueInputStream activePayloadStream;
    private long trackUnderruns = -1;
    private long stableSinceNanos;
    private int initialTargetFrames;
    private int targetFrames;
    private int maxTargetFrames;
    private int maxQueuedFrames;
    private long maxPacketAgeNanos;
    private long waitBudgetNanos;
    private boolean rebuffering = true;
    private long rebufferDeadlineNanos;
    private boolean gapWaitActive;
    private long gapWaitFrame;
    private long gapDeadlineNanos;
    private boolean fadeNext;

    UdpPcmQueue() { this(null, "balanced"); }

    UdpPcmQueue(PairingProtocol.AudioParameters expectedParameters) {
        this(expectedParameters, expectedParameters != null && expectedParameters.mode == 1
                ? "lowlatency" : "balanced");
    }

    UdpPcmQueue(PairingProtocol.AudioParameters expectedParameters, String playbackMode) {
        this.expectedParameters = expectedParameters;
        lowLatency = "lowlatency".equalsIgnoreCase(playbackMode);
        if (expectedParameters != null) {
            format = new UdpPacketDecoder.Format(expectedParameters.sampleRate, expectedParameters.channels);
            updateBudget(expectedParameters.sampleRate);
        }
    }

    synchronized void configureAudioTrack(int preRollFrames) {
        if (format == null) return;
        // AudioTrack pre-roll is a separate device buffer, not part of this network jitter budget.
        updateBudget(format.sampleRate);
        targetFrames = initialTargetFrames;
        updateWaitBudget();
        stableSinceNanos = System.nanoTime();
        rebufferDeadlineNanos = packets.isEmpty() ? 0 : packets.get(0).enqueuedAtNanos + waitBudgetNanos;
        notifyAll();
    }

    synchronized void reportTrackUnderruns(int currentCount) {
        if (currentCount < 0) return;
        long now = System.nanoTime();
        if (trackUnderruns < 0) {
            trackUnderruns = currentCount;
            stableSinceNanos = now;
            return;
        }
        long increase = Math.max(0, (long) currentCount - trackUnderruns);
        trackUnderruns = currentCount;
        if (increase > 0) {
            int stepFrames = framesForMillis(STEP_MS);
            targetFrames = (int) Math.min(maxTargetFrames,
                    targetFrames + Math.min(increase, CAPACITY_PACKETS) * stepFrames);
            updateWaitBudget();
            stableSinceNanos = now;
            notifyAll();
        }
    }

    synchronized void accept(UdpPacketDecoder.Packet packet, UdpPacketDecoder decoder)
            throws UdpPacketDecoder.ProtocolException {
        acceptAt(packet, decoder, System.nanoTime());
    }

    synchronized void acceptAt(UdpPacketDecoder.Packet packet, UdpPacketDecoder decoder, long now)
            throws UdpPacketDecoder.ProtocolException {
        if (stopped) throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.AUDIO);
        if (packet.type == UdpPacketDecoder.FORMAT) {
            if (packet.sequence != 0)
                throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
            UdpPacketDecoder.Format acceptedFormat = UdpPacketDecoder.decodeFormat(packet);
            if (expectedParameters != null && (acceptedFormat.sampleRate != expectedParameters.sampleRate
                    || acceptedFormat.channels != expectedParameters.channels))
                throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
            if (format != null && (acceptedFormat.sampleRate != format.sampleRate
                    || acceptedFormat.channels != format.channels))
                throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
            decoder.commit(packet.sequence);
            if (format != null) return;
            format = acceptedFormat;
            updateBudget(format.sampleRate);
            stableSinceNanos = System.nanoTime();
            notifyAll();
            return;
        }

        if (packet.type != UdpPacketDecoder.AUDIO || packet.sequence == 0 || format == null || packet.payload.length == 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
        int frameBytes = format.channels * 2;
        if (packet.payload.length % frameBytes != 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
        int frameCount = packet.payload.length / frameBytes;
        long frameEnd = packet.sampleFrameIndex + frameCount;
        if (Long.compareUnsigned(frameEnd, packet.sampleFrameIndex) <= 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);

        decoder.commit(packet.sequence);
        lastAuthenticatedAudioAtNanos = now;
        hasAuthenticatedAudio = true;
        if (paused) {
            droppedFrames += frameCount;
            advancePlayoutFrame(frameEnd);
            return;
        }
        if (hasPlayoutFrame && Long.compareUnsigned(packet.sampleFrameIndex, playoutFrame) < 0) {
            droppedFrames += frameCount;
            if (Long.compareUnsigned(frameEnd, playoutFrame) > 0) {
                advancePlayoutFrame(frameEnd);
                markDiscontinuity();
            }
            return;
        }

        expireStalePackets(now);
        discardBehindWatermark();
        AudioPacket audio = new AudioPacket(packet.sequence, packet.sampleFrameIndex, frameCount,
                packet.payload, now);
        int insertAt = 0;
        while (insertAt < packets.size()
                && Long.compareUnsigned(packets.get(insertAt).frameIndex, audio.frameIndex) < 0) insertAt++;
        if (insertAt < packets.size()
                && packets.get(insertAt).frameIndex == audio.frameIndex) {
            droppedFrames += frameCount;
            return;
        }
        int evictions = 0;
        long remainingFrames = queuedFrames;
        int remainingPackets = packets.size();
        boolean predictedWatermarkSet = hasPlayoutFrame;
        long predictedWatermark = playoutFrame;
        boolean needsEviction = packets.size() >= CAPACITY_PACKETS
                || queuedFrames + frameCount > maxQueuedFrames;
        if (needsEviction && insertAt == 0 && !packets.isEmpty()) {
            droppedFrames += frameCount;
            expiredFrames += frameCount;
            return;
        }
        while (remainingPackets > 0 && (remainingPackets >= CAPACITY_PACKETS
                || remainingFrames + frameCount > maxQueuedFrames)) {
            AudioPacket stale = packets.get(evictions++);
            remainingFrames -= stale.frameCount;
            remainingPackets--;
            long end = stale.frameIndex + stale.frameCount;
            if (!predictedWatermarkSet || Long.compareUnsigned(end, predictedWatermark) > 0) {
                predictedWatermark = end;
                predictedWatermarkSet = true;
            }
        }
        if (frameCount > maxQueuedFrames || remainingFrames + frameCount > maxQueuedFrames
                || (evictions > 0 && predictedWatermarkSet
                        && Long.compareUnsigned(audio.frameIndex, predictedWatermark) < 0)) {
            droppedFrames += frameCount;
            expiredFrames += frameCount;
            return;
        }
        int insertionAfterEvictions = Math.max(0, insertAt - evictions);
        int previousIndex = evictions + insertionAfterEvictions - 1;
        int nextIndex = evictions + insertionAfterEvictions;
        if ((previousIndex >= evictions && Long.compareUnsigned(audio.frameIndex,
                        packets.get(previousIndex).frameIndex + packets.get(previousIndex).frameCount) < 0)
                || (nextIndex < packets.size() && Long.compareUnsigned(audio.frameIndex + frameCount,
                        packets.get(nextIndex).frameIndex) > 0)) {
            droppedFrames += frameCount;
            return;
        }
        for (int i = 0; i < evictions; i++) {
            AudioPacket stale = packets.remove(0);
            queuedFrames -= stale.frameCount;
            dropQueuedPacket(stale, true);
        }
        discardBehindWatermark();
        insertAt = 0;
        while (insertAt < packets.size()
                && Long.compareUnsigned(packets.get(insertAt).frameIndex, audio.frameIndex) < 0) insertAt++;
        if ((hasPlayoutFrame && Long.compareUnsigned(audio.frameIndex, playoutFrame) < 0)
                || queuedFrames + frameCount > maxQueuedFrames) {
            droppedFrames += frameCount;
            expiredFrames += frameCount;
            long end = audio.frameIndex + frameCount;
            boolean future = !hasPlayoutFrame || Long.compareUnsigned(end, playoutFrame) > 0;
            advancePlayoutFrame(end);
            if (future) markDiscontinuity();
            return;
        }
        boolean wasEmpty = packets.isEmpty();
        packets.add(insertAt, audio);
        queuedFrames += frameCount;
        if (rebuffering && wasEmpty) rebufferDeadlineNanos = now + waitBudgetNanos;
        notifyAll();
    }

    synchronized UdpPacketDecoder.Format awaitFormat() throws IOException {
        while (format == null && !stopped) waitForChange();
        if (format == null && failure != null) throw failure;
        return format;
    }

    synchronized AudioPacket take() throws IOException {
        discardBehindWatermark();
        while ((packets.isEmpty() || paused) && !stopped) {
            waitForChange();
            discardBehindWatermark();
        }
        if (packets.isEmpty() && failure != null) throw failure;
        if (stopped) return null;
        AudioPacket packet = packets.remove(0);
        queuedFrames -= packet.frameCount;
        playoutFrame = packet.frameIndex + packet.frameCount;
        hasPlayoutFrame = true;
        discardBehindWatermark();
        return packet;
    }

    synchronized void setPaused(boolean value) {
        if (paused == value) return;
        paused = value;
        payloadGeneration++;
        gapWaitActive = false;
        rebuffering = true;
        rebufferDeadlineNanos = 0;
        if (paused) {
            for (AudioPacket packet : packets) {
                droppedFrames += packet.frameCount;
                advancePlayoutFrame(packet.frameIndex + packet.frameCount);
            }
            packets.clear();
            queuedFrames = 0;
            if (activePayloadStream != null && activePayloadStream.currentPayload != null) {
                droppedFrames += (activePayloadStream.currentPayload.length
                        - activePayloadStream.payloadOffset) / (format.channels * 2);
                activePayloadStream.currentPayload = null;
                activePayloadStream.payloadOffset = 0;
                activePayloadStream.enqueuedAtNanos = 0;
            }
            activePayloadStream = null;
        } else {
            fadeNext = true;
        }
        notifyAll();
    }

    private void updateBudget(int sampleRate) {
        if (sampleRate <= 0) return;
        int initialMs = lowLatency ? LOW_LATENCY_INITIAL_MS : BALANCED_INITIAL_MS;
        int maxMs = lowLatency ? LOW_LATENCY_MAX_MS : BALANCED_MAX_MS;
        initialTargetFrames = Math.max(1, sampleRate * initialMs / 1000);
        maxTargetFrames = Math.max(initialTargetFrames, sampleRate * maxMs / 1000);
        targetFrames = Math.min(Math.max(targetFrames, initialTargetFrames), maxTargetFrames);
        maxQueuedFrames = maxTargetFrames;
        maxPacketAgeNanos = maxMs * 1_000_000L;
        updateWaitBudget();
    }

    private void updateWaitBudget() {
        int sampleRate = format != null ? format.sampleRate
                : expectedParameters != null ? expectedParameters.sampleRate : 0;
        waitBudgetNanos = sampleRate <= 0 ? 0 : (long) targetFrames * 1_000_000_000L / sampleRate;
    }

    private int framesForMillis(int millis) {
        int sampleRate = format != null ? format.sampleRate
                : expectedParameters != null ? expectedParameters.sampleRate : 0;
        return sampleRate <= 0 ? 1 : Math.max(1, sampleRate * millis / 1000);
    }

    synchronized void decayTarget(long now) {
        if (targetFrames <= initialTargetFrames || now - stableSinceNanos < STABLE_DECAY_NS) return;
        targetFrames = Math.max(initialTargetFrames, targetFrames - framesForMillis(STEP_MS));
        updateWaitBudget();
        stableSinceNanos = now;
    }

    synchronized void expireStalePackets(long nowNanos) {
        boolean changed = false;
        for (int index = packets.size() - 1; index >= 0; index--) {
            AudioPacket packet = packets.get(index);
            if (nowNanos - packet.enqueuedAtNanos > maxPacketAgeNanos) {
                packets.remove(index);
                queuedFrames -= packet.frameCount;
                dropQueuedPacket(packet, true);
                changed = true;
            }
        }
        if (changed) {
            discardBehindWatermark();
            rebuffering = true;
            rebufferDeadlineNanos = packets.isEmpty() ? 0 : System.nanoTime() + waitBudgetNanos;
        }
    }

    private void discardBehindWatermark() {
        while (hasPlayoutFrame && !packets.isEmpty()
                && Long.compareUnsigned(packets.get(0).frameIndex, playoutFrame) < 0) {
            AudioPacket stale = packets.remove(0);
            queuedFrames -= stale.frameCount;
            droppedFrames += stale.frameCount;
            expiredFrames += stale.frameCount;
            long end = stale.frameIndex + stale.frameCount;
            if (Long.compareUnsigned(end, playoutFrame) > 0) {
                advancePlayoutFrame(end);
                markDiscontinuity();
            }
        }
    }

    private void dropQueuedPacket(AudioPacket packet, boolean expired) {
        droppedFrames += packet.frameCount;
        if (expired) expiredFrames += packet.frameCount;
        long end = packet.frameIndex + packet.frameCount;
        boolean future = !hasPlayoutFrame || Long.compareUnsigned(end, playoutFrame) > 0;
        advancePlayoutFrame(end);
        if (future) markDiscontinuity();
    }

    private void advancePlayoutFrame(long frame) {
        if (!hasPlayoutFrame || Long.compareUnsigned(frame, playoutFrame) > 0) {
            playoutFrame = frame;
            hasPlayoutFrame = true;
        }
    }

    private void markDiscontinuity() {
        payloadGeneration++;
        fadeNext = true;
        gapWaitActive = false;
    }

    private synchronized int readPayload(QueueInputStream stream, byte[] target, int offset, int length)
            throws IOException {
        while (true) {
            if (stream.payloadGeneration != payloadGeneration) {
                if (stream.currentPayload != null) {
                    int frameBytes = format.channels * 2;
                    droppedFrames += (stream.currentPayload.length - stream.payloadOffset) / frameBytes;
                    stream.currentPayload = null;
                    stream.payloadOffset = 0;
                    stream.enqueuedAtNanos = 0;
                    if (activePayloadStream == stream) {
                        activePayloadStream = null;
                    }
                }
                stream.payloadGeneration = payloadGeneration;
            }
            if (failure != null) throw failure;
            if (stopped) return -1;
            if (paused) {
                waitForChange();
                continue;
            }

            long now = System.nanoTime();
            decayTarget(now);
            if (stream.currentPayload != null) {
                if (expireCurrentPayload(stream, now)) {
                    stream.payloadGeneration = payloadGeneration;
                    continue;
                }
                int count = Math.min(length, stream.currentPayload.length - stream.payloadOffset);
                System.arraycopy(stream.currentPayload, stream.payloadOffset, target, offset, count);
                stream.payloadOffset += count;
                if (stream.payloadOffset == stream.currentPayload.length) {
                    stream.currentPayload = null;
                    stream.payloadOffset = 0;
                    stream.enqueuedAtNanos = 0;
                    if (activePayloadStream == stream) {
                        activePayloadStream = null;
                    }
                }
                return count;
            }

            expireStalePackets(now);
            if (packets.isEmpty()) {
                rebuffering = true;
                rebufferDeadlineNanos = 0;
                waitForChange();
                continue;
            }

            long packetWaitDeadline = rebufferDeadlineNanos;
            if (rebuffering && targetFrames > 0 && queuedFrames < targetFrames) {
                if (rebufferDeadlineNanos == 0)
                    rebufferDeadlineNanos = packets.get(0).enqueuedAtNanos + waitBudgetNanos;
                long remaining = rebufferDeadlineNanos - now;
                if (remaining > 0) {
                    waitForChange(remaining);
                    continue;
                }
            }
            packetWaitDeadline = rebufferDeadlineNanos;
            rebuffering = false;
            rebufferDeadlineNanos = 0;

            AudioPacket next = packets.get(0);
            if (hasPlayoutFrame && Long.compareUnsigned(next.frameIndex, playoutFrame) > 0) {
                if (!gapWaitActive || gapWaitFrame != playoutFrame) {
                    gapWaitActive = true;
                    gapWaitFrame = playoutFrame;
                    gapDeadlineNanos = packetWaitDeadline > 0
                            ? packetWaitDeadline : next.enqueuedAtNanos + waitBudgetNanos;
                }
                long remaining = gapDeadlineNanos - now;
                if (remaining > 0) {
                    waitForChange(remaining);
                    continue;
                }
                markDiscontinuity();
            }

            next = packets.remove(0);
            queuedFrames -= next.frameCount;
            if (hasPlayoutFrame && Long.compareUnsigned(next.frameIndex, playoutFrame) > 0) fadeNext = true;
            if (fadeNext) {
                fadeIn(next.payload, format.channels, format.sampleRate);
                fadeNext = false;
            }
            playoutFrame = next.frameIndex + next.frameCount;
            hasPlayoutFrame = true;
            discardBehindWatermark();
            gapWaitActive = false;
            stream.currentPayload = next.payload;
            stream.payloadOffset = 0;
            stream.enqueuedAtNanos = next.enqueuedAtNanos;
            stream.payloadGeneration = payloadGeneration;
            activePayloadStream = stream;
        }
    }

    synchronized boolean expireCurrentPayload(QueueInputStream stream, long nowNanos) {
        if (stream.currentPayload == null || nowNanos - stream.enqueuedAtNanos <= maxPacketAgeNanos) return false;
        int frameBytes = format.channels * 2;
        int remainingFrames = (stream.currentPayload.length - stream.payloadOffset) / frameBytes;
        droppedFrames += remainingFrames;
        expiredFrames += remainingFrames;
        stream.currentPayload = null;
        stream.payloadOffset = 0;
        stream.enqueuedAtNanos = 0;
        if (activePayloadStream == stream) {
            activePayloadStream = null;
        }
        markDiscontinuity();
        return true;
    }

    private void fadeIn(byte[] payload, int channels, int sampleRate) {
        int frameBytes = channels * 2;
        int frames = payload.length / frameBytes;
        int fadeFrames = Math.min(frames, Math.max(1, sampleRate * FADE_MS / 1000));
        for (int frame = 0; frame < fadeFrames; frame++) {
            int gainNumerator = frame + 1;
            for (int channel = 0; channel < channels; channel++) {
                int index = frame * frameBytes + channel * 2;
                short sample = (short) ((payload[index] & 0xff) | (payload[index + 1] << 8));
                int faded = sample * gainNumerator / fadeFrames;
                payload[index] = (byte) faded;
                payload[index + 1] = (byte) (faded >>> 8);
            }
        }
    }

    synchronized void stop() {
        if (stopped) return;
        stopped = true;
        packets.clear();
        queuedFrames = 0;
        activePayloadStream = null;
        payloadGeneration++;
        notifyAll();
    }

    synchronized void fail(IOException error) {
        if (stopped) return;
        failure = error;
        stopped = true;
        packets.clear();
        queuedFrames = 0;
        activePayloadStream = null;
        payloadGeneration++;
        notifyAll();
    }

    synchronized int size() { return packets.size(); }
    synchronized long droppedFrames() { return droppedFrames; }
    synchronized long expiredFrames() { return expiredFrames; }
    synchronized int targetFrames() { return targetFrames; }
    synchronized long queuedFrames() { return queuedFrames; }
    synchronized UdpPacketDecoder.Format format() { return format; }
    boolean hasAuthenticatedAudio() { return hasAuthenticatedAudio; }
    long lastAuthenticatedAudioAtNanos() { return lastAuthenticatedAudioAtNanos; }

    InputStream asPcmInputStream() { return new QueueInputStream(this); }

    private void waitForChange() throws InterruptedIOException {
        try {
            wait();
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            InterruptedIOException error = new InterruptedIOException("UDP playback interrupted");
            error.initCause(interrupted);
            throw error;
        }
    }

    private void waitForChange(long nanos) throws InterruptedIOException {
        try {
            long millis = nanos / 1_000_000L;
            int extraNanos = (int) (nanos % 1_000_000L);
            wait(millis, extraNanos);
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            InterruptedIOException error = new InterruptedIOException("UDP playback interrupted");
            error.initCause(interrupted);
            throw error;
        }
    }

    static final class AudioPacket {
        final long sequence;
        final long frameIndex;
        final int frameCount;
        final byte[] payload;
        final long enqueuedAtNanos;

        AudioPacket(long sequence, long frameIndex, int frameCount, byte[] payload, long enqueuedAtNanos) {
            this.sequence = sequence;
            this.frameIndex = frameIndex;
            this.frameCount = frameCount;
            this.payload = payload;
            this.enqueuedAtNanos = enqueuedAtNanos;
        }
    }

    static final class QueueInputStream extends InputStream {
        private final UdpPcmQueue queue;
        private final byte[] formatHeader = new byte[8];
        private boolean headerReady;
        private boolean closed;
        private int headerOffset;
        private byte[] currentPayload;
        private int payloadOffset;
        private long payloadGeneration = -1;
        private long enqueuedAtNanos;

        QueueInputStream(UdpPcmQueue queue) { this.queue = queue; }

        @Override
        public int read() throws IOException {
            byte[] one = new byte[1];
            int count = read(one, 0, 1);
            return count < 0 ? -1 : one[0] & 0xff;
        }

        @Override
        public int read(byte[] target, int offset, int length) throws IOException {
            if (target == null) throw new NullPointerException("target");
            if (offset < 0 || length < 0 || length > target.length - offset)
                throw new IndexOutOfBoundsException();
            if (length == 0) return 0;
            if (closed) return -1;
            if (!headerReady) {
                UdpPacketDecoder.Format format = queue.awaitFormat();
                if (format == null) return -1;
                writeLittleInt(formatHeader, 0, format.sampleRate);
                writeLittleShort(formatHeader, 4, format.channels);
                writeLittleShort(formatHeader, 6, 16);
                headerReady = true;
            }
            if (headerOffset < formatHeader.length) {
                int count = Math.min(length, formatHeader.length - headerOffset);
                System.arraycopy(formatHeader, headerOffset, target, offset, count);
                headerOffset += count;
                return count;
            }
            return queue.readPayload(this, target, offset, length);
        }

        @Override
        public void close() {
            closed = true;
            queue.stop();
        }

        private static void writeLittleInt(byte[] target, int offset, int value) {
            target[offset] = (byte) value;
            target[offset + 1] = (byte) (value >>> 8);
            target[offset + 2] = (byte) (value >>> 16);
            target[offset + 3] = (byte) (value >>> 24);
        }

        private static void writeLittleShort(byte[] target, int offset, int value) {
            target[offset] = (byte) value;
            target[offset + 1] = (byte) (value >>> 8);
        }
    }
}

/** Per-session evidence that AudioTrack is consuming recently received and written PCM. */
final class UdpPlaybackActivity {
    private static final long ACTIVE_WINDOW_NANOS = 2_000_000_000L;

    private final Pcm16FrameStream.PlaybackHeadTracker playbackHead = new Pcm16FrameStream.PlaybackHeadTracker();
    private long lastPlayedFrames;
    private boolean hasHeadAdvance;
    private long lastHeadAdvanceAtNanos;
    private volatile boolean hasSuccessfulWrite;
    private volatile long lastSuccessfulWriteAtNanos;

    synchronized void resetPlaybackHead(int position) {
        playbackHead.reset(position);
        lastPlayedFrames = 0;
        hasHeadAdvance = false;
    }

    synchronized long playedFrames(int position) {
        return updatePlaybackHead(position, System.nanoTime());
    }

    void noteSuccessfulWrite(long nowNanos) {
        lastSuccessfulWriteAtNanos = nowNanos;
        hasSuccessfulWrite = true;
    }

    synchronized boolean isActive(long nowNanos, int playbackHeadPosition,
            boolean hasReceivedAudio, long lastReceivedAudioAtNanos, boolean paused) {
        updatePlaybackHead(playbackHeadPosition, nowNanos);
        return !paused && hasReceivedAudio && hasSuccessfulWrite && hasHeadAdvance
                && isRecent(nowNanos, lastReceivedAudioAtNanos)
                && isRecent(nowNanos, lastSuccessfulWriteAtNanos)
                && isRecent(nowNanos, lastHeadAdvanceAtNanos);
    }

    synchronized boolean recentlyAdvanced(long nowNanos, int playbackHeadPosition) {
        updatePlaybackHead(playbackHeadPosition, nowNanos);
        return hasHeadAdvance && isRecent(nowNanos, lastHeadAdvanceAtNanos);
    }

    private long updatePlaybackHead(int position, long nowNanos) {
        long played = playbackHead.update(position);
        if (played > lastPlayedFrames) {
            lastHeadAdvanceAtNanos = nowNanos;
            hasHeadAdvance = true;
        }
        lastPlayedFrames = played;
        return played;
    }

    private static boolean isRecent(long nowNanos, long activityAtNanos) {
        long age = nowNanos - activityAtNanos;
        return age >= 0 && age <= ACTIVE_WINDOW_NANOS;
    }
}
