package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class AodMetricsSamplerTest {
    @Test
    public void rateNeedsAFullWindow() {
        AodMetricsSampler sampler = new AodMetricsSampler();

        sampler.recordCumulativeBytes(0, 0);
        sampler.recordCumulativeBytes(192_000, 0);
        assertNull(sampler.bitsPerSecond(499_999_999L));
        assertEquals(Long.valueOf(1_536_000L), sampler.bitsPerSecond(1_000_000_000L));
    }

    @Test
    public void rateDropsToZeroWithoutNewBytes() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeBytes(0, 0);
        sampler.recordCumulativeBytes(192_000, 0);
        sampler.bitsPerSecond(1_000_000_000L);

        assertEquals(Long.valueOf(0L), sampler.bitsPerSecond(2_000_000_000L));
    }

    @Test
    public void cumulativeBytesBecomeDeltas() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeBytes(0, 0);
        sampler.recordCumulativeBytes(96_000, 0);
        sampler.recordCumulativeBytes(192_000, 400_000_000L);

        assertEquals(Long.valueOf(1_536_000L), sampler.bitsPerSecond(1_000_000_000L));
    }

    @Test
    public void silenceStillCountsAsPayload() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeBytes(0, 0);
        sampler.recordCumulativeBytes(192_000, 0);

        assertEquals(Long.valueOf(1_536_000L), sampler.bitsPerSecond(1_000_000_000L));
        assertTrue(sampler.receivedAudioRecently(1_200_000_000L, 2_000_000_000L));
        assertFalse(sampler.receivedAudioRecently(3_500_000_000L, 2_000_000_000L));
    }

    @Test
    public void lossUsesConcealedOverExpected() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        assertNull(sampler.lossPercent());

        sampler.updateFrameTotals(900, 100);
        assertEquals(10.0f, sampler.lossPercent(), 0.0001f);
    }

    @Test
    public void resetClearsSessionCounters() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.updateFrameTotals(900, 100);
        sampler.recordCumulativeBytes(192_000, 0);

        sampler.reset(5_000_000_000L);
        assertNull(sampler.lossPercent());
        assertNull(sampler.bitsPerSecond(5_100_000_000L));
    }

    @Test
    public void bufferLatencyUsesQueuePlusTrackPending() {
        assertEquals(18.0f, AodMetricsSampler.bufferMs(240, 624, 48_000), 0.0001f);
        assertNull(AodMetricsSampler.bufferMs(-1, 100, 48_000));
        assertNull(AodMetricsSampler.bufferMs(10, 20, 0));
        assertNull(AodMetricsSampler.bufferMs(10, -1, 48_000));
    }
}
