package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class AodMetricsSamplerTest {
    @Test
    public void unresolvedRecoveryTailKeepsOverlappingSourceWindowUnknown() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        com.anpod.server.SourceFrameRanges.Range[] none = new com.anpod.server.SourceFrameRanges.Range[0];
        sampler.recordSourceLossSnapshot(1, 0, 0, 10_000, 1_000, none, true);
        sampler.recordUnknownSourceRange(10_000, 12_000, 1_000);
        sampler.recordSourceLossSnapshot(2, 12_000, 0, 1_000, 1_000, none, true);
        assertNull(sampler.recentSourceLossWindow(10_000_000_000L, 1_000));
        sampler.recordSourceLossSnapshot(2, 12_000, 0, 10_000, 1_000, none, true);
        assertEquals(10_000, sampler.recentSourceLossWindow(10_000_000_000L, 1_000).eligibleFrames);
        assertNull(sampler.recentSourceLossWindow(60_000_000_000L, 1_000));
    }
    @Test
    public void unmappedRecoveryTailExpiresAfterSixtySecondsOfNewSourceFrames() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        com.anpod.server.SourceFrameRanges.Range[] none = new com.anpod.server.SourceFrameRanges.Range[0];
        sampler.recordSourceLossSnapshot(1, 0, 0, 100, 1, none, true);
        sampler.recordSourceLossSnapshot(2, 100, 0, 0, 1, none, true);
        sampler.recordUnknownSourceRange(40, 100, 1);
        assertNull(sampler.recentSourceLossWindow(10_000_000_000L, 1));
        sampler.recordSourceLossSnapshot(2, 100, 0, 10, 1, none, true);
        assertNull(sampler.recentSourceLossWindow(60_000_000_000L, 1));
        sampler.recordSourceLossSnapshot(2, 100, 0, 60, 1, none, true);
        AodMetricsSampler.LossWindow window = sampler.recentSourceLossWindow(60_000_000_000L, 1);
        assertEquals(60, window.eligibleFrames);
        assertEquals(0, window.lossFrames);
    }
    @Test
    public void recoverySegmentKeepsSourceTimeWindowWithoutPauseFrames() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        com.anpod.server.SourceFrameRanges.Range[] priorSegmentLoss = {
                new com.anpod.server.SourceFrameRanges.Range(5_990, 20)};
        com.anpod.server.SourceFrameRanges.Range[] none = new com.anpod.server.SourceFrameRanges.Range[0];
        sampler.recordSourceLossSnapshot(1, 0, 0, 11_000, 1_000, priorSegmentLoss, true);
        sampler.recordSourceLossSnapshot(2, 11_000, 0, 5_000, 1_000, none, true);
        AodMetricsSampler.LossWindow window = sampler.recentSourceLossWindow(10_000_000_000L, 1_000);
        assertEquals(10, window.lossFrames);
        assertEquals(10_000, window.eligibleFrames);
    }
    @Test
    public void delayedLossSettlementUsesSourceFrameWindow() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        com.anpod.server.SourceFrameRanges.Range[] none = new com.anpod.server.SourceFrameRanges.Range[0];
        sampler.recordSourceLossSnapshot(1, 0, 0, 15_000, 1_000, none, true);
        com.anpod.server.SourceFrameRanges.Range[] delayed = {
                new com.anpod.server.SourceFrameRanges.Range(4_990, 20)};
        sampler.recordSourceLossSnapshot(1, 0, 0, 15_000, 1_000, delayed, true);
        AodMetricsSampler.LossWindow window = sampler.recentSourceLossWindow(10_000_000_000L, 1_000);
        assertEquals(10, window.lossFrames);
        assertEquals(10_000, window.eligibleFrames);
    }
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
    public void resetClearsSessionCounters() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeBytes(192_000, 0);

        sampler.reset(5_000_000_000L);
        assertNull(sampler.bitsPerSecond(5_100_000_000L));
    }


    @Test
    public void rateStaysStableBetweenPolls() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeBytes(0, 0);
        sampler.recordCumulativeBytes(192_000, 0);
        assertEquals(Long.valueOf(1_536_000L), sampler.bitsPerSecond(1_000_000_000L));
        assertEquals(Long.valueOf(1_536_000L), sampler.bitsPerSecond(1_250_000_000L));
        assertEquals(Long.valueOf(0L), sampler.bitsPerSecond(2_000_000_000L));
    }

    @Test
    public void bufferLatencyUsesQueuePlusTrackPending() {
        assertEquals(18.0f, AodMetricsSampler.bufferMs(240, 624, 48_000), 0.0001f);
        assertNull(AodMetricsSampler.bufferMs(-1, 100, 48_000));
        assertNull(AodMetricsSampler.bufferMs(10, 20, 0));
        assertNull(AodMetricsSampler.bufferMs(10, -1, 48_000));
    }

    @Test
    public void recentLossWindowFractionallyClipsTheCutoffSample() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeLoss(0, 0, true, 0);
        sampler.recordCumulativeLoss(50, 100, true, 1_000_000_000L);
        for (int second = 2; second <= 60; second++) {
            sampler.recordCumulativeLoss(49 + second, 100L * second, true,
                    second * 1_000_000_000L);
        }

        AodMetricsSampler.LossWindow window = sampler.recentLossWindow(
                60_000_000_000L, 60_500_000_000L);
        assertEquals(84, window.lossFrames);
        assertEquals(5_950, window.eligibleFrames);
        assertEquals(100.0f * 84 / 5_950,
                sampler.recentLossPercent(60_000_000_000L, 60_500_000_000L), 0.0001f);
    }

    @Test
    public void recoveryAndUserPauseExcludeElapsedTimeButKeepLossHistory() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordCumulativeLoss(0, 0, true, 0);
        sampler.recordCumulativeLoss(10, 100, true, 1_000_000_000L);
        sampler.excludeLossTime(10_000_000_000L);
        sampler.recordCumulativeLoss(10, 100, true, 11_000_000_000L);
        sampler.recordCumulativeLoss(11, 200, true, 12_000_000_000L);
        assertEquals(5.5f, sampler.recentLossPercent(2_000_000_000L, 12_000_000_000L), 0.0001f);
        sampler.recordCumulativeLoss(12, 300, true, 13_000_000_000L);
        assertEquals(1.0f, sampler.recentLossPercent(2_000_000_000L, 13_000_000_000L), 0.0001f);
    }
    @Test
    public void lossNeedsAContinuousKnownSixtySecondWindowAndUsesWeightedFrames() {
        AodMetricsSampler sampler = new AodMetricsSampler();
        assertNull(sampler.recordCumulativeLoss(0, 0, true, 0));
        for (int second = 1; second < 60; second++)
            assertNull(sampler.recordCumulativeLoss(second, second * 100L, true,
                    second * 1_000_000_000L));
        assertEquals(Float.valueOf(1.0f), sampler.recordCumulativeLoss(
                60, 6_000, true, 60_000_000_000L));
        assertEquals(Float.valueOf(1.0f), sampler.recentLossPercent(
                10_000_000_000L, 60_000_000_000L));
        assertEquals(Float.valueOf(1.0084034f), sampler.recordCumulativeLoss(
                60, 6_000, true, 60_500_000_000L));
        assertEquals(Float.valueOf(1.15f), sampler.recordCumulativeLoss(
                70, 6_100, true, 61_000_000_000L));
        assertEquals(Float.valueOf(1.9f), sampler.recentLossPercent(
                10_000_000_000L, 61_000_000_000L));
        assertNull(sampler.recordCumulativeLoss(70, 6_100, false, 62_000_000_000L));
        assertNull(sampler.recordCumulativeLoss(70, 6_100, true, 63_000_000_000L));
        for (int second = 64; second < 123; second++)
            assertNull(sampler.recordCumulativeLoss(70 + second - 63,
                    6_100 + (second - 63) * 100L, true, second * 1_000_000_000L));
        assertEquals(Float.valueOf(1.0f), sampler.recordCumulativeLoss(
                130, 12_100, true, 123_000_000_000L));
    }
}
