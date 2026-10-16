package app.anpod.receiver;

import com.anpod.server.Pcm16FrameStream;
import java.io.DataInputStream;
import java.io.ByteArrayInputStream;
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
        checkJitterQueueExpiryAndFade();
        checkAdaptiveJitterTimeline();
        checkUdpLossConcealment();
        checkMonoPlcFrameAlignment();
        checkPlaybackActivityReset();
        checkFormatSequenceRejected();
        checkLoopbackReceiveThread();
        checkPairingSecurity();
        System.out.println("Android UDP parser, pairing, replay and loopback checks passed.");
    }

    private static void checkPlaybackActivityReset() {
        SyntheticClock clock = new SyntheticClock();
        UdpPlaybackActivity activity = new UdpPlaybackActivity(clock);
        long now = 100;
        clock.set(now);
        activity.resetPlaybackHead(100);
        activity.noteSuccessfulWrite(now + 100);
        clock.set(now + 100);
        check(activity.playedFrames(110) == 10,
                "playback-head observer did not record the new position");
        check(activity.recentlyAdvanced(now, 110),
                "recentlyAdvanced rejected another observer's progress using an older caller timestamp");
        check(activity.isActive(now, 110, true, now + 100, false),
                "concurrent playback evidence newer than the caller timestamp was rejected");
        check(activity.playedFrames(110) == 10 && activity.recentlyAdvanced(now, 110),
                "playback-head observers disagreed about the same position");

        clock.set(now + 2_000_000_101L);
        check(!activity.isActive(now + 2_000_000_101L, 110, true, now + 100, false),
                "stale playback evidence remained active after the two-second window");
        check(!activity.recentlyAdvanced(now, 110),
                "recentlyAdvanced remained true after two seconds without head progress");
        activity.noteSuccessfulWrite(now + 2_000_000_102L);
        clock.set(now + 2_000_000_102L);
        check(activity.isActive(now + 2_000_000_102L, 111, true, now + 2_000_000_102L, false),
                "playback activity did not recover after fresh audio and head progress");

        activity.resetPlaybackHead(0);
        check(activity.playedFrames(0) == 0, "flush retained the previous playback-head count");
        check(!activity.isActive(now + 2_000_000_103L, 0, true, now + 2_000_000_103L, false),
                "flush retained stale successful-write or playback-advance evidence");
        check(!activity.isActive(now + 2_000_000_103L, 0, true, now + 2_000_000_103L, true),
                "pause did not suppress playback activity");
    }

    private static void checkPairingSecurity() throws Exception {
        PairingProtocol.BindingMaterial generatedOne = PairingProtocol.BindingMaterial.generate();
        PairingProtocol.BindingMaterial generatedTwo = PairingProtocol.BindingMaterial.generate();
        byte[] generatedRecordOne = generatedOne.encodedRecord();
        byte[] generatedRecordTwo = generatedTwo.encodedRecord();
        check(!Arrays.equals(generatedRecordOne, generatedRecordTwo), "new binding material was not random");
        PairingProtocol.BindingMaterial streamCopy = PairingProtocol.BindingMaterial.readRecord(
                new ByteArrayInputStream(generatedRecordOne));
        byte[] streamCopyRecord = streamCopy.encodedRecord();
        check(Arrays.equals(streamCopyRecord, generatedRecordOne), "binding stdin parser changed the record");
        Arrays.fill(streamCopyRecord, (byte) 0);
        streamCopy.close();
        expectBindingReadFailure(Arrays.copyOf(generatedRecordOne, generatedRecordOne.length - 1),
                "short binding stdin was accepted");
        byte[] longRecord = Arrays.copyOf(generatedRecordOne, generatedRecordOne.length + 1);
        longRecord[longRecord.length - 1] = 1;
        expectBindingReadFailure(longRecord, "long binding stdin was accepted");
        byte[] unknownVersion = generatedRecordOne.clone();
        unknownVersion[4]++;
        expectBindingReadFailure(unknownVersion, "unknown binding version was accepted");
        Arrays.fill(unknownVersion, (byte) 0);
        generatedOne.close();
        generatedTwo.close();
        Arrays.fill(generatedRecordOne, (byte) 0);
        Arrays.fill(generatedRecordTwo, (byte) 0);

        byte[] secret = sequence(0, 32);
        byte[] hostId = sequence(0x10, 16);
        byte[] receiverId = sequence(0x20, 16);
        PairingProtocol.BindingMaterial vectorBinding = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        byte[] vectorCommitment = vectorBinding.commitment();
        check(hex(vectorCommitment).equals("dcb02a268976be74a08d28d7124724dbeef4c130715d6544ad8b7eb74f79eda4"),
                "binding commitment differs from the Rust vector");
        Arrays.fill(vectorCommitment, (byte) 0);
        vectorBinding.close();
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

    private static void expectBindingReadFailure(byte[] record, String message) throws Exception {
        try {
            PairingProtocol.BindingMaterial unexpected = PairingProtocol.BindingMaterial.readRecord(
                    new ByteArrayInputStream(record));
            unexpected.close();
            throw new AssertionError(message);
        } catch (PairingProtocol.Failure expected) {
            // Exact length and version are rejected before any material is stored.
        } finally {
            Arrays.fill(record, (byte) 0);
        }
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
                byte[] sameFrameNewSequence = encode(key, sessionId, 5, 0, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x55));
                check(accept(receiver, sameFrameNewSequence, sender) == UdpReceiver.ProcessResult.AUDIO
                        && queue.size() == 3, "same-frame PCM with a fresh sequence was queued twice");
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
                byte[] playedFrameDuplicate = encode(key, sessionId, 7, 0, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x77));
                check(accept(receiver, playedFrameDuplicate, sender) == UdpReceiver.ProcessResult.AUDIO
                        && queue.size() == 0, "already-played PCM frame was queued again");

                byte[] authenticatedRetry = encode(key, sessionId, 4, 720, UdpPacketDecoder.AUDIO, pcm(240));
                byte[] badTag = authenticatedRetry.clone();
                badTag[badTag.length - 1] ^= 0x40;
                check(accept(receiver, badTag, sender) == UdpReceiver.ProcessResult.AUTHENTICATION,
                        "tampered tag was accepted");
                check(queue.size() == 0, "bad tag changed the audio queue");
                check(accept(receiver, authenticatedRetry, sender) == UdpReceiver.ProcessResult.AUDIO,
                        "bad tag committed the replay sequence");

                byte[] invalidFormat = encode(key, sessionId, 6, 960, UdpPacketDecoder.AUDIO, new byte[] {1, 2, 3});
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
                byte[] pausedOne = encode(key, sessionId, 8, 960, UdpPacketDecoder.AUDIO, pcm(240));
                byte[] pausedTwo = encode(key, sessionId, 9, 1200, UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, pausedOne, sender) == UdpReceiver.ProcessResult.AUDIO
                        && accept(receiver, pausedTwo, sender) == UdpReceiver.ProcessResult.AUDIO,
                        "valid packets were rejected while paused");
                check(queue.size() == 0, "paused session accumulated audio");
                queue.setPaused(false);
                byte[] delayedPauseAudio = encode(key, sessionId, 10, 960, UdpPacketDecoder.AUDIO, pcm(240, 0x66));
                check(accept(receiver, delayedPauseAudio, sender) == UdpReceiver.ProcessResult.AUDIO
                        && queue.size() == 0, "delayed audio from the pause interval survived resume");
                byte[] stale = encode(key, sessionId, 11, 720, UdpPacketDecoder.AUDIO, pcm(240));
                check(accept(receiver, stale, sender) == UdpReceiver.ProcessResult.AUDIO && queue.size() == 0,
                        "pre-pause audio was replayed after resume");

                byte[] newest = encode(key, sessionId, 12, 1440, UdpPacketDecoder.AUDIO, pcm(240, 0x44));
                check(accept(receiver, newest, sender) == UdpReceiver.ProcessResult.AUDIO, "fresh audio rejected");
                byte[] resumedPayload = new byte[800];
                check(pauseInput.read(resumedPayload) == resumedPayload.length && (resumedPayload[720] & 0xff) == 0x44,
                        "the partially consumed pre-pause payload continued after resume");

                byte[] oldWindowPacket = null;
                for (int sequence = 13; sequence <= 22; sequence++) {
                    byte[] packet = encode(key, sessionId, sequence, (sequence - 6L) * 240,
                            UdpPacketDecoder.AUDIO, pcm(240));
                    if (sequence == 22) oldWindowPacket = packet;
                    check(accept(receiver, packet, sender) == UdpReceiver.ProcessResult.AUDIO,
                            "in-window bounded-queue packet rejected");
                }
                check(queue.size() == 10 && queue.size() <= UdpPcmQueue.CAPACITY_PACKETS,
                        "audio queue exceeded its fixed bound or dropped in-budget audio");
                check(queue.take().frameIndex == 1680, "bounded queue retained stale history");
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

    private static void checkJitterQueueExpiryAndFade() throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        UdpPcmQueue lowLatency = new UdpPcmQueue(parameters, "lowlatency");
        UdpPcmQueue balanced = new UdpPcmQueue(parameters, "balanced");
        check(lowLatency.targetFrames() < balanced.targetFrames(),
                "low-latency queue budget must start shorter than balanced");
        check(lowLatency.targetFrames() == 48_000 * 20 / 1000
                && balanced.targetFrames() == 48_000 * 40 / 1000,
                "wireless queue modes did not start at their configured targets");

        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        byte[] key = filledKey(0x39);
        long sessionId = 0x1122334455667788L;
        try (DatagramSocket sender = new DatagramSocket(new InetSocketAddress(loopback, 0))) {
            UdpSessionContext context = new UdpSessionContext(
                    new InetSocketAddress(loopback, 0), (InetSocketAddress) sender.getLocalSocketAddress(),
                    sessionId, key, "balanced");
            UdpPcmQueue queue = new UdpPcmQueue(parameters, "balanced");
            try (UdpReceiver receiver = new UdpReceiver(context, queue)) {
                check(accept(receiver, encode(key, sessionId, 0, 0, UdpPacketDecoder.FORMAT,
                        formatPayload(48_000, 2)), sender) == UdpReceiver.ProcessResult.FORMAT,
                        "jitter scenario format was rejected");
                queue.configureAudioTrack(0);
                InputStream input = queue.asPcmInputStream();
                byte[] header = new byte[8];
                new DataInputStream(input).readFully(header);
                Pcm16FrameStream pcm = new Pcm16FrameStream(2);
                byte[] output = new byte[960];

                check(accept(receiver, encode(key, sessionId, 1, 0, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x1234)), sender) == UdpReceiver.ProcessResult.AUDIO, "first PCM rejected");
                check(accept(receiver, encode(key, sessionId, 3, 480, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x3456)), sender) == UdpReceiver.ProcessResult.AUDIO, "out-of-order PCM rejected");
                check(accept(receiver, encode(key, sessionId, 2, 240, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x2345)), sender) == UdpReceiver.ProcessResult.AUDIO, "reordered PCM rejected");
                for (int sample = 0x1234; sample <= 0x3456; sample += 0x1111) {
                    check(pcm.readFrames(input, output, output.length) == output.length,
                            "short-reorder playback lost a packet");
                    int actual = (output[0] & 0xff) | (output[1] << 8);
                    check(actual == sample, "short-reorder playback was not in frame order");
                }

                check(accept(receiver, encode(key, sessionId, 4, 720, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x4567)), sender) == UdpReceiver.ProcessResult.AUDIO, "partial payload rejected");
                byte[] partial = new byte[64];
                check(input.read(partial) == partial.length, "could not hold a partial packet payload");
                check(accept(receiver, encode(key, sessionId, 5, 960, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x5678)), sender) == UdpReceiver.ProcessResult.AUDIO, "queued stale PCM rejected");
                check(accept(receiver, encode(key, sessionId, 6, 1200, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x6789)), sender) == UdpReceiver.ProcessResult.AUDIO, "queued stale PCM rejected");
                queue.expireStalePackets(System.nanoTime() + 200_000_000L);
                check(queue.expiredFrames() == 480, "local-age expiry did not account the queued backlog");
                check(accept(receiver, encode(key, sessionId, 7, 1440, UdpPacketDecoder.AUDIO,
                        pcm(240, 0x7fff)), sender) == UdpReceiver.ProcessResult.AUDIO, "fresh PCM rejected after expiry");
                check(pcm.readFrames(input, output, output.length) == output.length,
                        "fresh PCM did not resume after expired backlog");
                int faded = (output[0] & 0xff) | (output[1] << 8);
                check(faded > 0 && faded < 0x7fff,
                        "discontinuity was not faded at the start of a complete PCM frame");
                check(queue.droppedFrames() >= 704, "expired backlog did not invalidate the held payload tail");

                queue.setPaused(true);
                CountDownLatch waiting = new CountDownLatch(1);
                AtomicReference<Integer> stoppedRead = new AtomicReference<>();
                Thread blockedReader = new Thread(() -> {
                    waiting.countDown();
                    try { stoppedRead.set(input.read()); }
                    catch (Exception error) { throw new AssertionError(error); }
                }, "UDP queue stop wakeup check");
                blockedReader.start();
                check(waiting.await(1, TimeUnit.SECONDS), "paused queue reader did not start");
                Thread.sleep(5);
                check(blockedReader.getState() == Thread.State.WAITING,
                        "paused empty queue reader was not waiting without polling");
                queue.stop();
                blockedReader.join(1_000);
                check(!blockedReader.isAlive() && Integer.valueOf(-1).equals(stoppedRead.get()),
                        "stopping the paused queue did not wake its reader");

                checkLatePacketDoesNotEvictQueuedAudio(key, parameters);
            } finally {
                context.destroy();
                Arrays.fill(key, (byte) 0);
            }
        }
    }

    private static void checkAdaptiveJitterTimeline() throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        byte[] key = filledKey(0x48);
        long sessionId = 0x3344556677889900L;
        SyntheticClock clock = new SyntheticClock();
        UdpPcmQueue lowLatency = new UdpPcmQueue(parameters, "lowlatency", clock);
        UdpPacketDecoder decoder = new UdpPacketDecoder(sessionId, key);
        try {
            acceptAuthenticatedAt(lowLatency, decoder, key, sessionId, 0, 0,
                    UdpPacketDecoder.FORMAT, formatPayload(48_000, 2), clock.nowNanos());
            lowLatency.configureAudioTrack(0);
            long arrival = clock.nowNanos();
            for (int packet = 1; packet <= 20; packet++) {
                arrival += 5_000_000L;
                acceptAuthenticatedAt(lowLatency, decoder, key, sessionId, packet, packet * 240L,
                        UdpPacketDecoder.AUDIO, pcm(240, packet), arrival);
                lowLatency.take();
            }
            check(lowLatency.targetFrames() == 48_000 * 20 / 1000,
                    "steady 5ms arrivals raised the low-latency target");

            arrival += 25_000_000L;
            acceptAuthenticatedAt(lowLatency, decoder, key, sessionId, 21, 21 * 240L,
                    UdpPacketDecoder.AUDIO, pcm(240, 21), arrival);
            lowLatency.take();
            check(lowLatency.targetFrames() > 48_000 * 20 / 1000,
                    "a 20ms arrival burst did not grow the target");
            arrival += 100_000_000L;
            acceptAuthenticatedAt(lowLatency, decoder, key, sessionId, 22, 22 * 240L,
                    UdpPacketDecoder.AUDIO, pcm(240, 22), arrival);
            lowLatency.take();
            check(lowLatency.targetFrames() > 48_000 * 20 / 1000,
                    "a 100ms arrival burst did not grow the target");
            int grownTarget = lowLatency.targetFrames();

            // A higher sequence with an old frame index is a reordered packet and must not
            // perturb the arrival estimator. A duplicate is rejected by the decoder replay window.
            arrival += 5_000_000L;
            acceptAuthenticatedAt(lowLatency, decoder, key, sessionId, 23, 20 * 240L,
                    UdpPacketDecoder.AUDIO, pcm(240, 23), arrival);
            check(lowLatency.targetFrames() == grownTarget,
                    "reordered audio contaminated the jitter estimator");
            byte[] duplicate = encode(key, sessionId, 22, 22 * 240L,
                    UdpPacketDecoder.AUDIO, pcm(240, 22));
            try {
                lowLatency.acceptAt(decoder.open(duplicate, duplicate.length), decoder, arrival + 1_000_000L);
                throw new AssertionError("duplicate jitter sample was accepted");
            } catch (UdpPacketDecoder.ProtocolException expected) {
                check(expected.reason == UdpPacketDecoder.ProtocolException.REPLAY,
                        "duplicate packet failed for an unexpected reason");
            }
            check(lowLatency.targetFrames() == grownTarget,
                    "duplicate audio changed the jitter target");
            byte[] invalid = encode(key, sessionId, 24, 23 * 240L,
                    UdpPacketDecoder.AUDIO, pcm(240, 24));
            invalid[invalid.length - 1] ^= 1;
            try {
                lowLatency.acceptAt(decoder.open(invalid, invalid.length), decoder, arrival + 5_000_000L);
                throw new AssertionError("unauthenticated jitter sample was accepted");
            } catch (UdpPacketDecoder.ProtocolException expected) {
                check(expected.reason == UdpPacketDecoder.ProtocolException.AUTHENTICATION,
                        "invalid packet failed for an unexpected reason");
            }
            check(lowLatency.targetFrames() == grownTarget,
                    "unauthenticated audio changed the jitter target");

            clock.set(arrival);
            lowLatency.reportTrackUnderruns(0);
            lowLatency.reportTrackUnderruns(1);
            check(lowLatency.targetFrames() >= grownTarget + 48_000 * 10 / 1000,
                    "track underrun did not grow the target by at least 10ms");
            int beforeDecay = lowLatency.targetFrames();
            long stableArrival = arrival;
            long stableFrame = 22 * 240L;
            for (int packet = 25; packet <= 35; packet++) {
                stableArrival += 1_000_000_000L;
                stableFrame += 48_000;
                acceptAuthenticatedAt(lowLatency, decoder, key, sessionId, packet, stableFrame,
                        UdpPacketDecoder.AUDIO, pcm(240, packet), stableArrival);
                lowLatency.take();
            }
            clock.set(stableArrival);
            lowLatency.decayTarget(clock.nowNanos());
            check(lowLatency.targetFrames() == beforeDecay - 48_000 * 5 / 1000,
                    "stable target decay did not wait 10 seconds or step down by 5ms");

            SyntheticClock balancedClock = new SyntheticClock();
            UdpPcmQueue balanced = new UdpPcmQueue(parameters, "balanced", balancedClock);
            UdpPacketDecoder balancedDecoder = new UdpPacketDecoder(sessionId + 1, key);
            try {
                acceptAuthenticatedAt(balanced, balancedDecoder, key, sessionId + 1, 0, 0,
                        UdpPacketDecoder.FORMAT, formatPayload(48_000, 2), 0);
                balanced.configureAudioTrack(0);
                long time = 0;
                for (int packet = 1; packet <= 40; packet++) {
                    time += 105_000_000L;
                    acceptAuthenticatedAt(balanced, balancedDecoder, key, sessionId + 1,
                            packet, packet * 240L, UdpPacketDecoder.AUDIO, pcm(240, packet), time);
                    balanced.take();
                }
                check(balanced.targetFrames() == 48_000 * 150 / 1000,
                        "balanced queue did not clamp repeated bursts to its 150ms target cap");
            } finally {
                balanced.stop();
                balancedDecoder.destroy();
            }

            SyntheticClock jumpClock = new SyntheticClock();
            UdpPcmQueue largeFrameJump = new UdpPcmQueue(parameters, "lowlatency", jumpClock);
            UdpPacketDecoder jumpDecoder = new UdpPacketDecoder(sessionId + 3, key);
            try {
                acceptAuthenticatedAt(largeFrameJump, jumpDecoder, key, sessionId + 3, 0, 0,
                        UdpPacketDecoder.FORMAT, formatPayload(48_000, 2), 0);
                acceptAuthenticatedAt(largeFrameJump, jumpDecoder, key, sessionId + 3, 1, 0,
                        UdpPacketDecoder.AUDIO, pcm(240, 1), 0);
                largeFrameJump.take();
                acceptAuthenticatedAt(largeFrameJump, jumpDecoder, key, sessionId + 3, 2, Long.MIN_VALUE,
                        UdpPacketDecoder.AUDIO, pcm(240, 2), 5_000_000L);
                check(largeFrameJump.targetFrames() == 48_000 * 60 / 1000,
                        "large unsigned frame jump overflowed the jitter estimate or exceeded its cap");
            } finally {
                largeFrameJump.stop();
                jumpDecoder.destroy();
            }

            SyntheticClock capClock = new SyntheticClock();
            UdpPcmQueue bounded = new UdpPcmQueue(parameters, "balanced", capClock);
            UdpPacketDecoder boundedDecoder = new UdpPacketDecoder(sessionId + 2, key);
            try {
                acceptAuthenticatedAt(bounded, boundedDecoder, key, sessionId + 2, 0, 0,
                        UdpPacketDecoder.FORMAT, formatPayload(48_000, 2), 0);
                bounded.configureAudioTrack(0);
                for (int packet = 1; packet <= 40; packet++)
                    acceptAuthenticatedAt(bounded, boundedDecoder, key, sessionId + 2,
                            packet, packet * 240L, UdpPacketDecoder.AUDIO, pcm(240, packet), 0);
                check(bounded.size() == UdpPcmQueue.CAPACITY_PACKETS
                        && bounded.queuedFrames() == 48_000 * 160 / 1000,
                        "queue exceeded its physical packet or frame cap");
                bounded.expireStalePackets(161_000_000L);
                check(bounded.size() == 0 && bounded.expiredFrames() > 0,
                        "aged queued audio survived the 160ms physical age cap");
                acceptAuthenticatedAt(bounded, boundedDecoder, key, sessionId + 2, 41, 41 * 240L,
                        UdpPacketDecoder.AUDIO, pcm(240, 41), 161_000_000L);
                check(bounded.size() == 1, "fresh audio failed to recover after age expiry");
                bounded.setPaused(true);
                check(bounded.size() == 0 && bounded.queuedFrames() == 0,
                        "pause retained queued PCM");
                bounded.setPaused(false);
                acceptAuthenticatedAt(bounded, boundedDecoder, key, sessionId + 2, 42, 42 * 240L,
                        UdpPacketDecoder.AUDIO, pcm(240, 42), 162_000_000L);
                check(bounded.size() == 1, "fresh PCM did not resume after pause");
                bounded.stop();
                check(bounded.size() == 0 && bounded.queuedFrames() == 0,
                        "stop retained queued PCM");
            } finally {
                bounded.stop();
                boundedDecoder.destroy();
            }
        } finally {
            lowLatency.stop();
            decoder.destroy();
            Arrays.fill(key, (byte) 0);
        }
    }

    private static final class SyntheticClock implements UdpPcmQueue.NanoClock {
        private long nowNanos;
        @Override public long nowNanos() { return nowNanos; }
        void set(long value) { nowNanos = value; }
    }

    private static void checkUdpLossConcealment() throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        byte[] key = filledKey(0x69);
        long baseSession = 0x4455667788990011L;

        SyntheticClock singleClock = new SyntheticClock();
        UdpPcmQueue single = new UdpPcmQueue(parameters, "lowlatency", singleClock);
        UdpPacketDecoder singleDecoder = new UdpPacketDecoder(baseSession, key);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(single, singleDecoder, key,
                    baseSession, singleClock, 1, 12_000, -12_000);
            acceptAuthenticatedAt(single, singleDecoder, key, baseSession, 5, 1_440,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 8_000, -8_000), 25_000_000L);
            setAfterGapDeadline(singleClock, single, 25_000_000L);
            byte[] output = readInChunks(fixture.input, 480 * 4, 3);
            check(Math.abs(readSample(output, 0, 0) - 12_000) < 100
                    && Math.abs(readSample(output, 0, 1) + 12_000) < 100,
                    "single-loss PLC did not repeat the previous stereo frame");
            check(Math.abs(readSample(output, 119, 0)) < Math.abs(readSample(output, 0, 0)),
                    "single-loss PLC energy did not fade toward zero");
            check(readSample(output, 239, 0) == 0 && readSample(output, 239, 1) == 0,
                    "single-loss PLC did not fade its last repeated frame to zero");
            check(readSample(output, 240, 0) != 0 && readSample(output, 240, 1) != 0,
                    "real audio after PLC did not use the existing fade-in");
            check(single.plcEvents() == 1 && single.plcRepeatedFrames() == 240
                    && single.plcSilentFrames() == 0 && single.plcOversizedGapSkips() == 0,
                    "single-loss PLC counters were incorrect");
            acceptAuthenticatedAt(single, singleDecoder, key, baseSession, 6, 1_200,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 1, 2), 46_000_000L);
            check(single.size() == 0, "late packet behind the PLC watermark was retained");
        } finally {
            single.stop();
            singleDecoder.destroy();
        }

        SyntheticClock burstClock = new SyntheticClock();
        UdpPcmQueue burst = new UdpPcmQueue(parameters, "lowlatency", burstClock);
        UdpPacketDecoder burstDecoder = new UdpPacketDecoder(baseSession + 1, key);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(burst, burstDecoder, key,
                    baseSession + 1, burstClock, 1, 9_000, -9_000);
            acceptAuthenticatedAt(burst, burstDecoder, key, baseSession + 1, 5, 1_680,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 8_000, -8_000), 25_000_000L);
            setAfterGapDeadline(burstClock, burst, 25_000_000L);
            byte[] output = readInChunks(fixture.input, 720 * 4, 5);
            check(Math.abs(readSample(output, 0, 0) - 9_000) < 100
                    && Math.abs(readSample(output, 0, 1) + 9_000) < 100,
                    "consecutive-loss PLC did not repeat one prior payload");
            check(Math.abs(readSample(output, 119, 0)) < Math.abs(readSample(output, 0, 0)),
                    "consecutive-loss PLC energy did not fade before silence");
            check(readSample(output, 239, 0) == 0 && readSample(output, 240, 0) == 0
                    && readSample(output, 479, 1) == 0,
                    "consecutive-loss PLC did not fade then silence the remainder");
            check(burst.plcEvents() == 1 && burst.plcRepeatedFrames() == 240
                    && burst.plcSilentFrames() == 240,
                    "consecutive-loss PLC counters were incorrect");
        } finally {
            burst.stop();
            burstDecoder.destroy();
        }

        SyntheticClock silenceClock = new SyntheticClock();
        UdpPcmQueue silence = new UdpPcmQueue(parameters, "lowlatency", silenceClock);
        UdpPacketDecoder silenceDecoder = new UdpPacketDecoder(baseSession + 2, key);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(silence, silenceDecoder, key,
                    baseSession + 2, silenceClock, 1, 0, 0);
            acceptAuthenticatedAt(silence, silenceDecoder, key, baseSession + 2, 5, 1_440,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 2_000, -2_000), 25_000_000L);
            setAfterGapDeadline(silenceClock, silence, 25_000_000L);
            byte[] output = readInChunks(fixture.input, 480 * 4, 7);
            check(allZero(output, 0, 240 * 4), "silence input produced nonzero PLC samples");
            check(silence.plcRepeatedFrames() == 240 && silence.plcSilentFrames() == 0,
                    "silent real PCM was not retained as the previous genuine payload");
        } finally {
            silence.stop();
            silenceDecoder.destroy();
        }

        SyntheticClock reorderClock = new SyntheticClock();
        UdpPcmQueue reorder = new UdpPcmQueue(parameters, "lowlatency", reorderClock);
        UdpPacketDecoder reorderDecoder = new UdpPacketDecoder(baseSession + 3, key);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(reorder, reorderDecoder, key,
                    baseSession + 3, reorderClock, 1, 4_000, -4_000);
            acceptAuthenticatedAt(reorder, reorderDecoder, key, baseSession + 3, 5, 1_440,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 8_000, -8_000), 25_000_000L);
            acceptAuthenticatedAt(reorder, reorderDecoder, key, baseSession + 3, 6, 1_200,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 6_000, -6_000), 30_000_000L);
            reorderClock.set(45_000_000L);
            byte[] output = readInChunks(fixture.input, 480 * 4, 9);
            check(readSample(output, 0, 0) == 6_000 && readSample(output, 240, 0) == 8_000,
                    "reordered missing packet was not played before the future packet");
            check(reorder.plcEvents() == 0 && reorder.plcRepeatedFrames() == 0
                    && reorder.plcSilentFrames() == 0,
                    "reordered packet incorrectly triggered PLC");
        } finally {
            reorder.stop();
            reorderDecoder.destroy();
        }

        SyntheticClock noHistoryClock = new SyntheticClock();
        UdpPcmQueue noHistory = new UdpPcmQueue(parameters, "lowlatency", noHistoryClock);
        UdpPacketDecoder noHistoryDecoder = new UdpPacketDecoder(baseSession + 4, key);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(noHistory, noHistoryDecoder, key,
                    baseSession + 4, noHistoryClock, 1, 5_000, -5_000);
            noHistory.setPaused(true);
            noHistory.setPaused(false);
            acceptAuthenticatedAt(noHistory, noHistoryDecoder, key, baseSession + 4, 5, 1_440,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 8_000, -8_000), 25_000_000L);
            for (int packet = 6; packet <= 8; packet++)
                acceptAuthenticatedAt(noHistory, noHistoryDecoder, key, baseSession + 4,
                        packet, (packet + 1) * 240L, UdpPacketDecoder.AUDIO,
                        stereoPcm(240, 8_000, -8_000), packet * 5_000_000L);
            setAfterGapDeadline(noHistoryClock, noHistory, 25_000_000L);
            byte[] output = readInChunks(fixture.input, 480 * 4, 11);
            check(allZero(output, 0, 240 * 4), "PLC reused genuine PCM invalidated by pause");
            check(noHistory.plcRepeatedFrames() == 0 && noHistory.plcSilentFrames() == 240,
                    "PLC without valid history did not count the whole gap as silence");
        } finally {
            noHistory.stop();
            noHistoryDecoder.destroy();
            Arrays.fill(key, (byte) 0);
        }

        SyntheticClock expiredHistoryClock = new SyntheticClock();
        UdpPcmQueue expiredHistory = new UdpPcmQueue(parameters, "lowlatency", expiredHistoryClock);
        byte[] expiredHistoryKey = filledKey(0x69);
        UdpPacketDecoder expiredHistoryDecoder = new UdpPacketDecoder(baseSession + 6, expiredHistoryKey);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(expiredHistory,
                    expiredHistoryDecoder, expiredHistoryKey, baseSession + 6,
                    expiredHistoryClock, 1, 7_000, -7_000);
            acceptAuthenticatedAt(expiredHistory, expiredHistoryDecoder, expiredHistoryKey,
                    baseSession + 6, 5, 1_200, UdpPacketDecoder.AUDIO,
                    stereoPcm(240, 6_000, -6_000), 25_000_000L);
            expiredHistory.expireStalePackets(96_000_000L);
            check(expiredHistory.size() == 0 && expiredHistory.expiredFrames() == 240,
                    "queued packet expiry did not create the expected discontinuity");
            acceptAuthenticatedAt(expiredHistory, expiredHistoryDecoder, expiredHistoryKey,
                    baseSession + 6, 6, 1_680, UdpPacketDecoder.AUDIO,
                    stereoPcm(240, 8_000, -8_000), 96_000_000L);
            setAfterGapDeadline(expiredHistoryClock, expiredHistory, 96_000_000L);
            byte[] output = readInChunks(fixture.input, 480 * 4, 3);
            check(allZero(output, 0, 240 * 4),
                    "PLC reused real audio from before an expired-packet discontinuity");
            check(expiredHistory.plcRepeatedFrames() == 0
                    && expiredHistory.plcSilentFrames() == 240,
                    "expired-packet discontinuity did not clear PLC history");
        } finally {
            expiredHistory.stop();
            expiredHistoryDecoder.destroy();
            Arrays.fill(expiredHistoryKey, (byte) 0);
        }

        SyntheticClock jumpClock = new SyntheticClock();
        UdpPcmQueue jump = new UdpPcmQueue(parameters, "lowlatency", jumpClock);
        byte[] jumpKey = filledKey(0x69);
        UdpPacketDecoder jumpDecoder = new UdpPacketDecoder(baseSession + 5, jumpKey);
        try {
            QueueInputStreamFixture fixture = startConcealmentQueue(jump, jumpDecoder, jumpKey,
                    baseSession + 5, jumpClock, 1, 3_000, -3_000);
            acceptAuthenticatedAt(jump, jumpDecoder, jumpKey, baseSession + 5, 5, Long.MIN_VALUE,
                    UdpPacketDecoder.AUDIO, stereoPcm(240, 7_000, -7_000), 25_000_000L);
            setAfterGapDeadline(jumpClock, jump, 25_000_000L);
            byte[] output = readInChunks(fixture.input, 240 * 4, 13);
            check(output.length == 240 * 4 && readSample(output, 0, 0) != 0,
                    "oversized unsigned gap did not immediately resume with real audio");
            check(jump.plcEvents() == 0 && jump.plcRepeatedFrames() == 0
                    && jump.plcSilentFrames() == 0 && jump.plcOversizedGapSkips() == 1,
                    "oversized unsigned gap was concealed or not counted");
        } finally {
            jump.stop();
            jumpDecoder.destroy();
            Arrays.fill(jumpKey, (byte) 0);
        }
    }

    private static void checkMonoPlcFrameAlignment() throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 1, 0);
        byte[] key = filledKey(0x6a);
        long sessionId = 0x5566778899001122L;
        SyntheticClock clock = new SyntheticClock();
        UdpPcmQueue queue = new UdpPcmQueue(parameters, "lowlatency", clock);
        UdpPacketDecoder decoder = new UdpPacketDecoder(sessionId, key);
        try {
            acceptAuthenticatedAt(queue, decoder, key, sessionId, 0, 0,
                    UdpPacketDecoder.FORMAT, formatPayload(48_000, 1), 0);
            queue.configureAudioTrack(0);
            for (int packet = 1; packet <= 4; packet++)
                acceptAuthenticatedAt(queue, decoder, key, sessionId, packet, packet * 240L,
                        UdpPacketDecoder.AUDIO, monoPcm(240, 10_000), packet * 5_000_000L);
            clock.set(20_000_000L);
            InputStream input = queue.asPcmInputStream();
            byte[] header = readInChunks(input, 8, 3);
            check(littleInt(header, 0) == 48_000 && littleShort(header, 4) == 1,
                    "mono queue stream header was incorrect");
            readInChunks(input, 960 * 2, 13);
            acceptAuthenticatedAt(queue, decoder, key, sessionId, 5, 1_440,
                    UdpPacketDecoder.AUDIO, monoPcm(240, 6_000), 25_000_000L);
            setAfterGapDeadline(clock, queue, 25_000_000L);
            byte[] output = readInChunks(input, 480 * 2, 5);
            check(output.length == 480 * 2 && readMonoSample(output, 0) != 0
                    && readMonoSample(output, 239) == 0 && readMonoSample(output, 240) != 0,
                    "mono PLC output lost PCM16 frame alignment");
            check(queue.plcEvents() == 1 && queue.plcRepeatedFrames() == 240
                    && queue.plcSilentFrames() == 0,
                    "mono PLC frame counters were incorrect");
        } finally {
            queue.stop();
            decoder.destroy();
            Arrays.fill(key, (byte) 0);
        }
    }

    private static QueueInputStreamFixture startConcealmentQueue(UdpPcmQueue queue,
            UdpPacketDecoder decoder, byte[] key, long sessionId, SyntheticClock clock,
            long firstSequence, int left, int right) throws Exception {
        acceptAuthenticatedAt(queue, decoder, key, sessionId, 0, 0,
                UdpPacketDecoder.FORMAT, formatPayload(48_000, 2), 0);
        queue.configureAudioTrack(0);
        for (int packet = 1; packet <= 4; packet++)
            acceptAuthenticatedAt(queue, decoder, key, sessionId, firstSequence + packet - 1,
                    packet * 240L, UdpPacketDecoder.AUDIO, stereoPcm(240, left, right),
                    packet * 5_000_000L);
        clock.set(20_000_000L);
        InputStream input = queue.asPcmInputStream();
        byte[] header = readInChunks(input, 8, 3);
        check(littleInt(header, 0) == 48_000 && littleShort(header, 4) == 2,
                "queue stream header was incorrect");
        readInChunks(input, 960 * 4, 17);
        return new QueueInputStreamFixture(input);
    }

    private static byte[] readInChunks(InputStream input, int length, int chunkSize) throws Exception {
        byte[] result = new byte[length];
        int offset = 0;
        byte[] chunk = new byte[Math.min(chunkSize, length)];
        while (offset < length) {
            int count = input.read(chunk, 0, Math.min(chunk.length, length - offset));
            if (count < 0) throw new AssertionError("queue stream ended before requested PCM frames");
            System.arraycopy(chunk, 0, result, offset, count);
            offset += count;
        }
        return result;
    }

    private static void setAfterGapDeadline(SyntheticClock clock, UdpPcmQueue queue,
            long packetArrivalNanos) {
        clock.set(packetArrivalNanos + queue.targetFrames() * 1_000_000_000L / 48_000);
    }

    private static byte[] stereoPcm(int frames, int left, int right) {
        byte[] payload = new byte[frames * 4];
        for (int frame = 0; frame < frames; frame++) {
            int offset = frame * 4;
            payload[offset] = (byte) left;
            payload[offset + 1] = (byte) (left >>> 8);
            payload[offset + 2] = (byte) right;
            payload[offset + 3] = (byte) (right >>> 8);
        }
        return payload;
    }

    private static byte[] monoPcm(int frames, int sample) {
        byte[] payload = new byte[frames * 2];
        for (int frame = 0; frame < frames; frame++) {
            payload[frame * 2] = (byte) sample;
            payload[frame * 2 + 1] = (byte) (sample >>> 8);
        }
        return payload;
    }

    private static int readMonoSample(byte[] pcm, int frame) {
        int offset = frame * 2;
        return (short) ((pcm[offset] & 0xff) | (pcm[offset + 1] << 8));
    }

    private static int readSample(byte[] pcm, int frame, int channel) {
        int offset = frame * 4 + channel * 2;
        return (short) ((pcm[offset] & 0xff) | (pcm[offset + 1] << 8));
    }

    private static boolean allZero(byte[] bytes, int offset, int length) {
        for (int index = offset; index < offset + length; index++)
            if (bytes[index] != 0) return false;
        return true;
    }

    private static final class QueueInputStreamFixture {
        final InputStream input;
        QueueInputStreamFixture(InputStream input) { this.input = input; }
    }

    private static void checkLatePacketDoesNotEvictQueuedAudio(byte[] key,
            PairingProtocol.AudioParameters parameters) throws Exception {
        long sessionId = 0x2233445566778899L;
        UdpPcmQueue queue = new UdpPcmQueue(parameters, "lowlatency");
        UdpPacketDecoder decoder = new UdpPacketDecoder(sessionId, key);
        long syntheticNow = System.nanoTime();
        try {
            acceptAuthenticatedAt(queue, decoder, key, sessionId, 0, 0,
                    UdpPacketDecoder.FORMAT, formatPayload(48_000, 2), syntheticNow);
            queue.configureAudioTrack(0);
            for (int packet = 1; packet <= 14; packet++) {
                acceptAuthenticatedAt(queue, decoder, key, sessionId, packet, packet * 240L,
                        UdpPacketDecoder.AUDIO, pcm(240, packet), syntheticNow);
            }
            long droppedBefore = queue.droppedFrames();
            acceptAuthenticatedAt(queue, decoder, key, sessionId, 15, 0,
                    UdpPacketDecoder.AUDIO, pcm(240, 0x55), syntheticNow);
            check(queue.size() == 14 && queue.droppedFrames() == droppedBefore + 240
                    && queue.take().frameIndex == 240,
                    "a late packet at the frame budget evicted newer queued audio");
        } finally {
            queue.stop();
            decoder.destroy();
        }
    }

    private static void acceptAuthenticatedAt(UdpPcmQueue queue, UdpPacketDecoder decoder,
            byte[] key, long sessionId, long sequence, long frameIndex, int type, byte[] payload,
            long receivedAtNanos) throws Exception {
        byte[] datagram = encode(key, sessionId, sequence, frameIndex, type, payload);
        queue.acceptAt(decoder.open(datagram, datagram.length), decoder, receivedAtNanos);
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
