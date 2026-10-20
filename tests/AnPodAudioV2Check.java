package app.anpod.receiver;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Map;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/** Independent Java checks for the offline v2 descriptor, handshake and datagram vectors. */
public final class AnPodAudioV2Check {
    private static Map<String, byte[]> vectors(Path path) throws Exception {
        Map<String, byte[]> result = new HashMap<>();
        for (String line : Files.readAllLines(path)) {
            if (line.isBlank() || line.startsWith("#")) continue;
            String[] fields = line.split("=", 2);
            result.put(fields[0], hex(fields[1]));
        }
        return result;
    }

    private static byte[] hex(String value) {
        byte[] bytes = new byte[value.length() / 2];
        for (int i = 0; i < bytes.length; i++) bytes[i] = (byte) Integer.parseInt(value.substring(i * 2, i * 2 + 2), 16);
        return bytes;
    }

    private static byte[] sequence(int start, int length) {
        byte[] bytes = new byte[length];
        for (int i = 0; i < length; i++) bytes[i] = (byte) (start + i);
        return bytes;
    }

    private static PairingProtocol.BindingMaterial material() {
        return new PairingProtocol.BindingMaterial(sequence(0x10, 16), sequence(0x20, 16), sequence(0, 32));
    }

    private static byte[] packet(byte[] key, int version, long session, long sequence, long frameIndex,
            int type, byte[] payload) throws Exception {
        byte[] header = ByteBuffer.allocate(34).order(ByteOrder.BIG_ENDIAN).put(new byte[] {'A','N','P','D'})
                .put((byte) version).put((byte) type).putShort((short) 0).putLong(session).putLong(sequence)
                .putLong(frameIndex).putShort((short) payload.length).array();
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN).putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] encrypted = cipher.doFinal(payload);
        byte[] result = Arrays.copyOf(header, header.length + encrypted.length);
        System.arraycopy(encrypted, 0, result, header.length, encrypted.length);
        return result;
    }

    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }

    public static void main(String[] args) throws Exception {
        Map<String, byte[]> v = vectors(Path.of(args[0]));
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(2, 0);
        byte[] descriptor = format.encode();
        check(Arrays.equals(descriptor, v.get("descriptor")), "descriptor vector");
        for (int profile = 1; profile <= 3; profile++) {
            for (int mode = 0; mode <= 1; mode++) {
                UdpPacketDecoder.AudioFormatV2 f = UdpPacketDecoder.AudioFormatV2.forProfile(profile, mode);
                check(Arrays.equals(UdpPacketDecoder.AudioFormatV2.decode(f.encode()).encode(), f.encode()), "descriptor roundtrip");
            }
        }
        for (int[] mutation : new int[][] {{0, 1}, {1, 9}, {2, 9}, {7, 1}, {14, 1}}) {
            byte[] invalidDescriptor = descriptor.clone(); invalidDescriptor[mutation[0]] = (byte) mutation[1];
            try { UdpPacketDecoder.AudioFormatV2.decode(invalidDescriptor); throw new AssertionError("invalid descriptor accepted"); }
            catch (IllegalArgumentException expected) { }
        }
        try { UdpPacketDecoder.AudioFormatV2.forProfile(4, 0); throw new AssertionError("unknown profile accepted"); }
        catch (IllegalArgumentException expected) { }
        byte[] hostId = sequence(0x10, 16), receiverId = sequence(0x20, 16), secret = sequence(0, 32);
        byte[] hostNonce = sequence(0x30, 32), receiverNonce = sequence(0x50, 32);
        long sessionId = 0x0102030405060708L;
        byte[] transcript = PairingProtocol.V2.transcriptForVector(hostId, receiverId, hostNonce,
                receiverNonce, sessionId, format);
        byte[] hostProof = PairingProtocol.V2.proofForVector(secret, "host", transcript);
        byte[] receiverProof = PairingProtocol.V2.proofForVector(secret, "receiver", transcript);
        byte[] readyProof = PairingProtocol.V2.readyForVector(secret, transcript, hostProof, receiverProof);
        check(Arrays.equals(transcript, v.get("transcript")), "transcript vector");
        check(Arrays.equals(hostProof, v.get("host_proof")), "host proof vector");
        check(Arrays.equals(receiverProof, v.get("receiver_proof")), "receiver proof vector");
        check(Arrays.equals(readyProof, v.get("ready_proof")), "ready proof vector");
        check(Arrays.equals(PairingProtocol.V2.helloForVector(hostId, receiverId, hostNonce, sessionId, format), v.get("hello")), "HELLO vector");
        check(Arrays.equals(PairingProtocol.V2.challengeForVector(receiverId, receiverNonce, format, receiverProof), v.get("challenge")), "CHALLENGE vector");
        check(Arrays.equals(PairingProtocol.V2.proofMessageForVector(3, hostProof), v.get("finish")), "FINISH vector");
        check(Arrays.equals(PairingProtocol.V2.proofMessageForVector(4, readyProof), v.get("ready")), "READY vector");
        PairingProtocol.SessionKeys vectorKeys = PairingProtocol.V2.deriveForVector(secret, transcript);
        check(Arrays.equals(vectorKeys.hostToReceiverAudio, v.get("key_h2r_audio")), "host audio key vector");
        check(Arrays.equals(vectorKeys.receiverToHostAudio, v.get("key_r2h_audio")), "receiver audio key vector");
        check(Arrays.equals(vectorKeys.hostToReceiverControl, v.get("key_h2r_control")), "host control key vector");
        check(Arrays.equals(vectorKeys.receiverToHostControl, v.get("key_r2h_control")), "receiver control key vector");
        vectorKeys.close();

        byte[] key = sequence(0, 32);
        byte[] packet = packet(key, 2, sessionId, 0x1112131415161718L, 0, UdpPacketDecoder.FORMAT, descriptor);
        check(Arrays.equals(Arrays.copyOf(packet, 34), v.get("aad")), "AAD vector");
        check(Arrays.equals(ByteBuffer.allocate(12).putInt(0x414e5048).putLong(0x1112131415161718L).array(), v.get("nonce")), "nonce vector");
        check(Arrays.equals(packet, v.get("format_packet")), "encrypted FORMAT vector");
        UdpPacketDecoder decoder = UdpPacketDecoder.newV2(sessionId, key, format);
        try { new UdpPacketDecoder(sessionId, key).open(packet, packet.length); throw new AssertionError("v1 accepted v2 packet"); }
        catch (UdpPacketDecoder.ProtocolException expected) { check(expected.reason == UdpPacketDecoder.ProtocolException.VERSION, "v1 rejects v2"); }
        UdpPacketDecoder.Packet decoded = decoder.open(packet, packet.length);
        check(Arrays.equals(decoded.payload, descriptor), "Java decrypts shared FORMAT packet");
        decoder.commit(decoded.sequence);
        decoder.destroy();

        byte[] v1Packet = packet(key, 1, sessionId, 8, 0, UdpPacketDecoder.FORMAT,
                new byte[] {1, 0, 2, 0, 0, (byte) 0xbb, (byte) 0x80});
        try { UdpPacketDecoder.newV2(sessionId, key, format).open(v1Packet, v1Packet.length); throw new AssertionError("v2 accepted v1 packet"); }
        catch (UdpPacketDecoder.ProtocolException expected) { check(expected.reason == UdpPacketDecoder.ProtocolException.VERSION, "v2 rejects v1"); }

        PairingProtocol.V2.HostStart start = PairingProtocol.V2.Host.start(material(), format);
        PairingProtocol.V2.ReceiverAwaitFinish receiver = PairingProtocol.V2.Receiver.acceptHello(material(), start.hello);
        PairingProtocol.V2.HostAwaitReady host = start.host.acceptChallenge(receiver.challenge);
        PairingProtocol.V2.FinishResult result = receiver.acceptFinish(host.finish);
        PairingProtocol.V2.AuthenticatedSessionV2 hostSession = host.acceptReady(result.ready);
        PairingProtocol.SessionKeys hostKeys = hostSession.takeKeys();
        PairingProtocol.SessionKeys receiverKeys = result.session.takeKeys();
        check(Arrays.equals(hostKeys.hostToReceiverAudio, receiverKeys.hostToReceiverAudio), "random handshake host key");
        check(Arrays.equals(hostKeys.receiverToHostControl, receiverKeys.receiverToHostControl), "random handshake control key");
        hostKeys.close(); receiverKeys.close();
        try { host.acceptReady(result.ready); throw new AssertionError("duplicate state accepted"); }
        catch (IllegalStateException expected) { }
        try { receiver.acceptFinish(host.finish); throw new AssertionError("duplicate receiver state accepted"); }
        catch (IllegalStateException expected) { }

        PairingProtocol.V2.HostStart cancelHostStart = PairingProtocol.V2.Host.start(material(), format);
        PairingProtocol.V2.ReceiverAwaitFinish cancelReceiver = PairingProtocol.V2.Receiver.acceptHello(material(), cancelHostStart.hello);
        PairingProtocol.V2.HostAwaitReady cancelHost = cancelHostStart.host.acceptChallenge(cancelReceiver.challenge);
        cancelHost.close();
        try { cancelHost.acceptReady(new byte[40]); throw new AssertionError("cancelled host state accepted"); }
        catch (IllegalStateException expected) { }
        cancelReceiver.close();

        PairingProtocol.V2.HostStart cancelReceiverStart = PairingProtocol.V2.Host.start(material(), format);
        PairingProtocol.V2.ReceiverAwaitFinish cancelledReceiver = PairingProtocol.V2.Receiver.acceptHello(material(), cancelReceiverStart.hello);
        cancelledReceiver.close();
        try { cancelledReceiver.acceptFinish(new byte[40]); throw new AssertionError("cancelled receiver state accepted"); }
        catch (IllegalStateException expected) { }
        cancelReceiverStart.host.close();

        PairingProtocol.V2.HostStart badStart = PairingProtocol.V2.Host.start(material(), format);
        byte[] badHello = badStart.hello.clone(); badHello[4] = 1;
        try { PairingProtocol.V2.Receiver.acceptHello(material(), badHello); throw new AssertionError("v1 HELLO accepted"); }
        catch (PairingProtocol.Failure expected) { }
        byte[] badLength = badStart.hello.clone(); badLength[7] = 95;
        try { PairingProtocol.V2.Receiver.acceptHello(material(), badLength); throw new AssertionError("bad length accepted"); }
        catch (PairingProtocol.Failure expected) { }
        byte[] badIdentity = badStart.hello.clone(); badIdentity[8] ^= 1;
        try { PairingProtocol.V2.Receiver.acceptHello(material(), badIdentity); throw new AssertionError("bad identity accepted"); }
        catch (PairingProtocol.Failure expected) { }

        PairingProtocol.V2.HostStart selectionStart = PairingProtocol.V2.Host.start(material(), format);
        PairingProtocol.V2.ReceiverAwaitFinish selectionReceiver = PairingProtocol.V2.Receiver.acceptHello(material(), selectionStart.hello);
        byte[] wrongSelection = selectionReceiver.challenge.clone();
        System.arraycopy(UdpPacketDecoder.AudioFormatV2.forProfile(1, 0).encode(), 0, wrongSelection, 56, 16);
        try { selectionStart.host.acceptChallenge(wrongSelection); throw new AssertionError("changed selection accepted"); }
        catch (PairingProtocol.Failure expected) { }
        selectionReceiver.close();

        PairingProtocol.V2.HostStart proofStart = PairingProtocol.V2.Host.start(material(), format);
        PairingProtocol.V2.ReceiverAwaitFinish proofReceiver = PairingProtocol.V2.Receiver.acceptHello(material(), proofStart.hello);
        byte[] badProof = proofReceiver.challenge.clone(); badProof[72] ^= 1;
        try { proofStart.host.acceptChallenge(badProof); throw new AssertionError("bad proof accepted"); }
        catch (PairingProtocol.Failure expected) { }
        proofReceiver.close();

        PairingProtocol.V2.HostStart keyStart = PairingProtocol.V2.Host.start(material(), format);
        byte[] wrongSecret = sequence(0, 32); wrongSecret[0] ^= 1;
        PairingProtocol.BindingMaterial wrongKeyMaterial = new PairingProtocol.BindingMaterial(
                sequence(0x10, 16), sequence(0x20, 16), wrongSecret);
        PairingProtocol.V2.ReceiverAwaitFinish wrongKeyReceiver = PairingProtocol.V2.Receiver.acceptHello(wrongKeyMaterial, keyStart.hello);
        try { keyStart.host.acceptChallenge(wrongKeyReceiver.challenge); throw new AssertionError("wrong secret accepted"); }
        catch (PairingProtocol.Failure expected) { }
        wrongKeyReceiver.close();

        PairingProtocol.V2.HostStart staleOne = PairingProtocol.V2.Host.start(material(), format);
        PairingProtocol.V2.ReceiverAwaitFinish staleReceiver = PairingProtocol.V2.Receiver.acceptHello(material(), staleOne.hello);
        PairingProtocol.V2.HostStart staleTwo = PairingProtocol.V2.Host.start(material(), format);
        try { staleTwo.host.acceptChallenge(staleReceiver.challenge); throw new AssertionError("old challenge accepted"); }
        catch (PairingProtocol.Failure expected) { }
        staleOne.host.close(); staleReceiver.close();

        byte[] audioKey = sequence(0x42, 32);
        long audioSession = 99;
        UdpPacketDecoder.AudioFormatV2 standard = UdpPacketDecoder.AudioFormatV2.forProfile(1, 0);
        byte[] mismatchedFormat = packet(audioKey, 2, audioSession, 3, 0, UdpPacketDecoder.FORMAT, standard.encode());
        UdpPacketDecoder v2decoder = UdpPacketDecoder.newV2(audioSession, audioKey, format);
        try { v2decoder.open(mismatchedFormat, mismatchedFormat.length); throw new AssertionError("valid-tag format mismatch accepted"); }
        catch (UdpPacketDecoder.ProtocolException expected) { check(expected.reason == UdpPacketDecoder.ProtocolException.FORMAT, "format mismatch reason"); }
        byte[] misalignedAudio = packet(audioKey, 2, audioSession, 4, 5, UdpPacketDecoder.AUDIO, new byte[] {1,2,3,4,5});
        try { v2decoder.open(misalignedAudio, misalignedAudio.length); throw new AssertionError("valid-tag misaligned audio accepted"); }
        catch (UdpPacketDecoder.ProtocolException expected) { check(expected.reason == UdpPacketDecoder.ProtocolException.FORMAT, "misaligned format reason"); }
        byte[] alignedSameSequence = packet(audioKey, 2, audioSession, 4, 5, UdpPacketDecoder.AUDIO, new byte[] {1,2,3,4,5,6});
        check(Arrays.equals(v2decoder.open(alignedSameSequence, alignedSameSequence.length).payload,
                new byte[] {1,2,3,4,5,6}), "format failure leaves sequence available");
        v2decoder.commit(4);
        byte[] goodAudio = packet(audioKey, 2, audioSession, 3, 5, UdpPacketDecoder.AUDIO, new byte[] {1,2,3,4,5,6});
        byte[] badAudio = goodAudio.clone(); badAudio[badAudio.length - 1] ^= 1;
        try { v2decoder.open(badAudio, badAudio.length); throw new AssertionError("bad tag accepted"); }
        catch (UdpPacketDecoder.ProtocolException expected) { check(expected.reason == UdpPacketDecoder.ProtocolException.AUTHENTICATION, "auth failure reason"); }
        UdpPacketDecoder.Packet accepted = v2decoder.open(goodAudio, goodAudio.length);
        v2decoder.commit(accepted.sequence);
        try { v2decoder.open(goodAudio, goodAudio.length); throw new AssertionError("replay accepted"); }
        catch (UdpPacketDecoder.ProtocolException expected) { check(expected.reason == UdpPacketDecoder.ProtocolException.REPLAY, "replay reason"); }
        byte[] maxAligned = new byte[1146];
        byte[] maxPacket = packet(audioKey, 2, audioSession, 6, 6, UdpPacketDecoder.AUDIO, maxAligned);
        UdpPacketDecoder maxDecoder = UdpPacketDecoder.newV2(audioSession, audioKey, format);
        check(maxDecoder.open(maxPacket, maxPacket.length).payload.length == 1146, "largest aligned datagram");
        maxDecoder.destroy();
        v2decoder.destroy();
        checkFloatUdpReceiverEdges(audioKey, audioSession, format);
        checkAoaIngress(audioKey, audioSession);
        System.out.println("Android audio v2 vector and handshake checks passed.");
    }

    private static void checkAoaIngress(byte[] key, long sessionId) throws Exception {
        UdpPacketDecoder.AudioFormatV2 standard = UdpPacketDecoder.AudioFormatV2.forProfile(1, 0);
        AoaSessionContext pcm = new AoaSessionContext(sessionId, key, standard);
        try {
            byte[] stereoFrame = { 0x34, 0x12, (byte) 0xcc, (byte) 0xed };
            byte[] firstAudio = packet(key, 2, sessionId, 1, 0, UdpPacketDecoder.AUDIO, stereoFrame);
            check(pcm.ingress.accept(firstAudio, firstAudio.length) == AnpdAudioIngress.Result.FORMAT_REJECTED,
                    "AOA must require its authenticated FORMAT before audio");
            check(pcm.pcmQueue.queuedFrames() == 0, "audio-before-FORMAT is not admitted into the AOA queue");

            byte[] formatPacket = packet(key, 2, sessionId, 0, 0,
                    UdpPacketDecoder.FORMAT, standard.encode());
            check(pcm.ingress.accept(formatPacket, formatPacket.length) == AnpdAudioIngress.Result.FORMAT,
                    "AOA profile1 commits the locked V2 FORMAT");
            check(pcm.ingress.accept(firstAudio, firstAudio.length) == AnpdAudioIngress.Result.AUDIO,
                    "AOA profile1 routes authenticated audio through the PCM16 ingress");
            check(pcm.pcmQueue.acceptedAudioFrames() == 1 && pcm.pcmQueue.audioPayloadBytes() == 4,
                    "profile1 audio is counted as one complete stereo PCM16 frame");
            check(pcm.pcmQueue.queuedFrames() == 1, "profile1 audio reaches its precreated bounded queue");
        } finally { pcm.destroy(); }

        checkAoaFloatProfile(key, sessionId + 1, 2, 48_000);
        checkAoaFloatProfile(key, sessionId + 2, 3, 96_000);
    }

    private static void checkAoaFloatProfile(byte[] key, long sessionId, int profile, int sampleRate)
            throws Exception {
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(profile, 0);
        AoaSessionContext context = new AoaSessionContext(sessionId, key, format);
        try {
            check(context.floatQueue.initialTargetFrames() == sampleRate / 25,
                    "AOA profile" + profile + " creates a queue at its locked sample rate");
            byte[] formatPacket = packet(key, 2, sessionId, 0, 0,
                    UdpPacketDecoder.FORMAT, format.encode());
            check(context.ingress.accept(formatPacket, formatPacket.length) == AnpdAudioIngress.Result.FORMAT,
                    "AOA profile" + profile + " commits the locked V2 FORMAT");
            byte[] pcm24 = { 1, 0, 0, (byte) 0xff, (byte) 0xff, (byte) 0xff };
            byte[] audio = packet(key, 2, sessionId, 1, 0, UdpPacketDecoder.AUDIO, pcm24);
            check(context.ingress.accept(audio, audio.length) == AnpdAudioIngress.Result.AUDIO,
                    "AOA profile" + profile + " routes through the shared Float32 ingress");
            float[] samples = new float[2];
            check(context.floatQueue.poll(samples, 0, 1) == 1
                    && samples[0] == 1f / 8_388_608f && samples[1] == -1f / 8_388_608f,
                    "AOA profile" + profile + " keeps PCM24 low bits without PCM16 conversion");
        } finally { context.destroy(); }
    }

    private static void checkFloatUdpReceiverEdges(byte[] key, long sessionId,
            UdpPacketDecoder.AudioFormatV2 format) throws Exception {
        DatagramSocket socket = new DatagramSocket(new InetSocketAddress(InetAddress.getLoopbackAddress(), 0));
        InetSocketAddress local = (InetSocketAddress) socket.getLocalSocketAddress();
        UdpSessionContext context = new UdpSessionContext(socket, local, sessionId, key, format);
        com.anpod.server.FloatPcmFrameQueue queue =
                new com.anpod.server.FloatPcmFrameQueue(48_000, true);
        UdpReceiver receiver = new UdpReceiver(context, queue);
        try {
            byte[] lowBits = { 1, 0, 0, (byte) 0xff, (byte) 0xff, (byte) 0xff };
            byte[] firstAudio = packet(key, 2, sessionId, 1, 0, UdpPacketDecoder.AUDIO, lowBits);
            check(receiver.processDatagram(firstAudio, firstAudio.length, local) == UdpReceiver.ProcessResult.AUDIO,
                    "V2 audio must not wait for a possibly lost FORMAT datagram");
            float[] samples = new float[2];
            check(queue.poll(samples, 0, 1) == 1 && samples[0] == 1f / 8_388_608f
                    && samples[1] == -1f / 8_388_608f,
                    "V2 receiver must preserve PCM24 low bits into the Float32 queue");

            byte[] formatPacket = packet(key, 2, sessionId, 0, 0,
                    UdpPacketDecoder.FORMAT, format.encode());
            check(receiver.processDatagram(formatPacket, formatPacket.length, local) == UdpReceiver.ProcessResult.FORMAT,
                    "a reordered authenticated FORMAT descriptor must remain valid");

            byte[] overflowingIndex = packet(key, 2, sessionId, 2, -1,
                    UdpPacketDecoder.AUDIO, lowBits);
            check(receiver.processDatagram(overflowingIndex, overflowingIndex.length, local)
                    == UdpReceiver.ProcessResult.FORMAT_REJECTED,
                    "wrapping V2 sample frame range must be rejected as a protocol error");
            byte[] validRetry = packet(key, 2, sessionId, 2, 1,
                    UdpPacketDecoder.AUDIO, lowBits);
            check(receiver.processDatagram(validRetry, validRetry.length, local) == UdpReceiver.ProcessResult.AUDIO,
                    "semantic rejection must leave its sequence available for a valid retry");
        } finally {
            receiver.close();
            context.destroy();
        }
    }
}
