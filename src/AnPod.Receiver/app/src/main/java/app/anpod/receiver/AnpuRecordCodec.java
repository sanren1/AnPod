package app.anpod.receiver;

import java.io.IOException;
import java.io.EOFException;
import java.io.InputStream;
import java.io.OutputStream;

/** Pure codec for one complete ANPU record; transport reads and writes stay with the caller. */
final class AnpuRecordCodec {
    static final int HEADER_BYTES = 8;
    static final int MAX_BODY_BYTES = 1200;
    static final int HANDSHAKE = 1;
    static final int CONTROL = 2;
    static final int AUDIO = 3;

    private AnpuRecordCodec() {}

    static final class Record {
        final int type;
        final byte[] body;
        private Record(int type, byte[] body) { this.type = type; this.body = body; }
    }

    static byte[] encode(int type, byte[] body) throws IOException {
        validateBody(type, body);
        byte[] record = new byte[HEADER_BYTES + body.length];
        record[0] = 'A'; record[1] = 'N'; record[2] = 'P'; record[3] = 'U';
        record[4] = 1;
        record[5] = (byte) type;
        record[6] = (byte) (body.length >>> 8);
        record[7] = (byte) body.length;
        System.arraycopy(body, 0, record, HEADER_BYTES, body.length);
        return record;
    }

    /**
     * Reads exactly one record. Stream adapters must bound waits and absorb EAGAIN/ZLP before returning bytes.
     * Any exception after partial consumption is session-fatal; callers must not resynchronize the stream.
     */
    static Record read(InputStream input) throws IOException {
        if (input == null) throw new NullPointerException("input");
        byte[] header = new byte[HEADER_BYTES];
        readFully(input, header, 0, header.length);
        validateHeader(header);
        int bodyLength = ((header[6] & 0xff) << 8) | (header[7] & 0xff);
        byte[] record = new byte[HEADER_BYTES + bodyLength];
        System.arraycopy(header, 0, record, 0, HEADER_BYTES);
        readFully(input, record, HEADER_BYTES, bodyLength);
        return decode(record);
    }

    /**
     * Writes one record through OutputStream's all-or-error contract; native partial writes belong in its adapter.
     * An IOException after partial output is session-fatal; callers must not retry the record on this stream.
     */
    static void write(OutputStream output, int type, byte[] body) throws IOException {
        if (output == null) throw new NullPointerException("output");
        output.write(encode(type, body));
    }

    static Record decode(byte[] bytes) throws IOException {
        if (bytes == null || bytes.length < HEADER_BYTES) throw new IOException("truncated ANPU header");
        validateHeader(bytes);
        int type = bytes[5] & 0xff;
        int bodyLength = ((bytes[6] & 0xff) << 8) | (bytes[7] & 0xff);
        if (bodyLength < 1 || bodyLength > MAX_BODY_BYTES
                || bytes.length != HEADER_BYTES + bodyLength) throw new IOException("invalid ANPU body length");
        byte[] body = new byte[bodyLength];
        System.arraycopy(bytes, HEADER_BYTES, body, 0, bodyLength);
        validateBody(type, body);
        return new Record(type, body);
    }

    private static void validateHeader(byte[] header) throws IOException {
        if (header.length < HEADER_BYTES || header[0] != 'A' || header[1] != 'N'
                || header[2] != 'P' || header[3] != 'U' || (header[4] & 0xff) != 1)
            throw new IOException("invalid ANPU header");
        int type = header[5] & 0xff;
        if (type < HANDSHAKE || type > AUDIO) throw new IOException("unknown ANPU record type");
        int bodyLength = ((header[6] & 0xff) << 8) | (header[7] & 0xff);
        if (bodyLength < 1 || bodyLength > MAX_BODY_BYTES) throw new IOException("invalid ANPU body length");
    }

    private static void readFully(InputStream input, byte[] target, int offset, int length) throws IOException {
        int done = 0;
        while (done < length) {
            int count = input.read(target, offset + done, length - done);
            if (count < 0) throw new EOFException("truncated ANPU record");
            if (count == 0) throw new IOException("ANPU stream read made no progress");
            done += count;
        }
    }

    private static void validateBody(int type, byte[] body) throws IOException {
        if (body == null || body.length < 1 || body.length > MAX_BODY_BYTES)
            throw new IOException("invalid ANPU body length");
        switch (type) {
            case HANDSHAKE: validateHandshake(body); return;
            case CONTROL: validateControl(body); return;
            case AUDIO: validateAudio(body); return;
            default: throw new IOException("unknown ANPU record type");
        }
    }

    private static void validateHandshake(byte[] body) throws IOException {
        if (body.length < 8 || body[0] != 'A' || body[1] != 'N' || body[2] != 'P' || body[3] != '5'
                || (body[4] & 0xff) != 2) throw new IOException("invalid ANPU handshake body");
        int type = body[5] & 0xff;
        int declaredLength = ((body[6] & 0xff) << 8) | (body[7] & 0xff);
        int expectedLength;
        switch (type) {
            case 1: expectedLength = 96; break;
            case 2: expectedLength = 104; break;
            case 3: case 4: expectedLength = 40; break;
            default: throw new IOException("unknown P5-A v2 message type");
        }
        if (body.length != expectedLength || declaredLength != expectedLength)
            throw new IOException("invalid P5-A v2 message length");
    }

    private static void validateControl(byte[] body) throws IOException {
        if (body.length < P5cControlProtocol.HEADER_BYTES + P5cControlProtocol.TAG_BYTES
                || body[0] != 'A' || body[1] != 'N' || body[2] != 'P' || body[3] != 'C'
                || (body[4] & 0xff) != 1 || body[6] != 0 || body[7] != 0)
            throw new IOException("invalid ANPU control body");
        int payloadLength = ((body[28] & 0xff) << 8) | (body[29] & 0xff);
        if (body.length != P5cControlProtocol.HEADER_BYTES + payloadLength + P5cControlProtocol.TAG_BYTES)
            throw new IOException("invalid ANPC payload length");
    }

    private static void validateAudio(byte[] body) throws IOException {
        if (body.length < UdpPacketDecoder.HEADER_BYTES + UdpPacketDecoder.TAG_BYTES
                || body.length > UdpPacketDecoder.MAX_DATAGRAM_BYTES
                || body[0] != 'A' || body[1] != 'N' || body[2] != 'P' || body[3] != 'D'
                || (body[4] & 0xff) != 2 || (body[5] & 0xff) != UdpPacketDecoder.FORMAT
                        && (body[5] & 0xff) != UdpPacketDecoder.AUDIO
                || body[6] != 0 || body[7] != 0)
            throw new IOException("invalid ANPU audio body");
        int payloadLength = ((body[32] & 0xff) << 8) | (body[33] & 0xff);
        if (payloadLength > UdpPacketDecoder.MAX_PLAINTEXT_BYTES
                || body.length != UdpPacketDecoder.HEADER_BYTES + payloadLength + UdpPacketDecoder.TAG_BYTES
                || ((body[5] & 0xff) == UdpPacketDecoder.FORMAT && payloadLength != 16))
            throw new IOException("invalid ANPD payload length");
    }
}
