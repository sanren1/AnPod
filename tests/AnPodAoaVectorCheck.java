package app.anpod.receiver;

import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
import java.io.InterruptedIOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Map;

/** Shared-vector checks for AOA binding derivation and ANPU record framing. */
public final class AnPodAoaVectorCheck {
    private static final class ChunkInputStream extends InputStream {
        final byte[] bytes;
        final int[] chunkSizes;
        int offset;
        int nextChunk;
        int zeroReads;
        boolean cancelled;
        ChunkInputStream(byte[] bytes, int... chunkSizes) { this.bytes = bytes; this.chunkSizes = chunkSizes; }
        @Override public int read() throws IOException {
            byte[] one = new byte[1];
            int count = read(one, 0, 1);
            return count < 0 ? -1 : count == 0 ? 0 : one[0] & 0xff;
        }
        @Override public int read(byte[] target, int targetOffset, int length) throws IOException {
            if (cancelled) throw new InterruptedIOException("fake read cancelled");
            if (zeroReads > 0) { zeroReads--; return 0; }
            if (offset == bytes.length) return -1;
            int chunk = chunkSizes.length == 0 ? length : chunkSizes[nextChunk++ % chunkSizes.length];
            int count = Math.min(Math.min(length, chunk), bytes.length - offset);
            System.arraycopy(bytes, offset, target, targetOffset, count);
            offset += count;
            return count;
        }
    }

    private static Map<String, byte[]> vectors(Path path) throws IOException {
        Map<String, byte[]> result = new HashMap<>();
        for (String line : Files.readAllLines(path)) {
            if (line.isBlank() || line.startsWith("#")) continue;
            String[] fields = line.split("=", 2);
            String value = fields[1];
            result.put(fields[0], isHex(value) ? hex(value) : value.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        }
        return result;
    }

    private static boolean isHex(String value) {
        if ((value.length() & 1) != 0 || value.isEmpty()) return false;
        for (int i = 0; i < value.length(); i++)
            if (Character.digit(value.charAt(i), 16) < 0) return false;
        return true;
    }

    private static byte[] hex(String value) {
        byte[] bytes = new byte[value.length() / 2];
        for (int i = 0; i < bytes.length; i++)
            bytes[i] = (byte) Integer.parseInt(value.substring(i * 2, i * 2 + 2), 16);
        return bytes;
    }

    private static byte[] sequence(int start, int length) {
        byte[] bytes = new byte[length];
        for (int i = 0; i < length; i++) bytes[i] = (byte) (start + i);
        return bytes;
    }

    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }

    private static void equal(byte[] actual, byte[] expected, String message) {
        check(Arrays.equals(actual, expected), message);
    }

    private static void reject(RunnableIo action, String message) throws Exception {
        try { action.run(); throw new AssertionError(message); }
        catch (IOException expected) { }
    }

    private interface RunnableIo { void run() throws Exception; }

    private static long sessionId(byte[] hello) {
        return ByteBuffer.wrap(hello, 72, 8).order(ByteOrder.BIG_ENDIAN).getLong();
    }

    private static void verifyProfile(Map<String, byte[]> v, PairingProtocol.BindingMaterial aoa,
            int profile) throws Exception {
        String prefix = "profile." + profile + ".";
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.decode(v.get(prefix + "descriptor"));
        byte[] hello = v.get(prefix + "hello");
        long sessionId = sessionId(hello);
        byte[] hostId = sequence(0x10, 16);
        byte[] receiverId = sequence(0x20, 16);
        byte[] hostNonce = sequence(0x30, 32);
        byte[] receiverNonce = sequence(0x50, 32);
        byte[] transcript = PairingProtocol.V2.transcriptForVector(hostId, receiverId, hostNonce,
                receiverNonce, sessionId, format);
        equal(transcript, v.get(prefix + "transcript"), prefix + "transcript");
        byte[] secret = aoa.discoverySecret();
        byte[] hostProof = PairingProtocol.V2.proofForVector(secret, "host", transcript);
        byte[] receiverProof = PairingProtocol.V2.proofForVector(secret, "receiver", transcript);
        byte[] readyProof = PairingProtocol.V2.readyForVector(secret, transcript, hostProof, receiverProof);
        equal(hostProof, v.get(prefix + "host_proof"), prefix + "host proof");
        equal(receiverProof, v.get(prefix + "receiver_proof"), prefix + "receiver proof");
        equal(readyProof, v.get(prefix + "ready_proof"), prefix + "ready proof");
        equal(PairingProtocol.V2.helloForVector(hostId, receiverId, hostNonce, sessionId, format), hello,
                prefix + "HELLO");
        equal(PairingProtocol.V2.challengeForVector(receiverId, receiverNonce, format, receiverProof),
                v.get(prefix + "challenge"), prefix + "CHALLENGE");
        equal(PairingProtocol.V2.proofMessageForVector(3, hostProof), v.get(prefix + "finish"), prefix + "FINISH");
        equal(PairingProtocol.V2.proofMessageForVector(4, readyProof), v.get(prefix + "ready"), prefix + "READY");

        PairingProtocol.SessionKeys keys = PairingProtocol.V2.deriveForVector(secret, transcript);
        try {
            equal(keys.hostToReceiverAudio, v.get(prefix + "key_h2r_audio"), prefix + "audio key");
            equal(keys.receiverToHostAudio, v.get(prefix + "key_r2h_audio"), prefix + "reverse audio key");
            equal(keys.hostToReceiverControl, v.get(prefix + "key_h2r_control"), prefix + "control key");
            equal(keys.receiverToHostControl, v.get(prefix + "key_r2h_control"), prefix + "reverse control key");

            byte[] capabilities = P5cControlProtocol.encode(keys.hostToReceiverControl,
                    P5cControlProtocol.HOST_TO_RECEIVER, sessionId, 0, P5cControlProtocol.CAPABILITIES_QUERY,
                    v.get(prefix + "capabilities_query_payload"));
            byte[] bind = P5cControlProtocol.encode(keys.hostToReceiverControl,
                    P5cControlProtocol.HOST_TO_RECEIVER, sessionId, 1, P5cControlProtocol.BIND,
                    v.get(prefix + "aoa_bind_payload"));
            equal(capabilities, v.get(prefix + "anpc_capabilities_query"), prefix + "ANPC capabilities");
            equal(bind, v.get(prefix + "anpc_aoa_bind"), prefix + "ANPC AOA bind");
            equal(AnpuRecordCodec.encode(AnpuRecordCodec.CONTROL, capabilities),
                    v.get(prefix + "anpu_capabilities_query"), prefix + "ANPU capabilities");
            equal(AnpuRecordCodec.encode(AnpuRecordCodec.CONTROL, bind),
                    v.get(prefix + "anpu_aoa_bind"), prefix + "ANPU AOA bind");

            equal(AnpuRecordCodec.encode(AnpuRecordCodec.HANDSHAKE, hello), v.get(prefix + "anpu_hello"),
                    prefix + "ANPU HELLO");
            equal(AnpuRecordCodec.encode(AnpuRecordCodec.HANDSHAKE, v.get(prefix + "challenge")),
                    v.get(prefix + "anpu_challenge"), prefix + "ANPU CHALLENGE");
            equal(AnpuRecordCodec.encode(AnpuRecordCodec.HANDSHAKE, v.get(prefix + "finish")),
                    v.get(prefix + "anpu_finish"), prefix + "ANPU FINISH");
            equal(AnpuRecordCodec.encode(AnpuRecordCodec.HANDSHAKE, v.get(prefix + "ready")),
                    v.get(prefix + "anpu_ready"), prefix + "ANPU READY");

            UdpPacketDecoder decoder = UdpPacketDecoder.newV2(sessionId, keys.hostToReceiverAudio, format);
            try {
                byte[] formatRecord = AnpuRecordCodec.encode(AnpuRecordCodec.AUDIO, v.get(prefix + "format_datagram"));
                equal(formatRecord, v.get(prefix + "anpu_format"), prefix + "ANPU FORMAT");
                UdpPacketDecoder.Packet decodedFormat = decoder.open(v.get(prefix + "format_datagram"),
                        v.get(prefix + "format_datagram").length);
                equal(decodedFormat.payload, v.get(prefix + "format_payload"), prefix + "FORMAT payload");
                check(decodedFormat.type == UdpPacketDecoder.FORMAT && decodedFormat.sampleFrameIndex == 0,
                        prefix + "FORMAT starts at frame zero");
                decoder.commit(decodedFormat.sequence);
                byte[] audio = v.get(prefix + "audio_datagram");
                equal(AnpuRecordCodec.encode(AnpuRecordCodec.AUDIO, audio), v.get(prefix + "anpu_audio"),
                        prefix + "ANPU AUDIO");
                UdpPacketDecoder.Packet decodedAudio = decoder.open(audio, audio.length);
                equal(decodedAudio.payload, v.get(prefix + "audio_payload"), prefix + "whole-frame AUDIO payload");
                check(decodedAudio.type == UdpPacketDecoder.AUDIO && decodedAudio.sampleFrameIndex == 0,
                        prefix + "AUDIO frame index");
                decoder.commit(decodedAudio.sequence);
                reject(() -> decoder.open(audio, audio.length), prefix + "duplicate AUDIO accepted");
            } finally { decoder.destroy(); }

            byte[] wrongDomainAudio = v.get(prefix + "wrong_domain_audio_datagram");
            byte[] lanSecret = sequence(0, 32);
            byte[] lanReceiverProof = PairingProtocol.V2.proofForVector(lanSecret, "receiver", transcript);
            equal(lanReceiverProof, v.get(prefix + "wrong_domain_receiver_proof"),
                    prefix + "LAN-domain receiver proof");
            PairingProtocol.SessionKeys lanKeys = PairingProtocol.V2.deriveForVector(lanSecret, transcript);
            try {
                equal(lanKeys.hostToReceiverAudio, v.get(prefix + "wrong_domain_audio_key"),
                        prefix + "LAN-domain audio key");
            } finally { lanKeys.close(); Arrays.fill(lanSecret, (byte) 0); }
            UdpPacketDecoder wrongDomainCheck = UdpPacketDecoder.newV2(sessionId,
                    keys.hostToReceiverAudio, format);
            try {
                try { wrongDomainCheck.open(wrongDomainAudio, wrongDomainAudio.length);
                    throw new AssertionError(prefix + "LAN-domain audio authenticated in AOA"); }
                catch (UdpPacketDecoder.ProtocolException expected) {
                    check(expected.reason == UdpPacketDecoder.ProtocolException.AUTHENTICATION,
                            prefix + "wrong-domain audio rejected at authentication");
                }
            } finally { wrongDomainCheck.destroy(); }
        } finally {
            Arrays.fill(secret, (byte) 0);
            keys.close();
        }
    }

    private static void verifyFakeReads(byte[] first, byte[] second) throws Exception {
        byte[] coalesced = new byte[first.length + second.length];
        System.arraycopy(first, 0, coalesced, 0, first.length);
        System.arraycopy(second, 0, coalesced, first.length, second.length);
        ChunkInputStream source = new ChunkInputStream(coalesced, 1, 3, 2, 5);
        equal(AnpuRecordCodec.read(source).body,
                AnpuRecordCodec.decode(first).body, "one-byte/short read");
        equal(AnpuRecordCodec.read(source).body,
                AnpuRecordCodec.decode(second).body, "coalesced following record");

        byte[] truncated = Arrays.copyOf(first, first.length - 1);
        reject(() -> AnpuRecordCodec.read(new ChunkInputStream(truncated, 7)), "truncated record accepted");
        reject(() -> AnpuRecordCodec.read(new ChunkInputStream(new byte[0])), "EOF accepted as a record");
        ChunkInputStream noProgress = new ChunkInputStream(first);
        noProgress.zeroReads = 1;
        reject(() -> AnpuRecordCodec.read(noProgress), "zero-progress read accepted");
        ChunkInputStream cancelled = new ChunkInputStream(first);
        cancelled.cancelled = true;
        try { AnpuRecordCodec.read(cancelled); throw new AssertionError("cancelled read continued"); }
        catch (InterruptedIOException expected) { }

        ByteArrayOutputStream output = new ByteArrayOutputStream();
        AnpuRecordCodec.write(output, AnpuRecordCodec.AUDIO, AnpuRecordCodec.decode(first).body);
        equal(output.toByteArray(), first, "OutputStream writes one complete ANPU record");
        ByteArrayOutputStream invalidOutput = new ByteArrayOutputStream();
        try { AnpuRecordCodec.write(invalidOutput, AnpuRecordCodec.AUDIO, new byte[0]);
            throw new AssertionError("invalid output body accepted"); }
        catch (IOException expected) { }
        check(invalidOutput.size() == 0, "invalid output was partially written");
    }

    public static void main(String[] args) throws Exception {
        Map<String, byte[]> v = vectors(Path.of(args[0]));
        byte[] hostId = sequence(0x10, 16);
        byte[] receiverId = sequence(0x20, 16);
        byte[] secret = sequence(0, 32);
        PairingProtocol.BindingMaterial original = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        PairingProtocol.BindingMaterial aoa = original.deriveAoa();
        byte[] derivedSecret = aoa.discoverySecret();
        equal(derivedSecret, v.get("aoa.binding_secret"), "AOA binding secret");
        byte[] saltLabel = "AnPod AOA Direct binding salt v1".getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        ByteBuffer saltInput = ByteBuffer.allocate(saltLabel.length + 1 + 32).put(saltLabel).put((byte) 0)
                .put(hostId).put(receiverId);
        equal(MessageDigest.getInstance("SHA-256").digest(saltInput.array()), v.get("aoa.salt"), "AOA salt");
        equal(secret, v.get("wrong_domain.binding_secret"), "LAN binding fixture remains unchanged");

        for (int profile = 1; profile <= 3; profile++) verifyProfile(v, aoa, profile);

        byte[] hello = v.get("profile.1.hello");
        try (PairingProtocol.V2.ReceiverAwaitFinish lanReceiver =
                PairingProtocol.V2.Receiver.acceptHello(original.duplicate(), hello)) {
            try {
                lanReceiver.acceptFinish(v.get("profile.1.finish"));
                throw new AssertionError("LAN-domain binding accepted AOA handshake proof");
            } catch (PairingProtocol.Failure expected) { }
        }

        AnpuRecordCodec.Record parsed = AnpuRecordCodec.decode(v.get("profile.1.anpu_audio"));
        check(parsed.type == AnpuRecordCodec.AUDIO, "ANPU type decoded");
        equal(parsed.body, v.get("profile.1.audio_datagram"), "ANPU body decoded");
        verifyFakeReads(v.get("profile.1.anpu_format"), v.get("profile.1.anpu_audio"));

        byte[] badType = v.get("profile.1.anpu_audio").clone();
        badType[5] = 4;
        reject(() -> AnpuRecordCodec.decode(badType), "unknown type accepted");
        byte[] zeroLength = v.get("profile.1.anpu_audio").clone();
        zeroLength[6] = 0; zeroLength[7] = 0;
        reject(() -> AnpuRecordCodec.decode(zeroLength), "zero body accepted");
        byte[] wrongVersion = v.get("profile.1.anpu_audio").clone();
        wrongVersion[4] = 2;
        reject(() -> AnpuRecordCodec.decode(wrongVersion), "unknown version accepted");

        Arrays.fill(derivedSecret, (byte) 0);
        aoa.close();
        original.close();
        System.out.println("AOA vectors and ANPU framing passed");
    }
}
