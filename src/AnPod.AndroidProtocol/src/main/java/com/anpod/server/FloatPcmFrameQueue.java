package com.anpod.server;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Arrays;

/** Bounded Float32 packet queue whose playout, concealment and edits use sample frames. */
public final class FloatPcmFrameQueue {
    public static final int FADE_MS = 3;
    public static final int LOW_LATENCY_INITIAL_MS = 20;
    public static final int BALANCED_INITIAL_MS = 40;
    private static final int LOW_LATENCY_MAX_MS = 60;
    private static final int BALANCED_MAX_MS = 150;
    private static final int MIN_PACKET_WINDOW_MS = 5;
    private static final int GAP_WAIT_MS = 5;
    private static final long NANOS_PER_MILLI = 1_000_000L;
    private static final long LONG_SOURCE_GAP_NANOS = 100 * NANOS_PER_MILLI;
    private static final int CHANNELS = 2;
    private static final int DEFAULT_MAX_PACKET_FRAMES = 1150 / (CHANNELS * Pcm24PackedCodec.BYTES_PER_SAMPLE);

    private final ArrayList<Packet> packets;
    private final int sampleRate;
    private final int maxPacketFrames;
    private final int initialTargetFrames;
    private final int maxQueuedFrames;
    private final long maxPacketAgeNanos;
    private final int fadeFrames;
    private final float[] lastFrame = new float[CHANNELS];
    private boolean hasPlayoutFrame;
    private boolean hasAdvanced;
    private long playoutFrame;
    private boolean hasLastFrame;
    private boolean fadeNext;
    private int fadeInProgressFrames;
    private boolean paused;
    private boolean stopped;
    private IOException failure;
    private boolean started;
    private long gapWaitStartedNanos = -1;
    private long noSourceSinceNanos = -1;
    private int plcFrames;
    private int queuedFrames;
    private long droppedFrames;
    private long expiredFrames;
    private long concealedFrames;
    private long oversizedGapSkips;
    private long insertedDriftFrames;
    private long deletedDriftFrames;

    public FloatPcmFrameQueue(int sampleRate, boolean lowLatency) {
        this(sampleRate, lowLatency, DEFAULT_MAX_PACKET_FRAMES);
    }

    public FloatPcmFrameQueue(int sampleRate, boolean lowLatency, int maxPacketFrames) {
        if (sampleRate <= 0) throw new IllegalArgumentException("sample rate must be positive");
        if (maxPacketFrames <= 0) throw new IllegalArgumentException("max packet frames must be positive");
        this.sampleRate = sampleRate;
        this.maxPacketFrames = maxPacketFrames;
        int initialMs = lowLatency ? LOW_LATENCY_INITIAL_MS : BALANCED_INITIAL_MS;
        int maxMs = lowLatency ? LOW_LATENCY_MAX_MS : BALANCED_MAX_MS;
        initialTargetFrames = framesForMillis(sampleRate, initialMs);
        maxQueuedFrames = framesForMillis(sampleRate, maxMs);
        maxPacketAgeNanos = maxMs * NANOS_PER_MILLI;
        int framesPerWindow = framesForMillis(sampleRate, MIN_PACKET_WINDOW_MS);
        int packetsPerWindow = (framesPerWindow + maxPacketFrames - 1) / maxPacketFrames;
        int expectedPacketCapacity = ((maxMs + MIN_PACKET_WINDOW_MS - 1) / MIN_PACKET_WINDOW_MS) * packetsPerWindow;
        packets = new ArrayList<>(expectedPacketCapacity);
        fadeFrames = Math.max(1, framesForMillis(sampleRate, FADE_MS));
    }

    /** Copies one interleaved stereo packet. A packet is limited to the v2 1150-byte payload. */
    public synchronized boolean offer(long frameIndex, float[] samples, int frameCount) {
        return offer(frameIndex, samples, frameCount, System.nanoTime());
    }

    /** Explicit arrival time keeps expiry and pressure behavior deterministic in CPU checks. */
    public synchronized boolean offer(long frameIndex, float[] samples, int frameCount, long nowNanos) {
        if (samples == null) throw new NullPointerException("samples");
        if (frameCount <= 0 || frameCount > maxPacketFrames
                || (long) frameCount * CHANNELS != samples.length) {
            throw new IllegalArgumentException("Float32 packet must contain whole, bounded stereo frames");
        }
        long endFrame = frameIndex + frameCount;
        if (Long.compareUnsigned(endFrame, frameIndex) <= 0)
            throw new IllegalArgumentException("packet frame range overflows");
        expireStalePackets(nowNanos);
        if (hasAdvanced && Long.compareUnsigned(frameIndex, playoutFrame) < 0) {
            droppedFrames += frameCount;
            return false;
        }
        if (paused || stopped) {
            droppedFrames += frameCount;
            return false;
        }
        int insert = 0;
        while (insert < packets.size()
                && Long.compareUnsigned(packets.get(insert).frameIndex, frameIndex) < 0) insert++;
        if (insert > 0 && overlaps(packets.get(insert - 1), frameIndex, endFrame)
                || insert < packets.size() && overlaps(packets.get(insert), frameIndex, endFrame)) {
            droppedFrames += frameCount;
            return false;
        }
        if (frameCount > maxQueuedFrames) {
            droppedFrames += frameCount;
            expiredFrames += frameCount;
            return false;
        }
        int evictions = 0;
        int remainingFrames = queuedFrames;
        while (remainingFrames + frameCount > maxQueuedFrames && evictions < packets.size()) {
            Packet oldest = packets.get(evictions);
            long oldestEnd = oldest.frameIndex + oldest.frameCount;
            if (Long.compareUnsigned(frameIndex, oldestEnd) < 0) {
                droppedFrames += frameCount;
                expiredFrames += frameCount;
                return false;
            }
            remainingFrames -= oldest.frameCount - oldest.offsetFrames;
            evictions++;
        }
        if (remainingFrames + frameCount > maxQueuedFrames) {
            droppedFrames += frameCount;
            expiredFrames += frameCount;
            return false;
        }
        for (int index = 0; index < evictions; index++) {
            Packet oldest = packets.remove(0);
            queuedFrames -= oldest.frameCount - oldest.offsetFrames;
            dropQueuedPacket(oldest, true);
        }
        discardStalePackets();
        if (hasAdvanced && Long.compareUnsigned(frameIndex, playoutFrame) < 0) {
            droppedFrames += frameCount;
            expiredFrames += frameCount;
            return false;
        }
        insert = 0;
        while (insert < packets.size()
                && Long.compareUnsigned(packets.get(insert).frameIndex, frameIndex) < 0) insert++;
        packets.add(insert, new Packet(frameIndex, Arrays.copyOf(samples, samples.length), nowNanos));
        queuedFrames += frameCount;
        if (!hasPlayoutFrame) {
            hasPlayoutFrame = true;
            playoutFrame = frameIndex;
        } else if (!hasAdvanced && Long.compareUnsigned(frameIndex, playoutFrame) < 0) {
            playoutFrame = frameIndex;
        }
        notifyAll();
        return true;
    }

    /** Waits for initial buffering, then returns contiguous audio or bounded PLC frames. */
    public synchronized int readFrames(float[] destination, int destinationFrameOffset, int maxFrames)
            throws IOException, InterruptedException {
        checkDestination(destination, destinationFrameOffset, maxFrames);
        if (maxFrames == 0) return 0;
        long gapDeadlineNanos = -1;
        while (!stopped) {
            if (failure != null) throw failure;
            if (started && noSourceSinceNanos >= 0
                    && System.nanoTime() - noSourceSinceNanos >= LONG_SOURCE_GAP_NANOS) started = false;
            if (paused || !hasPlayoutFrame || (!started && queuedFrames < initialTargetFrames)) {
                wait();
                continue;
            }
            int frames = poll(destination, destinationFrameOffset, maxFrames);
            if (frames > 0) {
                started = true;
                noSourceSinceNanos = -1;
                gapWaitStartedNanos = -1;
                return frames;
            }
            if (packets.isEmpty()) {
                if (started && noSourceSinceNanos < 0) noSourceSinceNanos = System.nanoTime();
                gapWaitStartedNanos = -1;
                wait();
                continue;
            }
            if (gapWaitStartedNanos < 0) gapWaitStartedNanos = System.nanoTime();
            long remaining = java.util.concurrent.TimeUnit.MILLISECONDS.toNanos(GAP_WAIT_MS)
                    - (System.nanoTime() - gapWaitStartedNanos);
            if (remaining > 0) {
                long waitMillis = Math.max(1, remaining / 1_000_000L);
                wait(waitMillis);
                continue;
            }
            frames = concealGap(destination, destinationFrameOffset, maxFrames);
            if (frames > 0) {
                started = true;
                gapWaitStartedNanos = -1;
                return frames;
            }
            gapWaitStartedNanos = -1;
        }
        if (failure != null) throw failure;
        return -1;
    }

    public synchronized void fail(IOException error) {
        if (stopped || failure != null) return;
        failure = error == null ? new IOException("Float32 audio receiver failed") : error;
        notifyAll();
    }

    public synchronized void setPaused(boolean value) {
        if (stopped || paused == value) return;
        paused = value;
        packets.clear();
        queuedFrames = 0;
        hasPlayoutFrame = false;
        hasAdvanced = false;
        hasLastFrame = false;
        fadeNext = false;
        fadeInProgressFrames = 0;
        started = false;
        noSourceSinceNanos = -1;
        gapWaitStartedNanos = -1;
        notifyAll();
    }

    public synchronized void stop() {
        stopped = true;
        packets.clear();
        queuedFrames = 0;
        notifyAll();
    }

    /** Copies contiguous frames at the playout cursor; returns 0 while a gap is pending. */
    public synchronized int poll(float[] destination, int destinationFrameOffset, int maxFrames) {
        checkDestination(destination, destinationFrameOffset, maxFrames);
        if (!hasPlayoutFrame || maxFrames == 0) return 0;
        discardStalePackets();
        if (packets.isEmpty() || Long.compareUnsigned(packets.get(0).frameIndex, playoutFrame) > 0)
            return 0;

        Packet packet = packets.get(0);
        int sourceOffsetFrames = (int) (playoutFrame - packet.frameIndex);
        int count = Math.min(maxFrames, packet.frameCount - sourceOffsetFrames);
        int sourceOffset = sourceOffsetFrames * CHANNELS;
        int destinationOffset = destinationFrameOffset * CHANNELS;
        System.arraycopy(packet.samples, sourceOffset, destination, destinationOffset, count * CHANNELS);
        if (fadeNext) {
            int fadeCount = Math.min(count, fadeFrames - fadeInProgressFrames);
            for (int frame = 0; frame < fadeCount; frame++) {
                float gain = (fadeInProgressFrames + frame) / (float) fadeFrames;
                int sampleOffset = destinationOffset + frame * CHANNELS;
                for (int channel = 0; channel < CHANNELS; channel++)
                    destination[sampleOffset + channel] *= gain;
            }
            fadeInProgressFrames += fadeCount;
            fadeNext = fadeInProgressFrames < fadeFrames;
        }
        System.arraycopy(destination, destinationOffset + (count - 1) * CHANNELS,
                lastFrame, 0, CHANNELS);
        hasLastFrame = true;
        plcFrames = 0;
        playoutFrame += count;
        hasAdvanced = true;
        queuedFrames -= count;
        packet.offsetFrames += count;
        if (packet.offsetFrames == packet.frameCount) packets.remove(0);
        return count;
    }

    /** Fills a bounded frame-index gap by fading the last stereo frame to zero. */
    public synchronized int concealGap(float[] destination, int destinationFrameOffset, int maxFrames) {
        checkDestination(destination, destinationFrameOffset, maxFrames);
        if (!hasPlayoutFrame || packets.isEmpty() || maxFrames == 0) return 0;
        long gap = packets.get(0).frameIndex - playoutFrame;
        if (Long.compareUnsigned(gap, 0) <= 0) return 0;
        if (Long.compareUnsigned(gap, initialTargetFrames) > 0) {
            playoutFrame = packets.get(0).frameIndex;
            hasAdvanced = true;
            oversizedGapSkips++;
            started = false;
            fadeNext = true;
            fadeInProgressFrames = 0;
            hasLastFrame = false;
            plcFrames = 0;
            return 0;
        }

        int count = Math.min(maxFrames, (int) gap);
        int destinationOffset = destinationFrameOffset * CHANNELS;
        for (int frame = 0; frame < count; frame++) {
            float gain = hasLastFrame && plcFrames + frame < fadeFrames
                    ? (fadeFrames - plcFrames - frame - 1) / (float) fadeFrames : 0f;
            destination[destinationOffset + frame * CHANNELS] = lastFrame[0] * gain;
            destination[destinationOffset + frame * CHANNELS + 1] = lastFrame[1] * gain;
        }
        playoutFrame += count;
        hasAdvanced = true;
        plcFrames += count;
        concealedFrames += count;
        fadeNext = true;
        fadeInProgressFrames = 0;
        return count;
    }

    /** Applies one frame insertion (-1) or deletion (+1) in place without integer quantization. */
    public synchronized int adjustDrift(float[] samples, int frameCount, int capacityFrames, int adjustment) {
        if (samples == null) throw new NullPointerException("samples");
        if (frameCount < 0 || capacityFrames < frameCount || capacityFrames > samples.length / CHANNELS
                || adjustment < -1 || adjustment > 1 || (long) frameCount * CHANNELS > samples.length)
            throw new IllegalArgumentException("invalid Float32 drift buffer");
        if (adjustment == 0) return frameCount;
        if (adjustment > 0) {
            if (frameCount < 3) throw new IllegalArgumentException("not enough frames to remove one");
            int frame = lowestDifferenceFrame(samples, frameCount);
            int moveSamples = (frameCount - frame - 1) * CHANNELS;
            System.arraycopy(samples, (frame + 1) * CHANNELS, samples, frame * CHANNELS, moveSamples);
            deletedDriftFrames++;
            return frameCount - 1;
        }
        if (frameCount < 2 || frameCount == capacityFrames)
            throw new IllegalArgumentException("no capacity to insert one frame");
        int pair = lowestDifferencePair(samples, frameCount);
        int insert = pair + 1;
        int moveSamples = (frameCount - insert) * CHANNELS;
        System.arraycopy(samples, insert * CHANNELS, samples, (insert + 1) * CHANNELS, moveSamples);
        for (int channel = 0; channel < CHANNELS; channel++)
            samples[insert * CHANNELS + channel] =
                    (samples[pair * CHANNELS + channel] + samples[(pair + 1) * CHANNELS + channel]) * 0.5f;
        insertedDriftFrames++;
        return frameCount + 1;
    }

    public static void fadeIn(float[] samples, int frameOffset, int frameCount,
            int channels, int fadeFrames) {
        fade(samples, frameOffset, frameCount, channels, fadeFrames, true);
    }

    public static void fadeOut(float[] samples, int frameOffset, int frameCount,
            int channels, int fadeFrames) {
        fade(samples, frameOffset, frameCount, channels, fadeFrames, false);
    }

    public int initialTargetFrames() { return initialTargetFrames; }
    public int audioTrackBufferFrames(int capacityFrames, boolean lowLatency) {
        return Pcm16FrameStream.requestedAudioTrackBufferFrames(sampleRate, capacityFrames, lowLatency);
    }
    public synchronized long playoutFrame() { return playoutFrame; }
    public synchronized int packetCount() { return packets.size(); }
    public int packetCapacity() { return maxQueuedFrames; }
    public synchronized int queuedFrames() { return queuedFrames; }
    public synchronized boolean isStopped() { return stopped; }
    public synchronized long droppedFrames() { return droppedFrames; }
    public synchronized long expiredFrames() { return expiredFrames; }
    public synchronized long concealedFrames() { return concealedFrames; }
    public synchronized long oversizedGapSkips() { return oversizedGapSkips; }
    public synchronized long insertedDriftFrames() { return insertedDriftFrames; }
    public synchronized long deletedDriftFrames() { return deletedDriftFrames; }

    private static boolean overlaps(Packet packet, long start, long end) {
        long packetEnd = packet.frameIndex + packet.frameCount;
        return Long.compareUnsigned(start, packetEnd) < 0
                && Long.compareUnsigned(end, packet.frameIndex) > 0;
    }

    private void discardStalePackets() {
        while (!packets.isEmpty()) {
            Packet packet = packets.get(0);
            long packetEnd = packet.frameIndex + packet.frameCount;
            if (Long.compareUnsigned(packetEnd, playoutFrame) > 0) break;
            packets.remove(0);
            queuedFrames -= packet.frameCount - packet.offsetFrames;
            droppedFrames += packet.frameCount - packet.offsetFrames;
        }
    }

    private void expireStalePackets(long nowNanos) {
        boolean changed = false;
        for (int index = packets.size() - 1; index >= 0; index--) {
            Packet packet = packets.get(index);
            if (nowNanos - packet.enqueuedAtNanos <= maxPacketAgeNanos) continue;
            packets.remove(index);
            queuedFrames -= packet.frameCount - packet.offsetFrames;
            dropQueuedPacket(packet, true);
            changed = true;
        }
        if (changed) discardStalePackets();
    }

    private void dropQueuedPacket(Packet packet, boolean expired) {
        int remaining = packet.frameCount - packet.offsetFrames;
        droppedFrames += remaining;
        if (expired) expiredFrames += remaining;
        long end = packet.frameIndex + packet.frameCount;
        boolean future = !hasPlayoutFrame || Long.compareUnsigned(end, playoutFrame) > 0;
        if (future) {
            playoutFrame = end;
            hasPlayoutFrame = true;
            hasAdvanced = true;
            fadeNext = true;
            fadeInProgressFrames = 0;
            hasLastFrame = false;
            plcFrames = 0;
        }
    }

    private static int lowestDifferenceFrame(float[] samples, int frames) {
        int best = 1;
        double bestDifference = Double.POSITIVE_INFINITY;
        for (int frame = 1; frame < frames - 1; frame++) {
            double difference = 0;
            for (int channel = 0; channel < CHANNELS; channel++)
                difference += Math.abs((double) samples[(frame - 1) * CHANNELS + channel]
                        - samples[(frame + 1) * CHANNELS + channel]);
            if (difference < bestDifference) { best = frame; bestDifference = difference; }
        }
        return best;
    }

    private static int lowestDifferencePair(float[] samples, int frames) {
        int best = 0;
        double bestDifference = Double.POSITIVE_INFINITY;
        for (int frame = 0; frame < frames - 1; frame++) {
            double difference = 0;
            for (int channel = 0; channel < CHANNELS; channel++)
                difference += Math.abs((double) samples[frame * CHANNELS + channel]
                        - samples[(frame + 1) * CHANNELS + channel]);
            if (difference < bestDifference) { best = frame; bestDifference = difference; }
        }
        return best;
    }

    private static void fade(float[] samples, int frameOffset, int frameCount,
            int channels, int fadeFrames, boolean fadeIn) {
        if (samples == null) throw new NullPointerException("samples");
        if (channels <= 0 || frameOffset < 0 || frameCount < 0 || fadeFrames <= 0
                || (long) (frameOffset + (long) frameCount) * channels > samples.length)
            throw new IllegalArgumentException("invalid Float32 fade range");
        int count = Math.min(frameCount, fadeFrames);
        int startFrame = fadeIn ? frameOffset : frameOffset + frameCount - count;
        for (int frame = 0; frame < count; frame++) {
            float gain = fadeIn ? frame / (float) fadeFrames
                    : (count - frame - 1) / (float) fadeFrames;
            int sampleOffset = (startFrame + frame) * channels;
            for (int channel = 0; channel < channels; channel++)
                samples[sampleOffset + channel] *= gain;
        }
    }

    private static void checkDestination(float[] destination, int frameOffset, int frameCount) {
        if (destination == null) throw new NullPointerException("destination");
        if (frameOffset < 0 || frameCount < 0 || (long) (frameOffset + (long) frameCount) * CHANNELS > destination.length)
            throw new IllegalArgumentException("Float32 destination is too small");
    }

    private static int framesForMillis(int sampleRate, int millis) {
        return (int) (((long) sampleRate * millis + 999) / 1000);
    }

    private static final class Packet {
        final long frameIndex;
        final int frameCount;
        final float[] samples;
        final long enqueuedAtNanos;
        int offsetFrames;

        Packet(long frameIndex, float[] samples, long enqueuedAtNanos) {
            this.frameIndex = frameIndex;
            this.frameCount = samples.length / CHANNELS;
            this.samples = samples;
            this.enqueuedAtNanos = enqueuedAtNanos;
        }
    }
}
