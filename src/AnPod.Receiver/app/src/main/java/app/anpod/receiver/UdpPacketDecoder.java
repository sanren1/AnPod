package app.anpod.receiver;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import javax.crypto.AEADBadTagException;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/** Decoder for the fixed P3 ANPD v1 host-to-receiver AES-GCM datagram. */
final class UdpPacketDecoder {
    static final int HEADER_BYTES = 34;
    static final int TAG_BYTES = 16;
    static final int MAX_DATAGRAM_BYTES = 1200;
    static final int MAX_PLAINTEXT_BYTES = MAX_DATAGRAM_BYTES - HEADER_BYTES - TAG_BYTES;
    static final int FORMAT = 1;
    static final int AUDIO = 2;
    private static final int NONCE_DOMAIN = 0x414e5048; // ANPH

    private final long expectedSessionId;
    private final byte[] key;
    private boolean hasHighest;
    private long highestSequence;
    private long replayBits;

    UdpPacketDecoder(long expectedSessionId, byte[] key) {
        if (key == null || key.length != 32) throw new IllegalArgumentException("AES-256 requires a 32-byte key");
        this.expectedSessionId = expectedSessionId;
        this.key = key.clone();
    }

    synchronized Packet open(byte[] datagram, int length) throws ProtocolException {
        if (datagram == null || length < HEADER_BYTES + TAG_BYTES || length > MAX_DATAGRAM_BYTES
                || length > datagram.length) throw new ProtocolException(ProtocolException.LENGTH);
        if (datagram[0] != 'A' || datagram[1] != 'N' || datagram[2] != 'P' || datagram[3] != 'D')
            throw new ProtocolException(ProtocolException.HEADER);
        if ((datagram[4] & 0xff) != 1) throw new ProtocolException(ProtocolException.VERSION);
        int type = datagram[5] & 0xff;
        if (type != FORMAT && type != AUDIO) throw new ProtocolException(ProtocolException.TYPE);
        if (datagram[6] != 0 || datagram[7] != 0) throw new ProtocolException(ProtocolException.FLAGS);

        ByteBuffer fields = ByteBuffer.wrap(datagram, 8, 26).order(ByteOrder.BIG_ENDIAN);
        long sessionId = fields.getLong();
        long sequence = fields.getLong();
        long sampleFrameIndex = fields.getLong();
        int payloadLength = fields.getShort() & 0xffff;
        if (payloadLength > MAX_PLAINTEXT_BYTES || length != HEADER_BYTES + payloadLength + TAG_BYTES)
            throw new ProtocolException(ProtocolException.LENGTH);
        if (sessionId != expectedSessionId) throw new ProtocolException(ProtocolException.IDENTITY);

        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN)
                .putInt(NONCE_DOMAIN).putLong(sequence).array();
        byte[] plaintext;
        try {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
            cipher.updateAAD(datagram, 0, HEADER_BYTES);
            plaintext = cipher.doFinal(datagram, HEADER_BYTES, length - HEADER_BYTES);
        } catch (AEADBadTagException error) {
            throw new ProtocolException(ProtocolException.AUTHENTICATION, error);
        } catch (Exception error) {
            throw new ProtocolException(ProtocolException.AUTHENTICATION, error);
        }
        if (!wouldAccept(sequence)) throw new ProtocolException(ProtocolException.REPLAY);
        return new Packet(type, sessionId, sequence, sampleFrameIndex, plaintext);
    }

    synchronized void commit(long sequence) {
        if (!hasHighest) {
            hasHighest = true;
            highestSequence = sequence;
            replayBits = 1;
            return;
        }
        if (Long.compareUnsigned(sequence, highestSequence) > 0) {
            long advance = sequence - highestSequence;
            replayBits = Long.compareUnsigned(advance, 64L) >= 0 ? 1 : (replayBits << (int) advance) | 1;
            highestSequence = sequence;
        } else {
            replayBits |= 1L << (int) (highestSequence - sequence);
        }
    }

    synchronized void destroy() {
        Arrays.fill(key, (byte) 0);
        replayBits = 0;
        hasHighest = false;
        highestSequence = 0;
    }

    private boolean wouldAccept(long sequence) {
        if (!hasHighest) return true;
        if (Long.compareUnsigned(sequence, highestSequence) > 0) return true;
        long behind = highestSequence - sequence;
        return Long.compareUnsigned(behind, 64L) < 0 && (replayBits & (1L << (int) behind)) == 0;
    }

    static Format decodeFormat(Packet packet) throws ProtocolException {
        byte[] payload = packet.payload;
        if (packet.type != FORMAT || packet.sampleFrameIndex != 0 || payload.length != 7 || payload[0] != 1)
            throw new ProtocolException(ProtocolException.FORMAT);
        int channels = ((payload[1] & 0xff) << 8) | (payload[2] & 0xff);
        long rate = ((long) (payload[3] & 0xff) << 24) | ((long) (payload[4] & 0xff) << 16)
                | ((long) (payload[5] & 0xff) << 8) | (payload[6] & 0xff);
        if ((channels != 1 && channels != 2) || rate < 8000 || rate > 192000)
            throw new ProtocolException(ProtocolException.FORMAT);
        return new Format((int) rate, channels);
    }

    static final class Packet {
        final int type;
        final long sessionId;
        final long sequence;
        final long sampleFrameIndex;
        final byte[] payload;

        Packet(int type, long sessionId, long sequence, long sampleFrameIndex, byte[] payload) {
            this.type = type;
            this.sessionId = sessionId;
            this.sequence = sequence;
            this.sampleFrameIndex = sampleFrameIndex;
            this.payload = payload;
        }
    }

    static final class Format {
        final int sampleRate;
        final int channels;

        Format(int sampleRate, int channels) {
            this.sampleRate = sampleRate;
            this.channels = channels;
        }
    }

    static final class ProtocolException extends IOException {
        static final int HEADER = 1;
        static final int VERSION = 2;
        static final int TYPE = 3;
        static final int FLAGS = 4;
        static final int IDENTITY = 5;
        static final int LENGTH = 6;
        static final int AUTHENTICATION = 7;
        static final int REPLAY = 8;
        static final int FORMAT = 9;
        static final int AUDIO = 10;
        final int reason;

        ProtocolException(int reason) { this.reason = reason; }
        ProtocolException(int reason, Throwable cause) { super(cause); this.reason = reason; }
    }
}
