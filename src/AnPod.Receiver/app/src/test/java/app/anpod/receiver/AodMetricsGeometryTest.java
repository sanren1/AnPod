package app.anpod.receiver;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;

import java.util.List;
import org.junit.Test;

public class AodMetricsGeometryTest {
    @Test
    public void wrapsWholeMetricsWithoutSeparatorsAtRowEdges() {
        List<int[]> rows = AodMetricsGeometry.wrap(new int[] {40, 42, 38, 50}, 12, 100);

        assertArrayEquals(new int[] {0, 2}, rows.get(0));
        assertArrayEquals(new int[] {2, 4}, rows.get(1));
    }

    @Test
    public void centersSeparatorInDistributedGap() {
        assertEquals(10, AodMetricsGeometry.gapBeforeSeparator(20));
        assertEquals(10, AodMetricsGeometry.gapBeforeSeparator(21));
    }
}
