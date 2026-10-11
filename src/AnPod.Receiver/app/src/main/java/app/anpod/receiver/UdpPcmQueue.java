package app.anpod.receiver;

import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.util.ArrayList;

/** Short-lock packet queue and byte-stream adapter for the existing AudioTrack worker. */
final class UdpPcmQueue {
    static final int CAPACITY_PACKETS = 8;
    private final ArrayList<AudioPacket> packets = new ArrayList<>(CAPACITY_PACKETS);
    private final PairingProtocol.AudioParameters expectedParameters;
    private UdpPacketDecoder.Format format;
    private boolean paused;
    private boolean stopped;
    private IOException failure;
    private boolean hasPlayoutFrame;
    private long playoutFrame;
    private long payloadGeneration;
    private long droppedFrames;

    UdpPcmQueue() { this(null); }

    UdpPcmQueue(PairingProtocol.AudioParameters expectedParameters) {
        this.expectedParameters = expectedParameters;
    }

    synchronized void accept(UdpPacketDecoder.Packet packet, UdpPacketDecoder decoder)
            throws UdpPacketDecoder.ProtocolException {
        if (stopped) throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.AUDIO);
        if (packet.type == UdpPacketDecoder.FORMAT) {
            if (format != null || packet.sequence != 0)
                throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
            UdpPacketDecoder.Format acceptedFormat = UdpPacketDecoder.decodeFormat(packet);
            if (expectedParameters != null && (acceptedFormat.sampleRate != expectedParameters.sampleRate
                    || acceptedFormat.channels != expectedParameters.channels))
                throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
            decoder.commit(packet.sequence);
            format = acceptedFormat;
            notifyAll();
            return;
        }

        if (packet.type != UdpPacketDecoder.AUDIO || format == null || packet.payload.length == 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
        int frameBytes = format.channels * 2;
        if (packet.payload.length % frameBytes != 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);
        int frameCount = packet.payload.length / frameBytes;
        long frameEnd = packet.sampleFrameIndex + frameCount;
        if (Long.compareUnsigned(frameEnd, packet.sampleFrameIndex) <= 0)
            throw new UdpPacketDecoder.ProtocolException(UdpPacketDecoder.ProtocolException.FORMAT);

        decoder.commit(packet.sequence);
        if (paused) {
            droppedFrames += frameCount;
            advancePlayoutFrame(frameEnd);
            return;
        }
        if (hasPlayoutFrame && Long.compareUnsigned(packet.sampleFrameIndex, playoutFrame) < 0) {
            droppedFrames += frameCount;
            return;
        }

        AudioPacket audio = new AudioPacket(packet.sequence, packet.sampleFrameIndex, frameCount, packet.payload);
        int insertAt = 0;
        while (insertAt < packets.size()
                && Long.compareUnsigned(packets.get(insertAt).frameIndex, audio.frameIndex) <= 0) insertAt++;
        if (packets.size() == CAPACITY_PACKETS) {
            if (insertAt == 0) {
                droppedFrames += frameCount;
                return;
            }
            AudioPacket stale = packets.remove(0);
            droppedFrames += stale.frameCount;
            long staleEnd = stale.frameIndex + stale.frameCount;
            if (!hasPlayoutFrame || Long.compareUnsigned(staleEnd, playoutFrame) > 0) {
                playoutFrame = staleEnd;
                hasPlayoutFrame = true;
            }
            if (Long.compareUnsigned(audio.frameIndex, playoutFrame) < 0) {
                droppedFrames += frameCount;
                return;
            }
            insertAt--;
        }
        packets.add(insertAt, audio);
        notifyAll();
    }

    synchronized UdpPacketDecoder.Format awaitFormat() throws IOException {
        while (format == null && !stopped) waitForChange();
        if (format == null && failure != null) throw failure;
        return format;
    }

    synchronized AudioPacket take() throws IOException {
        while ((packets.isEmpty() || paused) && !stopped) waitForChange();
        if (packets.isEmpty() && failure != null) throw failure;
        if (stopped) return null;
        AudioPacket packet = packets.remove(0);
        playoutFrame = packet.frameIndex + packet.frameCount;
        hasPlayoutFrame = true;
        return packet;
    }

    synchronized void setPaused(boolean value) {
        if (paused == value) return;
        paused = value;
        payloadGeneration++;
        if (paused) {
            for (AudioPacket packet : packets) {
                droppedFrames += packet.frameCount;
                long end = packet.frameIndex + packet.frameCount;
                advancePlayoutFrame(end);
            }
            packets.clear();
        }
        notifyAll();
    }

    private void advancePlayoutFrame(long frame) {
        if (!hasPlayoutFrame || Long.compareUnsigned(frame, playoutFrame) > 0) {
            playoutFrame = frame;
            hasPlayoutFrame = true;
        }
    }

    private synchronized int readPayload(QueueInputStream stream, byte[] target, int offset, int length)
            throws IOException {
        while (true) {
            if (stream.payloadGeneration != payloadGeneration) {
                stream.currentPayload = null;
                stream.payloadOffset = 0;
                stream.payloadGeneration = payloadGeneration;
            }
            if (failure != null) throw failure;
            if (stopped) return -1;
            if (paused) {
                waitForChange();
                continue;
            }
            if (stream.currentPayload == null || stream.payloadOffset == stream.currentPayload.length) {
                if (packets.isEmpty()) {
                    waitForChange();
                    continue;
                }
                AudioPacket packet = packets.remove(0);
                advancePlayoutFrame(packet.frameIndex + packet.frameCount);
                stream.currentPayload = packet.payload;
                stream.payloadOffset = 0;
            }
            int count = Math.min(length, stream.currentPayload.length - stream.payloadOffset);
            System.arraycopy(stream.currentPayload, stream.payloadOffset, target, offset, count);
            stream.payloadOffset += count;
            return count;
        }
    }

    synchronized void stop() {
        if (stopped) return;
        stopped = true;
        packets.clear();
        notifyAll();
    }

    synchronized void fail(IOException error) {
        if (stopped) return;
        failure = error;
        stopped = true;
        packets.clear();
        notifyAll();
    }

    synchronized int size() { return packets.size(); }
    synchronized long droppedFrames() { return droppedFrames; }
    synchronized UdpPacketDecoder.Format format() { return format; }

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

    static final class AudioPacket {
        final long sequence;
        final long frameIndex;
        final int frameCount;
        final byte[] payload;

        AudioPacket(long sequence, long frameIndex, int frameCount, byte[] payload) {
            this.sequence = sequence;
            this.frameIndex = frameIndex;
            this.frameCount = frameCount;
            this.payload = payload;
        }
    }

    private static final class QueueInputStream extends InputStream {
        private final UdpPcmQueue queue;
        private final byte[] formatHeader = new byte[8];
        private boolean headerReady;
        private boolean closed;
        private int headerOffset;
        private byte[] currentPayload;
        private int payloadOffset;
        private long payloadGeneration = -1;

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
