package app.anpod.receiver;

import com.anpod.server.Pcm16FrameStream;
import java.io.DataInputStream;
import java.io.InputStream;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

public final class AnPodUdpVectorCheck {
    private static final String VECTOR = "414e50440101000001020304050607081112131415161718212223242526272800073bd32562c2e2cdfe093c22223a9705d1ae457629baad04";

    public static void main(String[] args) throws Exception {
        checkJavaVectorThroughReceiverParser();
        checkAuthenticatedQueueAndReplay();
        checkLoopbackReceiveThread();
        System.out.println("Android UDP parser, queue, replay and loopback checks passed.");
    }

    private static void checkJavaVectorThroughReceiverParser() throws Exception {
        byte[] key = new byte[32];
        for (int i = 0; i < key.length; i++) key[i] = (byte) i;
        long sessionId = 0x0102030405060708L;
        long sequence = 0x1112131415161718L;
        long frameIndex = 0x2122232425262728L;
        byte[] payload = {0x00, 0x01, 0x7f, (byte) 0x80, (byte) 0xfe, (byte) 0xff, 0x42};
        byte[] datagram = encode(key, sessionId, sequence, frameIndex, UdpPacketDecoder.FORMAT, payload);
        check(VECTOR.equals(hex(datagram)), "JCA packet differs from fixed Rust vector");

        UdpPacketDecoder parser = new UdpPacketDecoder(sessionId, key);
        UdpPacketDecoder.Packet decoded = parser.open(datagram, datagram.length);
        check(decoded.type == UdpPacketDecoder.FORMAT && decoded.sequence == sequence
                && decoded.sampleFrameIndex == frameIndex && Arrays.equals(decoded.payload, payload),
                "the receiver parser did not decode the shared vector");
        parser.destroy();
    }

    private static void checkAuthenticatedQueueAndReplay() throws Exception {
        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        byte[] key = filledKey(0x5a);
        long sessionId = 0x8123456789abcdefL;
        try (DatagramSocket sender = new DatagramSocket(new InetSocketAddress(loopback, 0))) {
            UdpSessionContext context = new UdpSessionContext(
                    new InetSocketAddress(loopback, 0), (InetSocketAddress) sender.getLocalSocketAddress(),
                    sessionId, key, "balanced");
            UdpPcmQueue queue = new UdpPcmQueue();
            UdpReceiver receiver = new UdpReceiver(context, queue);
            try {
                byte[] format = encode(key, sessionId, 0, 0, UdpPacketDecoder.FORMAT, formatPayload(48_000, 2));
                check(accept(receiver, format, sender) == UdpReceiver.ProcessResult.FORMAT, "format negotiation failed");
                check(queue.format().sampleRate == 48_000 && queue.format().channels == 2, "negotiated format mismatch");
                check(accept(receiver, format, sender) == UdpReceiver.ProcessResult.REPLAY, "duplicate format packet was accepted");

                byte[] first = encode(key, sessionId, 1, 0, UdpPacketDecoder.AUDIO, pcm(240, 0x11));
                byte[] third = encode(key, sessionId, 3, 480, UdpPacketDecoder.AUDIO, pcm(240, 0x33));
                byte[] second = encode(key, sessionId, 2, 240, UdpPacketDecoder.AUDIO, pcm(240, 0x22));
                check(accept(receiver, first, sender) == UdpReceiver.ProcessResult.AUDIO, "normal audio rejected");
                check(accept(receiver, third, sender) == UdpReceiver.ProcessResult.AUDIO, "later audio rejected");
                check(accept(receiver, second, sender) == UdpReceiver.ProcessResult.AUDIO, "in-window reordered audio rejected");
                check(accept(receiver, second, sender) == UdpReceiver.ProcessResult.REPLAY, "duplicate audio reached the queue");
                InputStream pcmInput = queue.asPcmInputStream();
                byte[] legacyHeader = new byte[8];
                new DataInputStream(pcmInput).readFully(legacyHeader);
                check(littleInt(legacyHeader, 0) == 48_000 && littleShort(legacyHeader, 4) == 2
                        && littleShort(legacyHeader, 6) == 16, "UDP format did not adapt to the existing PCM stream header");
                Pcm16FrameStream framedPcm = new Pcm16FrameStream(2);
                byte[] audioBuffer = new byte[960];
                check(framedPcm.readFrames(pcmInput, audioBuffer, audioBuffer.length) == 960
                        && (audioBuffer[0] & 0xff) == 0x11, "first sorted PCM packet mismatch");
                check(framedPcm.readFrames(pcmInput, audioBuffer, audioBuffer.length) == 960
                        && (audioBuffer[0] & 0xff) == 0x22, "reordered PCM packet was not sorted");
                check(framedPcm.readFrames(pcmInput, audioBuffer, audioBuffer.length) == 960
                        && (audioBuffer[0] & 0xff) == 0x33, "last sorted PCM packet mismatch");

                byte[] authenticatedRetry = encode(key, sessionId, 4, 720, UdpPacketDecoder.AUDIO, pcm(240));
                byte[] badTag = authenticatedRetry.clone();
                badTag[badTag.length - 1] ^= 0x40;
                check(accept(receiver, badTag, sender) == UdpReceiver.ProcessResult.AUTHENTICATION,
                        "tampered tag was accepted");
                check(queue.size() == 0, "bad tag changed the audio queue");
                check(accept(receiver, authenticatedRetry, sender) == UdpReceiver.ProcessResult.AUDIO,
                        "bad tag committed the replay sequence");

                byte[] invalidFormat = encode(key, sessionId, 5, 960, UdpPacketDecoder.AUDIO, new byte[] {1, 2, 3});
                check(accept(receiver, invalidFormat, sender) == UdpReceiver.ProcessResult.FORMAT_REJECTED,
                        "unaligned PCM was accepted");
                check(queue.size() == 1 && queue.format().sampleRate == 48_000,
                        "invalid PCM changed the session or queue");
                check(accept(receiver, invalidFormat, sender) == UdpReceiver.ProcessResult.FORMAT_REJECTED,
                        "invalid PCM committed its replay sequence");

                InputStream pauseInput = queue.asPcmInputStream();
                byte[] pauseHeader = new byte[8];
                new DataInputStream(pauseInput).readFully(pauseHeader);
                byte[] partialPayload = new byte[64];
                check(pauseInput.read(partialPayload) == partialPayload.length && (partialPayload[0] & 0xff) == 0x34,
                        "could not read the first portion of the pre-pause payload");

                byte[] wrongSession = encode(key, sessionId + 1, 50, 960, UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, wrongSession, sender) == UdpReceiver.ProcessResult.IDENTITY,
                        "wrong session identity was accepted");
                byte[] wrongFlags = authenticatedRetry.clone();
                wrongFlags[6] = 1;
                check(accept(receiver, wrongFlags, sender) == UdpReceiver.ProcessResult.HEADER_REJECTED,
                        "reserved flags were accepted");
                byte[] wrongLength = authenticatedRetry.clone();
                wrongLength[33]++;
                check(accept(receiver, wrongLength, sender) == UdpReceiver.ProcessResult.HEADER_REJECTED,
                        "inconsistent payload length was accepted");

                int wrongPort = sender.getLocalPort() == 1 ? 2 : 1;
                check(receiver.processDatagram(first, first.length,
                        new InetSocketAddress(loopback, wrongPort)) == UdpReceiver.ProcessResult.WRONG_PEER,
                        "unexpected peer reached the parser");

                queue.setPaused(true);
                byte[] pausedOne = encode(key, sessionId, 7, 960, UdpPacketDecoder.AUDIO, pcm(240));
                byte[] pausedTwo = encode(key, sessionId, 8, 1200, UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, pausedOne, sender) == UdpReceiver.ProcessResult.AUDIO
                        && accept(receiver, pausedTwo, sender) == UdpReceiver.ProcessResult.AUDIO,
                        "valid packets were rejected while paused");
                check(queue.size() == 0, "paused session accumulated audio");
                queue.setPaused(false);
                byte[] delayedPauseAudio = encode(key, sessionId, 6, 960, UdpPacketDecoder.AUDIO, pcm(240, 0x66));
                check(accept(receiver, delayedPauseAudio, sender) == UdpReceiver.ProcessResult.AUDIO
                        && queue.size() == 0, "delayed audio from the pause interval survived resume");
                byte[] stale = encode(key, sessionId, 9, 720, UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, stale, sender) == UdpReceiver.ProcessResult.AUDIO && queue.size() == 0,
                        "pre-pause audio was replayed after resume");

                byte[] newest = encode(key, sessionId, 10, 1440, UdpPacketDecoder.AUDIO, pcm(240, 0x44));
                check(accept(receiver, newest, sender) == UdpReceiver.ProcessResult.AUDIO, "fresh audio rejected");
                byte[] resumedPayload = new byte[64];
                check(pauseInput.read(resumedPayload) == resumedPayload.length && (resumedPayload[0] & 0xff) == 0x44,
                        "the partially consumed pre-pause payload continued after resume");

                byte[] oldWindowPacket = null;
                for (int sequence = 11; sequence <= 20; sequence++) {
                    byte[] packet = encode(key, sessionId, sequence, (sequence - 4L) * 240,
                            UdpPacketDecoder.AUDIO, pcm(240));
                    if (sequence == 20) oldWindowPacket = packet;
                    check(accept(receiver, packet, sender) == UdpReceiver.ProcessResult.AUDIO,
                            "in-window bounded-queue packet rejected");
                }
                check(queue.size() == UdpPcmQueue.CAPACITY_PACKETS, "audio queue exceeded or missed its fixed bound");
                check(queue.take().frameIndex == 2160, "bounded queue retained stale history");
                check(queue.droppedFrames() >= 6L * 240, "queue drop accounting missed stale frames");
                byte[] farAhead = encode(key, sessionId, 100, 4320, UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, farAhead, sender) == UdpReceiver.ProcessResult.AUDIO,
                        "sequence outside the replay window did not advance the window");
                check(accept(receiver, oldWindowPacket, sender) == UdpReceiver.ProcessResult.REPLAY,
                        "packet older than the 64-sequence window was accepted");

                byte[] highSequence = encode(key, sessionId, Long.MAX_VALUE, Long.MIN_VALUE,
                        UdpPacketDecoder.AUDIO, pcm(240));
                byte[] unsignedSuccessor = encode(key, sessionId, Long.MIN_VALUE, Long.MIN_VALUE + 240,
                        UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, highSequence, sender) == UdpReceiver.ProcessResult.AUDIO
                        && accept(receiver, unsignedSuccessor, sender) == UdpReceiver.ProcessResult.AUDIO,
                        "unsigned sequence or frame ordering failed across the signed boundary");
                check(accept(receiver, highSequence, sender) == UdpReceiver.ProcessResult.REPLAY,
                        "unsigned replay window accepted a duplicate");
            } finally {
                receiver.close();
                context.destroy();
            }
        }
    }

    private static void checkLoopbackReceiveThread() throws Exception {
        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        byte[] key = filledKey(0x26);
        long sessionId = 0x1020304050607080L;
        try (DatagramSocket sender = new DatagramSocket(new InetSocketAddress(loopback, 0))) {
            UdpSessionContext context = new UdpSessionContext(
                    new InetSocketAddress(loopback, 0), (InetSocketAddress) sender.getLocalSocketAddress(),
                    sessionId, key, "lowlatency");
            UdpReceiver unregisteredReceiver = new UdpReceiver(context, new UdpPcmQueue());
            InetSocketAddress unregisteredAddress = unregisteredReceiver.localAddress();
            unregisteredReceiver.close();
            try (DatagramSocket rebound = new DatagramSocket(null)) {
                rebound.setReuseAddress(false);
                rebound.bind(unregisteredAddress);
            }
            UdpPcmQueue queue = new UdpPcmQueue();
            UdpReceiver receiver = new UdpReceiver(context, queue);
            Thread thread = new Thread(receiver, "UDP loopback check");
            thread.start();
            ExecutorWait wait = new ExecutorWait(queue);
            try {
                send(sender, receiver.localAddress(), encode(key, sessionId, 0, 0,
                        UdpPacketDecoder.FORMAT, formatPayload(48_000, 2)));
                check(wait.awaitFormat().sampleRate == 48_000, "loopback format packet was not received");
                send(sender, receiver.localAddress(), encode(key, sessionId, 1, 0,
                        UdpPacketDecoder.AUDIO, pcm(240)));
                check(wait.awaitPacket().payload.length == 960, "loopback audio payload mismatch");
            } finally {
                receiver.close();
                thread.join(2_000);
                wait.close();
                context.destroy();
                check(!thread.isAlive(), "closing the UDP socket did not end recv");
            }
        }
    }

    private static UdpReceiver.ProcessResult accept(UdpReceiver receiver, byte[] packet, DatagramSocket sender) {
        return receiver.processDatagram(packet, packet.length, sender.getLocalSocketAddress());
    }

    private static byte[] encode(byte[] key, long sessionId, long sequence, long frameIndex,
            int type, byte[] payload) throws Exception {
        byte[] header = ByteBuffer.allocate(UdpPacketDecoder.HEADER_BYTES).order(ByteOrder.BIG_ENDIAN)
                .put(new byte[] {'A', 'N', 'P', 'D'}).put((byte) 1).put((byte) type).putShort((short) 0)
                .putLong(sessionId).putLong(sequence).putLong(frameIndex).putShort((short) payload.length).array();
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN)
                .putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] encrypted = cipher.doFinal(payload);
        return ByteBuffer.allocate(header.length + encrypted.length).put(header).put(encrypted).array();
    }

    private static byte[] formatPayload(int sampleRate, int channels) {
        return ByteBuffer.allocate(7).order(ByteOrder.BIG_ENDIAN)
                .put((byte) 1).putShort((short) channels).putInt(sampleRate).array();
    }

    private static byte[] pcm(int frames) {
        return pcm(frames, 0x34);
    }

    private static byte[] pcm(int frames, int sample) {
        byte[] payload = new byte[frames * 4];
        for (int i = 0; i < payload.length; i += 2) {
            payload[i] = (byte) sample;
            payload[i + 1] = (byte) (sample >>> 8);
        }
        return payload;
    }

    private static int littleInt(byte[] bytes, int offset) {
        return (bytes[offset] & 0xff) | ((bytes[offset + 1] & 0xff) << 8)
                | ((bytes[offset + 2] & 0xff) << 16) | (bytes[offset + 3] << 24);
    }

    private static int littleShort(byte[] bytes, int offset) {
        return (bytes[offset] & 0xff) | ((bytes[offset + 1] & 0xff) << 8);
    }

    private static byte[] filledKey(int value) {
        byte[] key = new byte[32];
        Arrays.fill(key, (byte) value);
        return key;
    }

    private static void send(DatagramSocket socket, InetSocketAddress address, byte[] bytes) throws Exception {
        socket.send(new DatagramPacket(bytes, bytes.length, address));
    }

    private static String hex(byte[] bytes) {
        StringBuilder result = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) result.append(String.format("%02x", value & 0xff));
        return result.toString();
    }

    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }

    private static final class ExecutorWait implements AutoCloseable {
        private final UdpPcmQueue queue;
        private ExecutorWait(UdpPcmQueue queue) { this.queue = queue; }
        UdpPacketDecoder.Format awaitFormat() throws Exception {
            CountDownLatch done = new CountDownLatch(1);
            AtomicReference<UdpPacketDecoder.Format> result = new AtomicReference<>();
            AtomicReference<Throwable> error = new AtomicReference<>();
            Thread thread = new Thread(() -> {
                try { result.set(queue.awaitFormat()); } catch (Throwable failure) { error.set(failure); }
                finally { done.countDown(); }
            });
            thread.start();
            check(done.await(2, TimeUnit.SECONDS), "timed out waiting for loopback format");
            if (error.get() != null) throw new AssertionError(error.get());
            return result.get();
        }
        UdpPcmQueue.AudioPacket awaitPacket() throws Exception {
            CountDownLatch done = new CountDownLatch(1);
            AtomicReference<UdpPcmQueue.AudioPacket> result = new AtomicReference<>();
            AtomicReference<Throwable> error = new AtomicReference<>();
            Thread thread = new Thread(() -> {
                try { result.set(queue.take()); } catch (Throwable failure) { error.set(failure); }
                finally { done.countDown(); }
            });
            thread.start();
            check(done.await(2, TimeUnit.SECONDS), "timed out waiting for loopback PCM");
            if (error.get() != null) throw new AssertionError(error.get());
            return result.get();
        }
        @Override public void close() { queue.stop(); }
    }
}
