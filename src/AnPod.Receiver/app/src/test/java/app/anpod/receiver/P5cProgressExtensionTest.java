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
}
