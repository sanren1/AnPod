package app.anpod.receiver;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.net.SocketTimeoutException;
import java.util.Arrays;
import java.util.concurrent.TimeUnit;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/** Fixed P5-C authenticated control frame shared with the Rust host. */
final class P5cControlProtocol {
    static final int HOST_TO_RECEIVER = 0x414e5048;
    static final int RECEIVER_TO_HOST = 0x414e5052;
    static final int HEADER_BYTES = 30;
    static final int TAG_BYTES = 32;
    static final int MAX_FRAME_BYTES = 1200;
    static final int BIND = 1;
    static final int SELECT_MODE = 2;
    static final int START = 3;
    static final int PAUSE = 4;
    static final int RESUME = 5;
    static final int STOP = 6;
    static final int HEARTBEAT = 7;
    static final int CAPABILITIES_QUERY = 8;
    static final int CAPABILITIES_INFO = 9;
    static final int PLAYBACK_PROGRESS_QUERY = 16;
    static final int CONNECTION_INTENT = 17;
    static final int ACK = 127;
    static final int MANAGEMENT_CONTROL_MAJOR = 3;
    static final int AUDIO_PROTOCOL_MAJOR = 1;
    static final int CAP_RECEIVER_UI = 0x00000001;
    static final int CAP_AUTHENTICATED_STATUS = 0x00000002;
    static final int CAP_PLAYBACK_PROGRESS = 0x00000008;
    static final int CAP_CONNECTION_INTENT = 0x00000010;
    static final int IMPLEMENTED_CAPABILITIES = CAP_RECEIVER_UI | CAP_AUTHENTICATED_STATUS;
    private static final long FRAME_READ_TIMEOUT_NANOS = TimeUnit.SECONDS.toNanos(3);

    private P5cControlProtocol() {}

    static byte[] encode(byte[] key, int direction, long sessionId, long sequence, int type, byte[] payload)
            throws IOException {
        if (key == null || key.length != 32 || payload.length > MAX_FRAME_BYTES - HEADER_BYTES - TAG_BYTES)
            throw new IOException("invalid P5-C control frame");
        byte[] frame = new byte[HEADER_BYTES + payload.length + TAG_BYTES];
        frame[0] = 'A'; frame[1] = 'N'; frame[2] = 'P'; frame[3] = 'C';
        frame[4] = 1; frame[5] = (byte) type;
        frame[6] = 0; frame[7] = 0;
        putInt(frame, 8, direction);
        putLong(frame, 12, sessionId);
        putLong(frame, 20, sequence);
        frame[28] = (byte) (payload.length >>> 8);
        frame[29] = (byte) payload.length;
        System.arraycopy(payload, 0, frame, HEADER_BYTES, payload.length);
        byte[] tag = mac(key, frame, HEADER_BYTES + payload.length);
        System.arraycopy(tag, 0, frame, HEADER_BYTES + payload.length, TAG_BYTES);
        Arrays.fill(tag, (byte) 0);
        return frame;
    }

    static Frame read(DataInputStream input, byte[] key, int direction, long sessionId) throws IOException {
        int high;
        try { high = input.read(); }
        catch (SocketTimeoutException idle) { throw new IdleTimeoutException(); }
        if (high < 0) throw new EOFException();
        long frameStartedAt = System.nanoTime();
        int low = readFrameByte(input, frameStartedAt);
        ensureFrameDeadline(frameStartedAt);
        int length = (high << 8) | low;
        if (length < HEADER_BYTES + TAG_BYTES || length > MAX_FRAME_BYTES)
            throw new IOException("invalid P5-C control length");
        byte[] frame = new byte[length];
        int offset = 0;
        while (offset < frame.length) {
            try {
                int count = input.read(frame, offset, frame.length - offset);
                if (count < 0) throw new EOFException();
                if (count > 0) offset += count;
                ensureFrameDeadline(frameStartedAt);
            } catch (SocketTimeoutException partialFrameTimeout) {
                if (System.nanoTime() - frameStartedAt >= FRAME_READ_TIMEOUT_NANOS)
                    throw new SocketTimeoutException("P5-C control frame read deadline expired");
            }
        }
        int payloadLength = ((frame[28] & 0xff) << 8) | (frame[29] & 0xff);
        if (frame[0] != 'A' || frame[1] != 'N' || frame[2] != 'P' || frame[3] != 'C'
                || frame[4] != 1 || frame[6] != 0 || frame[7] != 0
                || getInt(frame, 8) != direction || getLong(frame, 12) != sessionId
                || length != HEADER_BYTES + payloadLength + TAG_BYTES)
            throw new IOException("invalid P5-C control identity or length");
        byte[] expected = mac(key, frame, HEADER_BYTES + payloadLength);
        boolean authenticated = MessageDigest.isEqual(expected,
                Arrays.copyOfRange(frame, HEADER_BYTES + payloadLength, length));
        Arrays.fill(expected, (byte) 0);
        if (!authenticated) throw new IOException("P5-C control authentication failed");
        return new Frame(frame, frame[5] & 0xff, getLong(frame, 20),
                Arrays.copyOfRange(frame, HEADER_BYTES, HEADER_BYTES + payloadLength));
    }

    private static int readFrameByte(DataInputStream input, long frameStartedAt) throws IOException {
        while (true) {
            try {
                int value = input.read();
                if (value < 0) throw new EOFException();
                ensureFrameDeadline(frameStartedAt);
                return value;
            } catch (SocketTimeoutException partialFrameTimeout) {
                if (System.nanoTime() - frameStartedAt >= FRAME_READ_TIMEOUT_NANOS)
                    throw new SocketTimeoutException("P5-C control frame read deadline expired");
            }
        }
    }

    private static void ensureFrameDeadline(long frameStartedAt) throws SocketTimeoutException {
        if (System.nanoTime() - frameStartedAt >= FRAME_READ_TIMEOUT_NANOS)
            throw new SocketTimeoutException("P5-C control frame read deadline expired");
    }

    static final class IdleTimeoutException extends SocketTimeoutException {
        private static final long serialVersionUID = 1L;
    }

    static void write(DataOutputStream output, byte[] frame) throws IOException {
        if (frame.length < HEADER_BYTES + TAG_BYTES || frame.length > MAX_FRAME_BYTES)
            throw new IOException("invalid P5-C control length");
        output.writeShort(frame.length);
        output.write(frame);
        output.flush();
    }

    static byte[] ackPayload(int commandType, long commandSequence, int result, int state, int udpPort) {
        return ackPayload(commandType, commandSequence, result, state, udpPort, new byte[0]);
    }

    static byte[] ackPayload(int commandType, long commandSequence, int result, int state, int udpPort, byte[] extra) {
        byte[] payload = new byte[13];
        payload[0] = (byte) commandType;
        putLong(payload, 1, commandSequence);
        payload[9] = (byte) result;
        payload[10] = (byte) state;
        payload[11] = (byte) (udpPort >>> 8);
        payload[12] = (byte) udpPort;
        byte[] combined = Arrays.copyOf(payload, payload.length + extra.length);
        System.arraycopy(extra, 0, combined, payload.length, extra.length);
        return combined;
    }

    static byte[] playbackProgressPayload(int sampleRate, int channels, int jitterTargetFrames,
            long expiredFrames, long queuedFrames, long pendingFrames, int underrunCount,
            long tcpDroppedFrames, Boolean playbackAdvancing, int requestedFrames,
            int acceptedFrames, int effectiveFrames, int capacityFrames, int startThresholdFrames,
            int preRollFrames) {
        byte[] payload = new byte[70];
        putInt(payload, 0, sampleRate);
        payload[4] = (byte) channels;
        putInt(payload, 5, jitterTargetFrames);
        putLong(payload, 9, expiredFrames);
        putLong(payload, 17, queuedFrames);
        putLong(payload, 25, pendingFrames);
        putInt(payload, 33, underrunCount);
        putLong(payload, 37, tcpDroppedFrames);
        payload[45] = playbackAdvancing == null ? (byte) 0xff : (byte) (playbackAdvancing ? 1 : 0);
        putInt(payload, 46, requestedFrames);
        putInt(payload, 50, acceptedFrames);
        putInt(payload, 54, effectiveFrames);
        putInt(payload, 58, capacityFrames);
        putInt(payload, 62, startThresholdFrames);
        putInt(payload, 66, preRollFrames);
        return payload;
    }

    static byte[] capabilitiesQueryPayload(int controlMajor, int audioMajor, int requiredCapabilities) {
        byte[] payload = new byte[8];
        payload[0] = (byte) (controlMajor >>> 8); payload[1] = (byte) controlMajor;
        payload[2] = (byte) audioMajor;
        putInt(payload, 4, requiredCapabilities);
        return payload;
    }

    static byte[] capabilitiesInfoPayload(long querySequence, int controlMajor, int audioMajor,
            int result, int capabilities, int versionCode) {
        byte[] payload = new byte[20];
        putLong(payload, 0, querySequence);
        payload[8] = (byte) (controlMajor >>> 8); payload[9] = (byte) controlMajor;
        payload[10] = (byte) audioMajor; payload[11] = (byte) result;
        putInt(payload, 12, capabilities); putInt(payload, 16, versionCode);
        return payload;
    }

    static int getUnsignedShort(byte[] bytes, int offset) { return ((bytes[offset] & 0xff) << 8) | (bytes[offset + 1] & 0xff); }
    static long getLong(byte[] bytes, int offset) {
        long value = 0;
        for (int i = 0; i < 8; i++) value = (value << 8) | (bytes[offset + i] & 0xffL);
        return value;
    }
    private static int getInt(byte[] bytes, int offset) {
        return ((bytes[offset] & 0xff) << 24) | ((bytes[offset + 1] & 0xff) << 16)
                | ((bytes[offset + 2] & 0xff) << 8) | (bytes[offset + 3] & 0xff);
    }
    static int getUnsignedInt(byte[] bytes, int offset) {
        return getInt(bytes, offset);
    }
    static long getUnsignedLong(byte[] bytes, int offset) {
        long value = getLong(bytes, offset);
        if (value < 0) throw new IllegalArgumentException("unsigned value exceeds supported range");
        return value;
    }
    private static void putInt(byte[] bytes, int offset, int value) {
        for (int i = 3; i >= 0; i--) { bytes[offset + i] = (byte) value; value >>>= 8; }
    }
    private static void putLong(byte[] bytes, int offset, long value) {
        for (int i = 7; i >= 0; i--) { bytes[offset + i] = (byte) value; value >>>= 8; }
    }
    private static byte[] mac(byte[] key, byte[] frame, int length) throws IOException {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            mac.update(frame, 0, length);
            return mac.doFinal();
        } catch (GeneralSecurityException error) {
            throw new IOException("P5-C HMAC unavailable", error);
        }
    }

    static final class Frame {
        final byte[] raw;
        final int type;
        final long sequence;
        final byte[] payload;
        Frame(byte[] raw, int type, long sequence, byte[] payload) {
            this.raw = raw; this.type = type; this.sequence = sequence; this.payload = payload;
        }
    }
}
