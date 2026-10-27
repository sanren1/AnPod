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

    @Test public void schema4AddsAppliedLossLedgerRevisionAfterSchema3Tail() {
        long[] counters = new long[12];
        byte[] schema2 = P5cControlProtocol.playbackProgressPayloadExtended(new byte[78],
                101, 202, 3, 2, counters, 17, 23);
        byte[] schema3 = P5cControlProtocol.playbackProgressPayloadSchema3(schema2,
                1920, 1955, 35, 420, 4);
        byte[] schema4 = P5cControlProtocol.playbackProgressPayloadSchema4(schema3, 42);
        assertEquals(229, schema4.length);
        assertEquals(4, schema4[78] & 0xff);
        assertEquals(42, ByteBuffer.wrap(schema4).order(ByteOrder.BIG_ENDIAN).getLong(221));
    }
    @Test public void schema5AddsTenSecondSixtySecondAndCumulativeLossPairs() {
        long[] counters = new long[12];
        byte[] schema2 = P5cControlProtocol.playbackProgressPayloadExtended(new byte[78],
                101, 202, 3, 2, counters, 17, 23);
        byte[] schema3 = P5cControlProtocol.playbackProgressPayloadSchema3(schema2,
                1920, 1955, 35, 420, 4);
        byte[] schema4 = P5cControlProtocol.playbackProgressPayloadSchema4(schema3, 42);
        byte[] schema5 = P5cControlProtocol.playbackProgressPayloadSchema5(schema4,
                new AodMetricsSampler.LossWindow(10, 100),
                new AodMetricsSampler.LossWindow(20, 200),
                new AodMetricsSampler.LossWindow(30, 300));
        assertEquals(277, schema5.length);
        assertEquals(5, schema5[78] & 0xff);
        ByteBuffer fields = ByteBuffer.wrap(schema5).order(ByteOrder.BIG_ENDIAN);
        assertEquals(42, fields.getLong(221));
        assertEquals(10, fields.getLong(229));
        assertEquals(100, fields.getLong(237));
        assertEquals(20, fields.getLong(245));
        assertEquals(200, fields.getLong(253));
        assertEquals(30, fields.getLong(261));
        assertEquals(300, fields.getLong(269));

        byte[] unknown = P5cControlProtocol.playbackProgressPayloadSchema5(schema4, null, null, null);
        ByteBuffer unknownFields = ByteBuffer.wrap(unknown).order(ByteOrder.BIG_ENDIAN);
        for (int offset = 229; offset < 277; offset += 8) assertEquals(-1L, unknownFields.getLong(offset));
    }
    @Test public void schema6AddsLossBreakdownAndRecoveryTotals() {
        long[] counters = new long[12];
        byte[] schema2 = P5cControlProtocol.playbackProgressPayloadExtended(new byte[78],
                101, 202, 3, 2, counters, 17, 23);
        byte[] schema3 = P5cControlProtocol.playbackProgressPayloadSchema3(schema2,
                1920, 1955, 35, 420, 4);
        byte[] schema4 = P5cControlProtocol.playbackProgressPayloadSchema4(schema3, 42);
        byte[] schema5 = P5cControlProtocol.playbackProgressPayloadSchema5(schema4, null, null, null);
        byte[] schema6 = P5cControlProtocol.playbackProgressPayloadSchema6(schema5, 1, 2, 3, 4, 5, 6);
        assertEquals(325, schema6.length);
        assertEquals(6, schema6[78] & 0xff);
        ByteBuffer fields = ByteBuffer.wrap(schema6).order(ByteOrder.BIG_ENDIAN);
        for (int index = 0; index < 6; index++) assertEquals(index + 1, fields.getLong(277 + index * 8));
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
