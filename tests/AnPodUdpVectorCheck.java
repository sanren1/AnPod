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
        checkFormatSequenceRejected();
        checkLoopbackReceiveThread();
        checkPairingSecurity();
        System.out.println("Android UDP parser, pairing, replay and loopback checks passed.");
    }

    private static void checkPairingSecurity() throws Exception {
        PairingProtocol.BindingMaterial generatedOne = PairingProtocol.BindingMaterial.generate();
        PairingProtocol.BindingMaterial generatedTwo = PairingProtocol.BindingMaterial.generate();
        byte[] generatedRecordOne = generatedOne.encodedRecord();
        byte[] generatedRecordTwo = generatedTwo.encodedRecord();
        check(!Arrays.equals(generatedRecordOne, generatedRecordTwo), "new binding material was not random");
        generatedOne.close();
        generatedTwo.close();
        Arrays.fill(generatedRecordOne, (byte) 0);
        Arrays.fill(generatedRecordTwo, (byte) 0);

        byte[] secret = sequence(0, 32);
        byte[] hostId = sequence(0x10, 16);
        byte[] receiverId = sequence(0x20, 16);
        byte[] hostNonce = sequence(0x30, 32);
        byte[] receiverNonce = sequence(0x50, 32);
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        byte[] transcript = PairingProtocol.transcriptForVector(hostId, receiverId, hostNonce,
                receiverNonce, 0x0102030405060708L, parameters);
        check(hex(transcript).equals("414e5035010102101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f010203040506070801020000bb8000"),
                "P5-A transcript differs from the shared Rust vector");
        byte[] hostProof = PairingProtocol.proofForVector(secret, "host", transcript);
        byte[] receiverProof = PairingProtocol.proofForVector(secret, "receiver", transcript);
        check(hex(hostProof).equals("2d28ffee9d17f1930898388aa09f4291dc47aba00f201ecdf58a00163388aaed"),
                "host proof differs from the shared Rust vector");
        check(hex(receiverProof).equals("1b4af0b2591a958ddb36186bdccd8fd1cdfc6c2f599a48aabeea5a865a8482a6"),
                "receiver proof differs from the shared Rust vector");
        check(hex(PairingProtocol.readyForVector(secret, transcript, hostProof, receiverProof))
                        .equals("80f32dbf0a5c1a29ec777fd06090593869254a34bc05eca236ad8b47b440fa1c"),
                "ready proof differs from the shared Rust vector");
        try (PairingProtocol.SessionKeys vectorKeys = PairingProtocol.deriveForVector(secret, transcript)) {
            check(hex(vectorKeys.hostToReceiverAudio).equals("00021dc01623e41fc2a636df8fb67b19bfc7d83bd0a762ef8759d149132fac58"),
                    "host audio key differs from the shared Rust vector");
            check(hex(vectorKeys.receiverToHostAudio).equals("fdbf5d4561402f457655b436f24557461e523eecd4a3e56b3634f4c47e1ebc7a"),
                    "receiver audio key differs from the shared Rust vector");
            check(hex(vectorKeys.hostToReceiverControl).equals("7b307e8464b5860d3cacf168026bab443e1ddf98adbe87204d4ccdcd6c324050"),
                    "host control key differs from the shared Rust vector");
            check(hex(vectorKeys.receiverToHostControl).equals("4c9c03cc5cc1fb8e04f7fc3a710886f8c9833901ab90464184d5256319943711"),
                    "receiver control key differs from the shared Rust vector");
        }

        PairingSession first = completePairing(hostId, receiverId, secret);
        PairingSession second = completePairing(hostId, receiverId, secret);
        try (first; second) {
            check(first.sessionId != second.sessionId, "rebuilt handshake reused a session id");
            check(!Arrays.equals(first.keys.hostToReceiverAudio, second.keys.hostToReceiverAudio),
                    "rebuilt handshake reused an audio key");
            check(!Arrays.equals(first.hostNonce, second.hostNonce)
                            && !Arrays.equals(first.receiverNonce, second.receiverNonce),
                    "rebuilt handshake reused a challenge nonce");
        }

        PairingProtocol.BindingMaterial hostMaterial = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        PairingProtocol.BindingMaterial receiverMaterial = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        PairingProtocol.HostHandshake host = PairingProtocol.HostHandshake.start(hostMaterial, parameters);
        PairingProtocol.ReceiverHandshake receiver = PairingProtocol.ReceiverHandshake.acceptHello(
                receiverMaterial, host.hello());
        byte[] badProof = receiver.challenge();
        badProof[badProof.length - 1] ^= 1;
        try {
            host.acceptChallenge(badProof);
            throw new AssertionError("bad receiver proof was accepted");
        } catch (PairingProtocol.Failure expected) { }
        finally { receiver.close(); }

        byte[] oldChallenge;
        PairingProtocol.BindingMaterial oldReceiverMaterial = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        PairingProtocol.HostHandshake oldHost = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        PairingProtocol.ReceiverHandshake oldReceiver = PairingProtocol.ReceiverHandshake.acceptHello(
                oldReceiverMaterial, oldHost.hello());
        oldChallenge = oldReceiver.challenge();
        PairingProtocol.HostHandshake newHost = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        try {
            newHost.acceptChallenge(oldChallenge);
            throw new AssertionError("cross-session challenge response was accepted");
        } catch (PairingProtocol.Failure expected) { }
        finally { oldHost.close(); oldReceiver.close(); }

        PairingProtocol.HostHandshake alteredHost = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        byte[] alteredHello = alteredHost.hello();
        alteredHello[22] ^= 1;
        try {
            expectPairingFailure(() -> PairingProtocol.ReceiverHandshake.acceptHello(
                    new PairingProtocol.BindingMaterial(hostId, receiverId, secret), alteredHello),
                    "wrong trusted identity was accepted");
        } finally { alteredHost.close(); }

        PairingProtocol.HostHandshake wrongRoleHost = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        byte[] wrongRoleHello = wrongRoleHost.hello();
        wrongRoleHello[5] = 3;
        try {
            expectPairingFailure(() -> PairingProtocol.ReceiverHandshake.acceptHello(
                    new PairingProtocol.BindingMaterial(hostId, receiverId, secret), wrongRoleHello),
                    "wrong handshake message role was accepted");
        } finally { wrongRoleHost.close(); }

        PairingProtocol.HostHandshake parameterHost = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        byte[] changedParameters = parameterHost.hello();
        changedParameters[82] ^= 1;
        PairingProtocol.ReceiverHandshake parameterReceiver = PairingProtocol.ReceiverHandshake.acceptHello(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), changedParameters);
        try {
            parameterHost.acceptChallenge(parameterReceiver.challenge());
            throw new AssertionError("audio parameter substitution was accepted");
        } catch (PairingProtocol.Failure expected) { }
        finally { parameterReceiver.close(); }

        PairingProtocol.HostHandshake noReadyHost = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        PairingProtocol.ReceiverHandshake noReadyReceiver = PairingProtocol.ReceiverHandshake.acceptHello(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), noReadyHost.hello());
        PairingProtocol.HostAwaitReady awaitingReady = noReadyHost.acceptChallenge(noReadyReceiver.challenge());
        try {
            awaitingReady.acceptReady(new byte[38]);
            throw new AssertionError("host created session without receiver Ready proof");
        } catch (PairingProtocol.Failure expected) { }
        finally { noReadyReceiver.close(); }

        Arrays.fill(secret, (byte) 0);
    }

    private static PairingSession completePairing(byte[] hostId, byte[] receiverId, byte[] secret) throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 1);
        PairingProtocol.HostHandshake host = PairingProtocol.HostHandshake.start(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), parameters);
        byte[] hello = host.hello();
        PairingProtocol.ReceiverHandshake receiver = PairingProtocol.ReceiverHandshake.acceptHello(
                new PairingProtocol.BindingMaterial(hostId, receiverId, secret), hello);
        byte[] challenge = receiver.challenge();
        PairingProtocol.HostAwaitReady awaiting = host.acceptChallenge(challenge);
        PairingProtocol.FinishResult finished = receiver.acceptFinish(awaiting.finish());
        PairingProtocol.AuthenticatedSession hostSession = awaiting.acceptReady(finished.ready);
        try (hostSession; finished.session) {
            check(hostSession.sessionId == finished.session.sessionId
                            && hostSession.parameters.equals(finished.session.parameters),
                    "both peers authenticated different session parameters");
            PairingProtocol.SessionKeys hostKeys = hostSession.takeKeys();
            PairingProtocol.SessionKeys receiverKeys = finished.session.takeKeys();
            check(Arrays.equals(hostKeys.hostToReceiverAudio, receiverKeys.hostToReceiverAudio)
                            && Arrays.equals(hostKeys.receiverToHostAudio, receiverKeys.receiverToHostAudio)
                            && Arrays.equals(hostKeys.hostToReceiverControl, receiverKeys.hostToReceiverControl)
                            && Arrays.equals(hostKeys.receiverToHostControl, receiverKeys.receiverToHostControl),
                    "both peers derived different session keys");
            check(!Arrays.equals(hostKeys.hostToReceiverAudio, hostKeys.hostToReceiverControl),
                    "audio and control purposes shared a key");
            PairingSession result = new PairingSession(hostSession.sessionId, hostKeys, hello, challenge);
            receiverKeys.close();
            return result;
        }
    }

    private static void expectPairingFailure(ThrowingOperation operation, String message) throws Exception {
        try { operation.run(); throw new AssertionError(message); }
        catch (PairingProtocol.Failure expected) { }
    }

    private interface ThrowingOperation { void run() throws Exception; }

    private static final class PairingSession implements AutoCloseable {
        final long sessionId;
        final PairingProtocol.SessionKeys keys;
        final byte[] hostNonce;
        final byte[] receiverNonce;

        PairingSession(long sessionId, PairingProtocol.SessionKeys keys, byte[] hello, byte[] challenge) {
            this.sessionId = sessionId;
            this.keys = keys;
            hostNonce = Arrays.copyOfRange(hello, 38, 70);
            receiverNonce = Arrays.copyOfRange(challenge, 22, 54);
        }

        @Override public void close() { keys.close(); }
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

    private static void checkFormatSequenceRejected() throws Exception {
        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        byte[] key = filledKey(0x6b);
        long sessionId = 0x1020304050607080L;
        try (DatagramSocket sender = new DatagramSocket(new InetSocketAddress(loopback, 0))) {
            UdpSessionContext context = new UdpSessionContext(
                    new InetSocketAddress(loopback, 0), (InetSocketAddress) sender.getLocalSocketAddress(),
                    sessionId, key, "balanced");
            UdpPcmQueue queue = new UdpPcmQueue();
            try (UdpReceiver receiver = new UdpReceiver(context, queue)) {
                byte[] invalidFormat = encode(key, sessionId, 1, 0, UdpPacketDecoder.FORMAT,
                        formatPayload(48_000, 2));
                check(accept(receiver, invalidFormat, sender) == UdpReceiver.ProcessResult.FORMAT_REJECTED,
                        "nonzero Format sequence was accepted");
                check(queue.format() == null, "invalid Format changed the negotiated format");

                byte[] validFormat = encode(key, sessionId, 0, 0, UdpPacketDecoder.FORMAT,
                        formatPayload(48_000, 2));
                check(accept(receiver, validFormat, sender) == UdpReceiver.ProcessResult.FORMAT,
                        "rejected Format sequence committed replay state");
                check(queue.format().sampleRate == 48_000 && queue.format().channels == 2,
                        "valid Format after rejected sequence was not negotiated");
            } finally {
                context.destroy();
                Arrays.fill(key, (byte) 0);
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

    private static byte[] sequence(int first, int length) {
        byte[] bytes = new byte[length];
        for (int i = 0; i < length; i++) bytes[i] = (byte) (first + i);
        return bytes;
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
