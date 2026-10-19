package app.anpod.receiver;

import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Arrays;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/** Offline P5-A binding and mutually authenticated session derivation. No socket or product hook. */
final class PairingProtocol {
    static final int VERSION = 1;
    static final int BINDING_BYTES = 69;
    private static final byte[] MAGIC = {'A', 'N', 'P', '5'};
    private static final byte[] BINDING_MAGIC = {'A', 'N', 'P', 'B'};
    private static final int HOST = 1;
    private static final int RECEIVER = 2;
    private static final int HELLO = 1;
    private static final int CHALLENGE = 2;
    private static final int FINISH = 3;
    private static final int READY = 4;
    private static final String HOST_PROOF = "AnPod P5-A proof host v1";
    private static final String RECEIVER_PROOF = "AnPod P5-A proof receiver v1";
    private static final String READY_PROOF = "AnPod P5-A ready v1";
    private static final String HKDF_SALT = "AnPod P5-A HKDF-SHA256 salt v1";
    private static final String AUDIO_HOST_TO_RECEIVER = "AnPod P5-A key host-to-receiver audio AES-256-GCM v1";
    private static final String AUDIO_RECEIVER_TO_HOST = "AnPod P5-A key receiver-to-host audio AES-256-GCM v1";
    private static final String CONTROL_HOST_TO_RECEIVER = "AnPod P5-A key host-to-receiver control HMAC-SHA256 v1";
    private static final String CONTROL_RECEIVER_TO_HOST = "AnPod P5-A key receiver-to-host control HMAC-SHA256 v1";
    private static final SecureRandom RANDOM = new SecureRandom();

    private PairingProtocol() {}

    static final class BindingMaterial implements AutoCloseable {
        private final byte[] hostId;
        private final byte[] receiverId;
        private final byte[] secret;
        private boolean closed;

        BindingMaterial(byte[] hostId, byte[] receiverId, byte[] secret) {
            if (hostId.length != 16 || receiverId.length != 16 || secret.length != 32)
                throw new IllegalArgumentException("invalid binding material lengths");
            this.hostId = hostId.clone();
            this.receiverId = receiverId.clone();
            this.secret = secret.clone();
        }

        byte[] discoverySecret() { return secret.clone(); }

        static BindingMaterial generate() {
            byte[] random = new byte[64];
            RANDOM.nextBytes(random);
            byte[] hostId = Arrays.copyOfRange(random, 0, 16);
            byte[] receiverId = Arrays.copyOfRange(random, 16, 32);
            byte[] secret = Arrays.copyOfRange(random, 32, 64);
            try { return new BindingMaterial(hostId, receiverId, secret); }
            finally {
                Arrays.fill(random, (byte) 0);
                Arrays.fill(hostId, (byte) 0);
                Arrays.fill(receiverId, (byte) 0);
                Arrays.fill(secret, (byte) 0);
            }
        }

        byte[] encodedRecord() {
            requireOpen();
            ByteBuffer bytes = ByteBuffer.allocate(BINDING_BYTES);
            bytes.put(BINDING_MAGIC).put((byte) 1).put(hostId).put(receiverId).put(secret);
            return bytes.array();
        }

        byte[] commitment() throws Failure {
            byte[] record = encodedRecord();
            try { return MessageDigest.getInstance("SHA-256").digest(record); }
            catch (GeneralSecurityException error) { throw new Failure("SHA-256 unavailable"); }
            finally { Arrays.fill(record, (byte) 0); }
        }

        static BindingMaterial readRecord(InputStream input) throws IOException, Failure {
            byte[] record = new byte[BINDING_BYTES];
            try {
                int offset = 0;
                while (offset < record.length) {
                    int count = input.read(record, offset, record.length - offset);
                    if (count < 0) throw new Failure("truncated binding record");
                    if (count > 0) offset += count;
                }
                if (input.read() != -1) throw new Failure("oversized binding record");
                return decode(record);
            } finally {
                Arrays.fill(record, (byte) 0);
            }
        }

        private BindingMaterial transfer() {
            requireOpen();
            BindingMaterial moved = new BindingMaterial(hostId, receiverId, secret);
            close();
            return moved;
        }

        BindingMaterial duplicate() {
            requireOpen();
            return new BindingMaterial(hostId, receiverId, secret);
        }

        static BindingMaterial decode(byte[] bytes) throws Failure {
            if (bytes.length != BINDING_BYTES || !matches(bytes, 0, BINDING_MAGIC) || bytes[4] != 1)
                throw new Failure("malformed binding record");
            return new BindingMaterial(Arrays.copyOfRange(bytes, 5, 21),
                    Arrays.copyOfRange(bytes, 21, 37), Arrays.copyOfRange(bytes, 37, 69));
        }

        private void requireOpen() {
            if (closed) throw new IllegalStateException("binding material already consumed");
        }

        @Override public void close() {
            if (closed) return;
            closed = true;
            Arrays.fill(hostId, (byte) 0);
            Arrays.fill(receiverId, (byte) 0);
            Arrays.fill(secret, (byte) 0);
        }
    }

    static final class AudioParameters {
        final int sampleRate;
        final int channels;
        /** 0 = balanced, 1 = low latency. */
        final int mode;

        AudioParameters(int sampleRate, int channels, int mode) throws Failure {
            if (sampleRate < 8_000 || sampleRate > 192_000 || (channels != 1 && channels != 2)
                    || (mode != 0 && mode != 1)) throw new Failure("unsupported audio parameters");
            this.sampleRate = sampleRate;
            this.channels = channels;
            this.mode = mode;
        }

        byte[] encode() {
            return ByteBuffer.allocate(7).order(ByteOrder.BIG_ENDIAN).put((byte) 1).put((byte) channels)
                    .putInt(sampleRate).put((byte) mode).array();
        }

        static AudioParameters decode(byte[] bytes) throws Failure {
            if (bytes.length != 7 || bytes[0] != 1) throw new Failure("unsupported audio parameters");
            return new AudioParameters(ByteBuffer.wrap(bytes, 2, 4).order(ByteOrder.BIG_ENDIAN).getInt(),
                    bytes[1] & 0xff, bytes[6] & 0xff);
        }

        @Override public boolean equals(Object other) {
            if (!(other instanceof AudioParameters)) return false;
            AudioParameters that = (AudioParameters) other;
            return sampleRate == that.sampleRate && channels == that.channels && mode == that.mode;
        }

        @Override public int hashCode() { return sampleRate * 31 + channels * 7 + mode; }
    }

    static final class HostHandshake implements AutoCloseable {
        private final BindingMaterial material;
        private final AudioParameters parameters;
        private final byte[] hostNonce;
        private final long sessionId;
        private final byte[] hello;
        private boolean consumed;

        private HostHandshake(BindingMaterial material, AudioParameters parameters, byte[] hostNonce, long sessionId) {
            this.material = material;
            this.parameters = parameters;
            this.hostNonce = hostNonce;
            this.sessionId = sessionId;
            this.hello = encodeHello(material, hostNonce, sessionId, parameters);
        }

        static HostHandshake start(BindingMaterial material, AudioParameters parameters) {
            BindingMaterial owned = material.transfer();
            try {
                byte[] nonce = randomBytes(32);
                byte[] session = randomBytes(8);
                long sessionId = ByteBuffer.wrap(session).order(ByteOrder.BIG_ENDIAN).getLong();
                Arrays.fill(session, (byte) 0);
                return new HostHandshake(owned, parameters, nonce, sessionId);
            } catch (RuntimeException error) {
                owned.close();
                throw error;
            }
        }

        byte[] hello() { requireActive(); return hello.clone(); }

        HostAwaitReady acceptChallenge(byte[] message) throws Failure {
            requireActive();
            consumed = true;
            try {
                byte[] body = parseMessage(message, CHALLENGE, 86);
                if (!MessageDigest.isEqual(material.receiverId, Arrays.copyOfRange(body, 0, 16)))
                    throw new Failure("peer identity mismatch");
                byte[] receiverNonce = Arrays.copyOfRange(body, 16, 48);
                byte[] proof = Arrays.copyOfRange(body, 48, 80);
                byte[] transcript = transcript(material, hostNonce, receiverNonce, sessionId, parameters);
                verify(material.secret, RECEIVER_PROOF, transcript, new byte[0], proof);
                byte[] hostProof = mac(material.secret, HOST_PROOF, transcript, new byte[0]);
                return new HostAwaitReady(material, parameters, sessionId, transcript, proof, hostProof);
            } catch (Failure error) {
                material.close();
                throw error;
            }
        }

        private void requireActive() {
            if (consumed) throw new IllegalStateException("host handshake already consumed");
            material.requireOpen();
        }

        @Override public void close() {
            if (!consumed) { consumed = true; material.close(); }
        }
    }

    static final class HostAwaitReady implements AutoCloseable {
        private final BindingMaterial material;
        private final AudioParameters parameters;
        private final long sessionId;
        private final byte[] transcript;
        private final byte[] receiverProof;
        private final byte[] hostProof;
        private boolean consumed;

        private HostAwaitReady(BindingMaterial material, AudioParameters parameters, long sessionId,
                byte[] transcript, byte[] receiverProof, byte[] hostProof) {
            this.material = material;
            this.parameters = parameters;
            this.sessionId = sessionId;
            this.transcript = transcript;
            this.receiverProof = receiverProof;
            this.hostProof = hostProof;
        }

        byte[] finish() { requireActive(); return encodeProof(FINISH, hostProof); }

        AuthenticatedSession acceptReady(byte[] message) throws Failure {
            requireActive();
            consumed = true;
            byte[] proofs = concat(hostProof, receiverProof);
            try {
                byte[] body = parseMessage(message, READY, 38);
                verify(material.secret, READY_PROOF, transcript, proofs, body);
                AuthenticatedSession session = authenticated(sessionId, parameters, transcript, material.secret);
                material.close();
                return session;
            } catch (Failure error) {
                material.close();
                throw error;
            } finally { Arrays.fill(proofs, (byte) 0); }
        }

        private void requireActive() {
            if (consumed) throw new IllegalStateException("host handshake already consumed");
            material.requireOpen();
        }

        @Override public void close() {
            if (!consumed) { consumed = true; material.close(); }
        }
    }

    static final class ReceiverHandshake implements AutoCloseable {
        private final BindingMaterial material;
        private final AudioParameters parameters;
        private final long sessionId;
        private final byte[] transcript;
        private final byte[] receiverProof;
        private final byte[] challenge;
        private boolean consumed;

        private ReceiverHandshake(BindingMaterial material, AudioParameters parameters, long sessionId,
                byte[] transcript, byte[] receiverProof, byte[] challenge) {
            this.material = material;
            this.parameters = parameters;
            this.sessionId = sessionId;
            this.transcript = transcript;
            this.receiverProof = receiverProof;
            this.challenge = challenge;
        }

        static ReceiverHandshake acceptHello(BindingMaterial material, byte[] message) throws Failure {
            BindingMaterial owned = material.transfer();
            try {
                byte[] body = parseMessage(message, HELLO, 85);
                if (!MessageDigest.isEqual(owned.hostId, Arrays.copyOfRange(body, 0, 16))
                        || !MessageDigest.isEqual(owned.receiverId, Arrays.copyOfRange(body, 16, 32)))
                    throw new Failure("peer identity mismatch");
                byte[] hostNonce = Arrays.copyOfRange(body, 32, 64);
                long sessionId = ByteBuffer.wrap(body, 64, 8).order(ByteOrder.BIG_ENDIAN).getLong();
                AudioParameters parameters = AudioParameters.decode(Arrays.copyOfRange(body, 72, 79));
                byte[] receiverNonce = randomBytes(32);
                byte[] transcript = transcript(owned, hostNonce, receiverNonce, sessionId, parameters);
                byte[] receiverProof = mac(owned.secret, RECEIVER_PROOF, transcript, new byte[0]);
                byte[] challenge = encodeChallenge(owned.receiverId, receiverNonce, receiverProof);
                return new ReceiverHandshake(owned, parameters, sessionId, transcript, receiverProof, challenge);
            } catch (Failure | RuntimeException error) {
                owned.close();
                throw error;
            }
        }

        byte[] challenge() { requireActive(); return challenge.clone(); }

        FinishResult acceptFinish(byte[] message) throws Failure {
            requireActive();
            consumed = true;
            try {
                byte[] body = parseMessage(message, FINISH, 38);
                verify(material.secret, HOST_PROOF, transcript, new byte[0], body);
                byte[] proofs = concat(body, receiverProof);
                byte[] ready = mac(material.secret, READY_PROOF, transcript, proofs);
                Arrays.fill(proofs, (byte) 0);
                AuthenticatedSession session = authenticated(sessionId, parameters, transcript, material.secret);
                material.close();
                return new FinishResult(encodeProof(READY, ready), session);
            } catch (Failure error) {
                material.close();
                throw error;
            }
        }

        private void requireActive() {
            if (consumed) throw new IllegalStateException("receiver handshake already consumed");
            material.requireOpen();
        }

        @Override public void close() {
            if (!consumed) { consumed = true; material.close(); }
        }
    }

    static final class FinishResult {
        final byte[] ready;
        final AuthenticatedSession session;
        private FinishResult(byte[] ready, AuthenticatedSession session) {
            this.ready = ready;
            this.session = session;
        }
    }

    static final class AuthenticatedSession implements AutoCloseable {
        final long sessionId;
        final AudioParameters parameters;
        private SessionKeys keys;

        private AuthenticatedSession(long sessionId, AudioParameters parameters, SessionKeys keys) {
            this.sessionId = sessionId;
            this.parameters = parameters;
            this.keys = keys;
        }

        synchronized SessionKeys takeKeys() {
            if (keys == null) throw new IllegalStateException("session keys already consumed");
            SessionKeys result = keys;
            keys = null;
            return result;
        }

        @Override public synchronized void close() {
            if (keys == null) return;
            keys.close();
            keys = null;
        }
    }

    static final class SessionKeys implements AutoCloseable {
        final byte[] hostToReceiverAudio;
        final byte[] receiverToHostAudio;
        final byte[] hostToReceiverControl;
        final byte[] receiverToHostControl;

        private SessionKeys(byte[] hostToReceiverAudio, byte[] receiverToHostAudio,
                byte[] hostToReceiverControl, byte[] receiverToHostControl) {
            this.hostToReceiverAudio = hostToReceiverAudio;
            this.receiverToHostAudio = receiverToHostAudio;
            this.hostToReceiverControl = hostToReceiverControl;
            this.receiverToHostControl = receiverToHostControl;
        }

        @Override public void close() {
            Arrays.fill(hostToReceiverAudio, (byte) 0);
            Arrays.fill(receiverToHostAudio, (byte) 0);
            Arrays.fill(hostToReceiverControl, (byte) 0);
            Arrays.fill(receiverToHostControl, (byte) 0);
        }
    }

    static final class Failure extends Exception {
        Failure(String message) { super(message); }
    }

    static byte[] transcriptForVector(byte[] hostId, byte[] receiverId, byte[] hostNonce,
            byte[] receiverNonce, long sessionId, AudioParameters parameters) {
        BindingMaterial fixture = new BindingMaterial(hostId, receiverId, new byte[32]);
        try { return transcript(fixture, hostNonce, receiverNonce, sessionId, parameters); }
        finally { fixture.close(); }
    }

    static byte[] proofForVector(byte[] secret, String roleLabel, byte[] transcript) {
        return mac(secret, "AnPod P5-A proof " + roleLabel + " v1", transcript, new byte[0]);
    }

    static byte[] readyForVector(byte[] secret, byte[] transcript, byte[] hostProof, byte[] receiverProof) {
        return mac(secret, READY_PROOF, transcript, concat(hostProof, receiverProof));
    }

    static byte[] helloForVector(byte[] hostId, byte[] receiverId, byte[] hostNonce,
            long sessionId, AudioParameters parameters) {
        BindingMaterial fixture = new BindingMaterial(hostId, receiverId, new byte[32]);
        try { return encodeHello(fixture, hostNonce, sessionId, parameters); }
        finally { fixture.close(); }
    }

    static SessionKeys deriveForVector(byte[] secret, byte[] transcript) throws Failure {
        return deriveKeys(secret, transcript);
    }

    private static byte[] encodeHello(BindingMaterial material, byte[] hostNonce, long sessionId,
            AudioParameters parameters) {
        ByteBuffer message = ByteBuffer.allocate(85).order(ByteOrder.BIG_ENDIAN);
        startMessage(message, HELLO);
        message.put(material.hostId).put(material.receiverId).put(hostNonce).putLong(sessionId).put(parameters.encode());
        return message.array();
    }

    private static byte[] encodeChallenge(byte[] receiverId, byte[] receiverNonce, byte[] proof) {
        ByteBuffer message = ByteBuffer.allocate(86).order(ByteOrder.BIG_ENDIAN);
        startMessage(message, CHALLENGE);
        message.put(receiverId).put(receiverNonce).put(proof);
        return message.array();
    }

    private static byte[] encodeProof(int type, byte[] proof) {
        ByteBuffer message = ByteBuffer.allocate(38).order(ByteOrder.BIG_ENDIAN);
        startMessage(message, type);
        message.put(proof);
        return message.array();
    }

    private static void startMessage(ByteBuffer message, int type) {
        message.put(MAGIC).put((byte) VERSION).put((byte) type);
    }

    private static byte[] parseMessage(byte[] message, int type, int length) throws Failure {
        if (message.length != length || !matches(message, 0, MAGIC) || (message[4] & 0xff) != VERSION)
            throw new Failure("malformed P5-A message");
        if ((message[5] & 0xff) != type) throw new Failure("wrong P5-A message role");
        return Arrays.copyOfRange(message, 6, message.length);
    }

    private static byte[] transcript(BindingMaterial material, byte[] hostNonce, byte[] receiverNonce,
            long sessionId, AudioParameters parameters) {
        material.requireOpen();
        if (hostNonce.length != 32 || receiverNonce.length != 32) throw new IllegalArgumentException("nonce length");
        ByteBuffer transcript = ByteBuffer.allocate(118).order(ByteOrder.BIG_ENDIAN);
        transcript.put(MAGIC).put((byte) VERSION).put((byte) HOST).put((byte) RECEIVER)
                .put(material.hostId).put(material.receiverId).put(hostNonce).put(receiverNonce)
                .putLong(sessionId).put(parameters.encode());
        return transcript.array();
    }

    private static byte[] mac(byte[] secret, String label, byte[] transcript, byte[] extra) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(secret, "HmacSHA256"));
            mac.update(label.getBytes(java.nio.charset.StandardCharsets.US_ASCII));
            mac.update((byte) 0);
            mac.update(transcript);
            mac.update(extra);
            return mac.doFinal();
        } catch (GeneralSecurityException error) {
            throw new IllegalStateException("HmacSHA256 unavailable", error);
        }
    }

    private static void verify(byte[] secret, String label, byte[] transcript, byte[] extra, byte[] received)
            throws Failure {
        if (received.length != 32 || !MessageDigest.isEqual(mac(secret, label, transcript, extra), received))
            throw new Failure("P5-A authentication failed");
    }

    private static byte[] concat(byte[] first, byte[] second) {
        byte[] result = Arrays.copyOf(first, first.length + second.length);
        System.arraycopy(second, 0, result, first.length, second.length);
        return result;
    }

    private static AuthenticatedSession authenticated(long sessionId, AudioParameters parameters,
            byte[] transcript, byte[] secret) throws Failure {
        return new AuthenticatedSession(sessionId, parameters, deriveKeys(secret, transcript));
    }

    private static SessionKeys deriveKeys(byte[] secret, byte[] transcript) throws Failure {
        return deriveKeys(secret, transcript, 1);
    }

    private static SessionKeys deriveKeys(byte[] secret, byte[] transcript, int version) throws Failure {
        String suffix = " v" + version;
        String saltLabel = "AnPod P5-A HKDF-SHA256 salt" + suffix;
        byte[] saltInput = concat((saltLabel + "\0").getBytes(java.nio.charset.StandardCharsets.US_ASCII), transcript);
        byte[] salt;
        try { salt = MessageDigest.getInstance("SHA-256").digest(saltInput); }
        catch (GeneralSecurityException error) { throw new Failure("SHA-256 unavailable"); }
        finally { Arrays.fill(saltInput, (byte) 0); }
        byte[] prk = rawHmac(salt, secret);
        Arrays.fill(salt, (byte) 0);
        byte[] hostAudio = null;
        byte[] receiverAudio = null;
        byte[] hostControl = null;
        byte[] receiverControl = null;
        try {
            hostAudio = expand(prk, "AnPod P5-A key host-to-receiver audio AES-256-GCM" + suffix);
            receiverAudio = expand(prk, "AnPod P5-A key receiver-to-host audio AES-256-GCM" + suffix);
            hostControl = expand(prk, "AnPod P5-A key host-to-receiver control HMAC-SHA256" + suffix);
            receiverControl = expand(prk, "AnPod P5-A key receiver-to-host control HMAC-SHA256" + suffix);
            SessionKeys keys = new SessionKeys(hostAudio, receiverAudio, hostControl, receiverControl);
            hostAudio = receiverAudio = hostControl = receiverControl = null;
            return keys;
        } finally {
            Arrays.fill(prk, (byte) 0);
            if (hostAudio != null) Arrays.fill(hostAudio, (byte) 0);
            if (receiverAudio != null) Arrays.fill(receiverAudio, (byte) 0);
            if (hostControl != null) Arrays.fill(hostControl, (byte) 0);
            if (receiverControl != null) Arrays.fill(receiverControl, (byte) 0);
        }
    }

    private static byte[] expand(byte[] prk, String info) {
        // RFC 5869 HKDF-Expand, one 32-byte output block for each distinct purpose label.
        return rawHmac(prk, concat(info.getBytes(java.nio.charset.StandardCharsets.US_ASCII), new byte[] {1}));
    }

    private static byte[] rawHmac(byte[] key, byte[] input) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            return mac.doFinal(input);
        } catch (GeneralSecurityException error) {
            throw new IllegalStateException("HmacSHA256 unavailable", error);
        }
    }

    private static byte[] randomBytes(int length) {
        byte[] bytes = new byte[length];
        RANDOM.nextBytes(bytes);
        return bytes;
    }

    private static boolean matches(byte[] bytes, int offset, byte[] expected) {
        return bytes.length >= offset + expected.length
                && MessageDigest.isEqual(Arrays.copyOfRange(bytes, offset, offset + expected.length), expected);
    }

    // Offline Phase 1 v2 negotiation only. Product pairing remains on the v1 classes above.
    static final class V2 {
        private static final int HEADER = 8;
        private static byte[] message(int type, int length, byte[]... parts) {
            ByteBuffer out = ByteBuffer.allocate(length).order(ByteOrder.BIG_ENDIAN);
            out.put(MAGIC).put((byte) 2).put((byte) type).putShort((short) length);
            for (byte[] part : parts) out.put(part);
            return out.array();
        }
        private static byte[] parse(byte[] message, int type, int length) throws Failure {
            if (message.length != length || !matches(message, 0, MAGIC) || message[4] != 2
                    || (message[5] & 0xff) != type || (((message[6] & 0xff) << 8) | (message[7] & 0xff)) != length)
                throw new Failure("malformed P5-A v2 message");
            return Arrays.copyOfRange(message, HEADER, length);
        }
        private static UdpPacketDecoder.AudioFormatV2 parseFormat(byte[] bytes) throws Failure {
            try { return UdpPacketDecoder.AudioFormatV2.decode(bytes); }
            catch (IllegalArgumentException error) { throw new Failure("unsupported v2 audio descriptor"); }
        }
        private static byte[] transcript(BindingMaterial material, byte[] hostNonce, byte[] receiverNonce,
                long sessionId, UdpPacketDecoder.AudioFormatV2 requested,
                UdpPacketDecoder.AudioFormatV2 selected) {
            ByteBuffer out = ByteBuffer.allocate(143).order(ByteOrder.BIG_ENDIAN);
            out.put(MAGIC).put((byte) 2).put((byte) HOST).put((byte) RECEIVER)
                    .put(material.hostId).put(material.receiverId).put(hostNonce).put(receiverNonce)
                    .putLong(sessionId).put(requested.encode()).put(selected.encode());
            return out.array();
        }
        private static byte[] mac(byte[] secret, String label, byte[] transcript, byte[] extra) {
            return PairingProtocol.mac(secret, "AnPod P5-A " + label + " v2", transcript, extra);
        }

        static final class Host implements AutoCloseable {
            private BindingMaterial material;
            private final UdpPacketDecoder.AudioFormatV2 format;
            private final byte[] hostNonce = randomBytes(32);
            private final long sessionId;
            private byte[] receiverProof;
            private byte[] hostProof;
            private int state = 0;
            private Host(BindingMaterial material, UdpPacketDecoder.AudioFormatV2 format) {
                this.material = material.transfer(); this.format = format;
                byte[] id = randomBytes(8); this.sessionId = ByteBuffer.wrap(id).getLong();
            }
            static HostStart start(BindingMaterial material, UdpPacketDecoder.AudioFormatV2 format) {
                format.encode();
                Host host = new Host(material, format);
                byte[] hello = message(HELLO, 96, host.material.hostId, host.material.receiverId,
                        host.hostNonce, ByteBuffer.allocate(8).putLong(host.sessionId).array(), format.encode());
                return new HostStart(host, hello);
            }
            synchronized HostAwaitReady acceptChallenge(byte[] challenge) throws Failure {
                if (state != 0) throw new IllegalStateException("v2 host state already consumed");
                state = 1;
                try {
                    byte[] body = parse(challenge, CHALLENGE, 104);
                    if (!MessageDigest.isEqual(Arrays.copyOfRange(body, 0, 16), material.receiverId))
                        throw new Failure("wrong v2 receiver identity");
                    byte[] nonce = Arrays.copyOfRange(body, 16, 48);
                    UdpPacketDecoder.AudioFormatV2 selected = parseFormat(Arrays.copyOfRange(body, 48, 64));
                    if (!Arrays.equals(selected.encode(), format.encode())) throw new Failure("v2 selection changed");
                    byte[] transcript = transcript(material, hostNonce, nonce, sessionId, format, selected);
                    receiverProof = Arrays.copyOfRange(body, 64, 96);
                    verify(material.secret, "AnPod P5-A proof receiver v2", transcript, new byte[0], receiverProof);
                    hostProof = mac(material.secret, "proof host", transcript, new byte[0]);
                    HostAwaitReady next = new HostAwaitReady(material, format, sessionId, transcript, receiverProof, hostProof);
                    material = null;
                    return next;
                } catch (Failure | RuntimeException error) { close(); throw error; }
            }
            @Override public synchronized void close() { if (material != null) material.close(); material = null; state = 2; }
        }
        static final class HostStart {
            final Host host; final byte[] hello;
            HostStart(Host host, byte[] hello) { this.host = host; this.hello = hello; }
        }
        static final class HostAwaitReady implements AutoCloseable {
            private BindingMaterial material; private final UdpPacketDecoder.AudioFormatV2 format;
            private final long sessionId; private final byte[] transcript, receiverProof, hostProof;
            final byte[] finish;
            private boolean consumed;
            HostAwaitReady(BindingMaterial material, UdpPacketDecoder.AudioFormatV2 format, long sessionId,
                    byte[] transcript, byte[] receiverProof, byte[] hostProof) {
                this.material = material; this.format = format; this.sessionId = sessionId;
                this.transcript = transcript; this.receiverProof = receiverProof; this.hostProof = hostProof;
                this.finish = message(FINISH, 40, hostProof);
            }
            synchronized AuthenticatedSessionV2 acceptReady(byte[] ready) throws Failure {
                if (consumed) throw new IllegalStateException("v2 host state already consumed"); consumed = true;
                try {
                    byte[] body = parse(ready, READY, 40);
                    verify(material.secret, "AnPod P5-A ready v2", transcript, concat(hostProof, receiverProof), body);
                    return new AuthenticatedSessionV2(sessionId, format, deriveKeys(material.secret, transcript, 2));
                } finally { close(); }
            }
            @Override public synchronized void close() { consumed = true; if (material != null) material.close(); material = null; }
        }
        static final class Receiver {
            static ReceiverAwaitFinish acceptHello(BindingMaterial material, byte[] hello) throws Failure {
                BindingMaterial owned = material.transfer();
                try {
                    byte[] body = parse(hello, HELLO, 96);
                    if (!MessageDigest.isEqual(Arrays.copyOfRange(body, 0, 16), owned.hostId)
                            || !MessageDigest.isEqual(Arrays.copyOfRange(body, 16, 32), owned.receiverId))
                        throw new Failure("wrong v2 peer identity");
                    byte[] hostNonce = Arrays.copyOfRange(body, 32, 64);
                    long id = ByteBuffer.wrap(body, 64, 8).getLong();
                    UdpPacketDecoder.AudioFormatV2 requested = parseFormat(Arrays.copyOfRange(body, 72, 88));
                    byte[] nonce = randomBytes(32);
                    byte[] transcript = transcript(owned, hostNonce, nonce, id, requested, requested);
                    byte[] proof = mac(owned.secret, "proof receiver", transcript, new byte[0]);
                    byte[] challenge = message(CHALLENGE, 104, owned.receiverId, nonce, requested.encode(), proof);
                    return new ReceiverAwaitFinish(owned, requested, id, transcript, proof, challenge);
                } catch (Failure | RuntimeException error) { owned.close(); throw error; }
            }
        }
        static final class ReceiverAwaitFinish implements AutoCloseable {
            private BindingMaterial material; private final UdpPacketDecoder.AudioFormatV2 format;
            private final long sessionId; private final byte[] transcript, receiverProof;
            final byte[] challenge;
            private boolean consumed;
            ReceiverAwaitFinish(BindingMaterial material, UdpPacketDecoder.AudioFormatV2 format,
                    long sessionId, byte[] transcript, byte[] receiverProof, byte[] challenge) {
                this.material = material; this.format = format; this.sessionId = sessionId;
                this.transcript = transcript; this.receiverProof = receiverProof; this.challenge = challenge;
            }
            synchronized FinishResult acceptFinish(byte[] finish) throws Failure {
                if (consumed) throw new IllegalStateException("v2 receiver state already consumed"); consumed = true;
                try {
                    byte[] body = parse(finish, FINISH, 40);
                    verify(material.secret, "AnPod P5-A proof host v2", transcript, new byte[0], body);
                    byte[] ready = mac(material.secret, "ready", transcript, concat(body, receiverProof));
                    byte[] response = message(READY, 40, ready);
                    AuthenticatedSessionV2 session = new AuthenticatedSessionV2(sessionId, format, deriveKeys(material.secret, transcript, 2));
                    return new FinishResult(response, session);
                } finally { close(); }
            }
            @Override public synchronized void close() { consumed = true; if (material != null) material.close(); material = null; }
        }
        static final class FinishResult { final byte[] ready; final AuthenticatedSessionV2 session;
            FinishResult(byte[] ready, AuthenticatedSessionV2 session) { this.ready = ready; this.session = session; } }
        static final class AuthenticatedSessionV2 implements AutoCloseable {
            final long sessionId; final UdpPacketDecoder.AudioFormatV2 format; private SessionKeys keys;
            AuthenticatedSessionV2(long id, UdpPacketDecoder.AudioFormatV2 format, SessionKeys keys) {
                this.sessionId = id; this.format = format; this.keys = keys;
            }
            synchronized SessionKeys takeKeys() { if (keys == null) throw new IllegalStateException("keys consumed"); SessionKeys out = keys; keys = null; return out; }
            @Override public synchronized void close() { if (keys != null) keys.close(); keys = null; }
        }
        static byte[] transcriptForVector(byte[] hostId, byte[] receiverId, byte[] hostNonce,
                byte[] receiverNonce, long sessionId, UdpPacketDecoder.AudioFormatV2 format) {
            BindingMaterial fixture = new BindingMaterial(hostId, receiverId, new byte[32]);
            try { return transcript(fixture, hostNonce, receiverNonce, sessionId, format, format); }
            finally { fixture.close(); }
        }
        static byte[] proofForVector(byte[] secret, String role, byte[] transcript) {
            return mac(secret, "proof " + role, transcript, new byte[0]);
        }
        static byte[] readyForVector(byte[] secret, byte[] transcript, byte[] hostProof, byte[] receiverProof) {
            return mac(secret, "ready", transcript, concat(hostProof, receiverProof));
        }
        static byte[] helloForVector(byte[] hostId, byte[] receiverId, byte[] hostNonce,
                long sessionId, UdpPacketDecoder.AudioFormatV2 format) {
            return message(HELLO, 96, hostId, receiverId, hostNonce,
                    ByteBuffer.allocate(8).putLong(sessionId).array(), format.encode());
        }
        static byte[] challengeForVector(byte[] receiverId, byte[] receiverNonce,
                UdpPacketDecoder.AudioFormatV2 format, byte[] proof) {
            return message(CHALLENGE, 104, receiverId, receiverNonce, format.encode(), proof);
        }
        static byte[] proofMessageForVector(int type, byte[] proof) { return message(type, 40, proof); }
        static SessionKeys deriveForVector(byte[] secret, byte[] transcript) throws Failure { return deriveKeys(secret, transcript, 2); }
    }

}
