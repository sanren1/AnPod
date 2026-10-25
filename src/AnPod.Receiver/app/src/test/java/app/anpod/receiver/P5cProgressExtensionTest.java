package app.anpod.receiver;

import static org.junit.Assert.assertEquals;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import org.junit.Test;

public final class P5cProgressExtensionTest {
    @Test public void schema2UsesFixed126ByteTailAfterEitherProgressBase() {
        long[] counters = new long[12];
        for (int i = 0; i < counters.length; i++) counters[i] = i + 1;
        byte[] legacy = new byte[70];
        byte[] extended = P5cControlProtocol.playbackProgressPayloadExtended(legacy,
                101, 202, 3, 2, counters, 17, 23);
        assertEquals(196, extended.length);
        assertEquals(2, extended[70] & 0xff);
        ByteBuffer fields = ByteBuffer.wrap(extended).order(ByteOrder.BIG_ENDIAN);
        assertEquals(101, fields.getLong(71));
        assertEquals(202, fields.getLong(79));
        assertEquals(3, fields.getInt(87));
        assertEquals(2, extended[91] & 0xff);
        for (int i = 0; i < counters.length; i++) assertEquals(counters[i], fields.getLong(92 + i * 8));
        assertEquals(17, fields.getInt(188));
        assertEquals(23, fields.getInt(192));

        assertEquals(204, P5cControlProtocol.playbackProgressPayloadExtended(new byte[78],
                101, 202, 3, 2, counters, 17, 23).length);
    }

    @Test public void schema3AddsFixed17ByteBufferMetricsTail() {
        long[] counters = new long[12];
        byte[] schema2 = P5cControlProtocol.playbackProgressPayloadExtended(new byte[78],
                101, 202, 3, 2, counters, 17, 23);
        byte[] schema3 = P5cControlProtocol.playbackProgressPayloadSchema3(schema2,
                1920, 1955, 35, 420, 4);
        assertEquals(221, schema3.length);
        assertEquals(3, schema3[78] & 0xff);
        ByteBuffer fields = ByteBuffer.wrap(schema3).order(ByteOrder.BIG_ENDIAN);
        assertEquals(1920, fields.getInt(204));
        assertEquals(1955, fields.getInt(208));
        assertEquals(35, fields.getInt(212));
        assertEquals(420, fields.getInt(216));
        assertEquals(4, schema3[220] & 0xff);

        byte[] schema3V1 = P5cControlProtocol.playbackProgressPayloadSchema3(
                P5cControlProtocol.playbackProgressPayloadExtended(new byte[70],
                        101, 202, 3, 2, counters, 17, 23), -1, -1,
                Integer.MIN_VALUE, Integer.MIN_VALUE, 0);
        assertEquals(213, schema3V1.length);
        assertEquals(-1, ByteBuffer.wrap(schema3V1).order(ByteOrder.BIG_ENDIAN).getInt(196));
        assertEquals(Integer.MIN_VALUE, ByteBuffer.wrap(schema3V1).order(ByteOrder.BIG_ENDIAN).getInt(204));
    }
}
