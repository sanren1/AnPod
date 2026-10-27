package app.anpod.receiver;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import com.anpod.server.Pcm16FrameStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;
import org.junit.Test;

public final class AnpdLossLedgerTest {
    private static final String PAYLOAD = "02010200000000000000000000000000000000070000000000000010000000000000000c00000000000000020000000000000003000000000000000a0000000000000005";
    private static final String PACKET = "414e504402030000010203040506070811121314151617180000000000000000004439d358e23c1d8fd383c7563d031edfd12b1063d3f80313eec3aea4c10383b03b6f607fdc9ab3f8c9c25c8e55a8f4af8cef3ab8935778a88b531232564293eedaa6de282391c8fc7f73a59562fb3445bd3982eb58";
    private static final String V1_PACKET = "414e504401030000010203040506070811121314151617180000000000000000004439d358e23c1d8fd383c7563d031edfd12b1063d3f80313eec3aea4c10383b03b6f607fdc9ab3f8c9c25c8e55a8f4af8cef3ab8935778a88b531232564293eedaa6de282354f421be4179f1a3887ce761218d3455";

    @Test public void lossLedgerPayloadMatchesRustVector() throws Exception {
        byte[] expected = hex(PAYLOAD);
        AnpdLossLedger ledger = AnpdLossLedger.decode(expected);
        assertEquals(7, ledger.revision);
        assertEquals(16, ledger.captureTail);
        assertEquals(12, ledger.settledThrough);
        assertEquals(2, ledger.intervals.length);
        assertEquals(false, ledger.hasSourceSegmentStart);
    }

    @Test public void authenticatedResumeLedgerExcludesPauseHoleAndNewSessionCountsPrefixDrops() throws Exception {
        byte[] key = new byte[32];
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(1, 0);
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment(0, 0);
        AodMetricsSampler sampler = new AodMetricsSampler();
        try (AoaSessionContext context = new AoaSessionContext(7, key, format)) {
            ReceiverService.AudioSession session = new ReceiverService.AudioSession(1,
                    ReceiverService.AudioRequest.aoa(context), "owner", 0, map, sampler);
            byte[] descriptor = packet(key, 7, 0, UdpPacketDecoder.FORMAT, 0, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT,
                    context.ingress.accept(descriptor, descriptor.length));
            byte[] firstLedger = packet(key, 7, 1, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshotV3(1, 112, 112, 0, 0, new long[][] {{110, 2}}));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER,
                    context.ingress.accept(firstLedger, firstLedger.length));
            ReceiverService.accountSettledPcDropGaps(session);
            map.recordWrite(0, 110, false);
            map.advancePlayed(110);
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(2, map.settledPcDropFrames());
            map.resetOutputEpoch(true);
            map.beginSourceSegment(112, 112);

            byte[] resumedLedger = packet(key, 7, 2, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshotV3(2, 135, 130, 1, 130, new long[][] {{110, 2}}));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER,
                    context.ingress.accept(resumedLedger, resumedLedger.length));
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(130, map.sourceSegmentOrigin());
            map.recordWrite(130, 5, false);
            map.advancePlayed(5);
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(0, map.settledTransportMissingFrames());
            assertEquals(2, map.settledPcDropFrames());
            assertEquals(117, map.settledEligibleSourceFrames());
        }

        try (AoaSessionContext context = new AoaSessionContext(8, key, format)) {
            ReceiverService.AudioSession session = new ReceiverService.AudioSession(2,
                    ReceiverService.AudioRequest.aoa(context), "owner", 0, map, sampler);
            byte[] descriptor = packet(key, 8, 0, UdpPacketDecoder.FORMAT, 0, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT,
                    context.ingress.accept(descriptor, descriptor.length));
            byte[] initialLedger = packet(key, 8, 1, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshotV3(1, 900, 900, 0, 0, new long[][] {{0, 100}}));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER,
                    context.ingress.accept(initialLedger, initialLedger.length));
            map.beginSourceSegment(-1, -1);
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(0, map.sourceSegmentOrigin());
            map.recordWrite(100, 800, false);
            map.advancePlayed(800);
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(102, map.settledPcDropFrames());
            assertEquals(915, map.settledGoodSourceFrames());
            assertEquals(1_017, map.settledEligibleSourceFrames());
        }
    }

    @Test public void authenticatedFloorCountsPureTransportPrefixWithEmptyPcDropLedger() throws Exception {
        byte[] key = new byte[32];
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(1, 0);
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment(0, 0);
        AodMetricsSampler sampler = new AodMetricsSampler();
        try (AoaSessionContext context = new AoaSessionContext(17, key, format)) {
            ReceiverService.AudioSession session = new ReceiverService.AudioSession(17,
                    ReceiverService.AudioRequest.aoa(context), "owner", 0, map, sampler);
            byte[] descriptor = packet(key, 17, 0, UdpPacketDecoder.FORMAT, 0, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT,
                    context.ingress.accept(descriptor, descriptor.length));
            byte[] ledger = packet(key, 17, 1, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshotV3(1, 1_000, 1_000, 0, 0, new long[0][0]));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER,
                    context.ingress.accept(ledger, ledger.length));
            ReceiverService.accountSettledPcDropGaps(session);
            map.recordWrite(100, 900, false);
            map.advancePlayed(900);
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(0, map.settledPcDropFrames());
            assertEquals(100, map.settledTransportMissingFrames());
            assertEquals(900, map.settledGoodSourceFrames());
            assertEquals(1_000, map.settledEligibleSourceFrames());
            assertEquals(100, map.settledLossFrames());
        }

        SourceFrameOutputMap freshMap = new SourceFrameOutputMap();
        AodMetricsSampler freshSampler = new AodMetricsSampler();
        try (AoaSessionContext context = new AoaSessionContext(18, key, format)) {
            ReceiverService.AudioSession session = new ReceiverService.AudioSession(18,
                    ReceiverService.AudioRequest.aoa(context), "owner", 0, freshMap, freshSampler);
            byte[] descriptor = packet(key, 18, 0, UdpPacketDecoder.FORMAT, 0, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT,
                    context.ingress.accept(descriptor, descriptor.length));
            byte[] ledger = packet(key, 18, 1, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshotV3(1, 1_000, 1_000, 0, 0, new long[0][0]));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER,
                    context.ingress.accept(ledger, ledger.length));
            freshMap.beginSourceSegment(-1, -1);
            ReceiverService.accountSettledPcDropGaps(session);
            freshMap.recordWrite(100, 900, false);
            freshMap.advancePlayed(900);
            ReceiverService.accountSettledPcDropGaps(session);
            assertEquals(100, freshMap.settledTransportMissingFrames());
            assertEquals(900, freshMap.settledGoodSourceFrames());
            assertEquals(1_000, freshMap.settledEligibleSourceFrames());
            assertEquals(100, freshMap.settledLossFrames());
        }
    }

    @Test public void unknownRangeSplitsSettledTransportGapAndAgesOutOfWindow() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        AodMetricsSampler sampler = new AodMetricsSampler();
        map.beginSourceSegment(0);
        map.recordWrite(0, 100, false);
        map.recordUnknownWrite(200, 100, 100);
        map.recordWrite(300, 100, false);
        map.advancePlayed(300);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 400, true, true, true);
        assertEquals(100, map.settledTransportMissingFrames());
        assertEquals(1, map.settledUnknownRanges().length);
        assertEquals(200, map.settledUnknownRanges()[0].start);
        assertEquals(100, map.settledUnknownRanges()[0].frameCount);
        sampler.recordSourceLossSnapshot(map.sourceSegmentId(), map.sourceSegmentLogicalStart(),
                map.sourceSegmentOrigin(), map.settledEligibleSourceThrough(), 100,
                map.settledLossRanges(), map.rollingComplete(), map.settledUnknownRanges());
        assertNull(sampler.recentSourceLossWindow(10_000_000_000L, 100));

        map.recordWrite(400, 6_000, false);
        map.advancePlayed(6_300);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 6_400, true, true, true);
        sampler.recordSourceLossSnapshot(map.sourceSegmentId(), map.sourceSegmentLogicalStart(),
                map.sourceSegmentOrigin(), map.settledEligibleSourceThrough(), 100,
                map.settledLossRanges(), map.rollingComplete(), map.settledUnknownRanges());
        AodMetricsSampler.LossWindow aged = sampler.recentSourceLossWindow(60_000_000_000L, 100);
        assertEquals(0, aged.lossFrames);
        assertEquals(6_000, aged.eligibleFrames);
    }

    @Test public void opensRustAuthenticatedV2LossLedgerPacket() throws Exception {
        byte[] key = new byte[32];
        for (int i = 0; i < key.length; i++) key[i] = (byte) i;
        UdpPacketDecoder decoder = UdpPacketDecoder.newV2(0x0102030405060708L, key,
                UdpPacketDecoder.AudioFormatV2.forProfile(1, 0));
        UdpPacketDecoder.Packet packet = decoder.open(hex(PACKET), hex(PACKET).length);
        assertEquals(UdpPacketDecoder.LOSS_LEDGER, packet.type);
        assertEquals(0x1112131415161718L, packet.sequence);
        assertArrayEquals(hex(PAYLOAD), packet.payload);
        decoder.commit(packet.sequence);
    }

    @Test public void opensRustAuthenticatedV1LossLedgerPacket() throws Exception {
        byte[] key = new byte[32];
        for (int i = 0; i < key.length; i++) key[i] = (byte) i;
        UdpPacketDecoder decoder = new UdpPacketDecoder(0x0102030405060708L, key);
        byte[] encoded = hex(V1_PACKET);
        UdpPacketDecoder.Packet packet = decoder.open(encoded, encoded.length);
        assertEquals(UdpPacketDecoder.LOSS_LEDGER, packet.type);
        assertEquals(0, packet.sampleFrameIndex);
        assertArrayEquals(hex(PAYLOAD), packet.payload);
        decoder.commit(packet.sequence);
    }

    @Test public void higherPeriodEpochResetsRangesAndIgnoresDelayedOldEpoch() throws Exception {
        AnpdLossLedger.Store store = new AnpdLossLedger.Store();
        store.apply(AnpdLossLedger.decode(snapshot(0, 1, 10, 10,
                new long[][] {{0, 5}}, false)));
        assertEquals(false, store.complete());
        boolean changed = store.apply(AnpdLossLedger.decode(snapshot(1, 2, 20, 20,
                new long[][] {{15, 5}}, true)));
        assertTrue(changed);
        assertEquals(1, store.periodEpoch());
        assertEquals(true, store.complete());
        assertEquals(1, store.intervals().length);
        assertEquals(15, store.intervals()[0].start);
        assertEquals(false, store.apply(AnpdLossLedger.decode(snapshot(0, 3, 30, 30,
                new long[][] {{0, 5}, {15, 5}, {25, 5}}, true))));
        assertEquals(2, store.appliedRevision());
        assertEquals(1, store.intervals().length);
    }
    @Test public void ingressPeriodEpochResetsReceiverDropsAndIgnoresReorderedOldLedger() throws Exception {
        byte[] key = new byte[32];
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(1, 0);
        UdpPcmQueue queue = new UdpPcmQueue(48_000, 2, "balanced");
        try (AnpdAudioIngress ingress = AnpdAudioIngress.record(7, key, format, queue, null)) {
            byte[] descriptor = packet(key, 7, 0, UdpPacketDecoder.FORMAT, 0, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT, ingress.accept(descriptor, descriptor.length));
            byte[] firstLedger = packet(key, 7, 1, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshot(0, 1, 110, 110, new long[0][0]));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER, ingress.accept(firstLedger, firstLedger.length));
            byte[] audio = new byte[40];
            byte[] first = packet(key, 7, 2, UdpPacketDecoder.AUDIO, 100, audio);
            byte[] duplicate = packet(key, 7, 3, UdpPacketDecoder.AUDIO, 100, audio);
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(first, first.length));
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(duplicate, duplicate.length));
            assertEquals(0, queue.receiverDropRanges().length);
            assertEquals(10, queue.take().frameCount);
            byte[] late = packet(key, 7, 4, UdpPacketDecoder.AUDIO, 105, new byte[40]);
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(late, late.length));
            assertEquals(1, queue.receiverDropRanges().length);
            assertEquals(110, queue.receiverDropRanges()[0].start);
            assertEquals(5, queue.receiverDropRanges()[0].frameCount);

            // The successful user PAUSE ACK invokes this reset before RESUME begins a new epoch.
            ingress.resetReceiverDropTrackingForPeriod();
            assertEquals(0, queue.receiverDropRanges().length);
            byte[] resumed = packet(key, 7, 5, UdpPacketDecoder.AUDIO, 200, audio);
            byte[] resumedDuplicate = packet(key, 7, 6, UdpPacketDecoder.AUDIO, 200, audio);
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(resumed, resumed.length));
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(resumedDuplicate, resumedDuplicate.length));
            assertEquals(0, queue.receiverDropRanges().length);
            assertEquals(10, queue.take().frameCount);
            byte[] lateAfterPause = packet(key, 7, 7, UdpPacketDecoder.AUDIO, 205, new byte[40]);
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(lateAfterPause, lateAfterPause.length));
            assertEquals(1, queue.receiverDropRanges().length);
            assertEquals(210, queue.receiverDropRanges()[0].start);
            assertEquals(5, queue.receiverDropRanges()[0].frameCount);

            byte[] nextPeriod = packet(key, 7, 8, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshot(1, 2, 220, 210, new long[][] {{210, 5}}));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER, ingress.accept(nextPeriod, nextPeriod.length));
            assertEquals(1, queue.receiverDropRanges().length);
            byte[] delayedOldPeriod = packet(key, 7, 9, UdpPacketDecoder.LOSS_LEDGER, 0,
                    snapshot(0, 3, 220, 220, new long[][] {{100, 10}}));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER, ingress.accept(delayedOldPeriod, delayedOldPeriod.length));
            assertEquals(1, queue.receiverDropRanges().length);
            assertEquals(1, ingress.lossLedgerPeriodEpoch());
        }
    }
    @Test public void productionIngressAcceptsLedgerWithoutCountingItAsAudio() throws Exception {
        byte[] key = new byte[32];
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(1, 0);
        UdpPcmQueue queue = new UdpPcmQueue(48_000, 2, "balanced");
        try (AnpdAudioIngress ingress = AnpdAudioIngress.record(7, key, format, queue, null)) {
            byte[] descriptor = packet(key, 7, 0, UdpPacketDecoder.FORMAT, 0, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT, ingress.accept(descriptor, descriptor.length));
            byte[] ledger = packet(key, 7, 1, UdpPacketDecoder.LOSS_LEDGER, 0, hex(PAYLOAD));
            assertEquals(AnpdAudioIngress.Result.LOSS_LEDGER, ingress.accept(ledger, ledger.length));
            assertEquals(7, ingress.appliedLossLedgerRevision());
            assertEquals(2, ingress.lossLedgerIntervals().length);
            assertEquals(2, ingress.lossLedgerIntervals()[0].start);
            assertEquals(0, ingress.authenticatedAudioFrames());
            assertEquals(0, ingress.droppedFrames());
        }
    }
    @Test public void storeRequiresCumulativeSnapshotsAndDoesNotRegressOnReorder() throws Exception {
        AnpdLossLedger.Store store = new AnpdLossLedger.Store();
        AnpdLossLedger first = AnpdLossLedger.decode(snapshot(7, 16, 12,
                new long[][] {{2, 3}, {10, 5}}));
        store.apply(first);
        assertEquals(7, store.appliedRevision());

        AnpdLossLedger newer = AnpdLossLedger.decode(snapshot(9, 24, 20,
                new long[][] {{2, 3}, {10, 5}, {18, 2}}));
        store.apply(newer);
        assertEquals(9, store.appliedRevision());
        store.apply(first);
        assertEquals(9, store.appliedRevision());

        AnpdLossLedger omitted = AnpdLossLedger.decode(snapshot(10, 30, 25,
                new long[][] {{10, 5}, {18, 2}}));
        try {
            store.apply(omitted);
            throw new AssertionError("newer snapshot omitted an already applied drop interval");
        } catch (UdpPacketDecoder.ProtocolException expected) {
            assertEquals(9, store.appliedRevision());
        }
    }

    @Test public void sourceRangesFollowRealAndPlcFramesThroughPlayedOutput() throws Exception {
        UdpPcmQueue queue = new UdpPcmQueue(
                new PairingProtocol.AudioParameters(48_000, 2, 0), "balanced", System::nanoTime);
        UdpPacketDecoder decoder = new UdpPacketDecoder(7, new byte[32]);
        long now = System.nanoTime();
        queue.acceptAt(new UdpPacketDecoder.Packet(UdpPacketDecoder.AUDIO, 7, 1, 100,
                new byte[20 * 4]), decoder, now);
        UdpPcmQueue.QueueInputStream input = (UdpPcmQueue.QueueInputStream) queue.asPcmInputStream();
        byte[] header = new byte[8];
        assertEquals(8, input.read(header, 0, header.length));
        Pcm16FrameStream pcm = new Pcm16FrameStream(2);
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        byte[] audio = new byte[20 * 4];
        input.beginSourceRangeBatch();
        assertEquals(audio.length, pcm.readFrames(input, audio, audio.length));
        assertEquals(1, input.sourceRangeCount());
        assertEquals(100, input.sourceRangeStart(0));
        assertEquals(20, input.sourceRangeFrames(0));
        assertEquals(false, input.sourceRangeSynthetic(0));
        map.recordWrite(input, 0, 20);
        map.advancePlayed(20);

        queue.acceptAt(new UdpPacketDecoder.Packet(UdpPacketDecoder.AUDIO, 7, 2, 145,
                new byte[10 * 4]), decoder, System.nanoTime());
        byte[] plc = new byte[25 * 4];
        input.beginSourceRangeBatch();
        assertEquals(plc.length, pcm.readFrames(input, plc, plc.length));
        assertEquals(1, input.sourceRangeCount());
        assertEquals(120, input.sourceRangeStart(0));
        assertEquals(25, input.sourceRangeFrames(0));
        assertEquals(true, input.sourceRangeSynthetic(0));

        map.recordWrite(input, 0, 25);
        map.advancePlayed(45);
        assertEquals(145, map.settledSourceThrough());
        assertEquals(45, map.settledSourceFrames());
        assertEquals(25, map.settledSyntheticFrames());
        byte[] nextReal = new byte[10 * 4];
        input.beginSourceRangeBatch();
        assertEquals(nextReal.length, pcm.readFrames(input, nextReal, nextReal.length));
        assertEquals(1, input.sourceRangeCount());
        assertEquals(145, input.sourceRangeStart(0));
        assertEquals(10, input.sourceRangeFrames(0));
        assertEquals(false, input.sourceRangeSynthetic(0));
        map.recordWrite(input, 0, 10);
        map.advancePlayed(55);
        assertEquals(25, map.settledSyntheticFrames());
        input.close();
    }
    @Test public void outputMapSettlesTenThousandAndKeepsClassifiableSourceGaps() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        for (int i = 0; i < 100; i++) {
            map.recordWrite(i * 100L, 100, false);
            map.advancePlayed((i + 1L) * 100);
        }
        assertEquals(10_000, map.settledSourceFrames());
        assertEquals(10_000, map.settledSourceThrough());
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 10_000, true, true, true);
        assertTrue(map.complete());

        map.recordWrite(10_020, 10, false);
        map.advancePlayed(10_010);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 10_030, true, true, true);
        assertEquals(1, map.sourceGapCount());
        assertEquals(10_000, map.sourceGapStart(0));
        assertEquals(10_020, map.sourceGapEnd(0));
        assertTrue(map.complete());
    }
    @Test public void onlyPcDropOverlapInsideSettledSourceGapsIsClassified() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(100, 5, false);
        map.advancePlayed(5);
        map.recordWrite(30_000, 10, false);
        map.advancePlayed(15);
        AnpdLossLedger.Interval[] pcDrops = {new AnpdLossLedger.Interval(105, 10)};
        map.accountSourceGaps(pcDrops, new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 112, true, true, true);
        assertEquals(7, map.settledPcDropFrames());
        map.accountSourceGaps(pcDrops, new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 120, true, true, true);
        assertEquals(10, map.settledPcDropFrames());
        assertTrue(map.complete());
    }

    @Test public void newEligibleSegmentExcludesPauseIndexHoleAndKeepsSettledHistory() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(100, 5, false);
        map.advancePlayed(5);
        map.resetOutputEpoch(true);
        map.beginSourceSegment();
        map.recordWrite(10_000, 5, false);
        map.advancePlayed(5);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 10_005, true, true, true);
        assertEquals(10, map.settledSourceFrames());
        assertEquals(10_005, map.settledSourceThrough());
        assertEquals(0, map.sourceGapCount());
        assertTrue(map.complete());
    }

    @Test public void sourceGapsAreMutuallyAttributedAndPlcCountsOnce() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(100, 10, false);
        map.advancePlayed(10);
        map.recordWrite(200, 10, false);
        map.advancePlayed(20);
        AnpdLossLedger.Interval[] pcDrops = {new AnpdLossLedger.Interval(120, 10),
                new AnpdLossLedger.Interval(210, 5)};
        com.anpod.server.SourceFrameRanges.Range[] receiverDrops = {
                new com.anpod.server.SourceFrameRanges.Range(140, 20)};
        map.accountSourceGaps(pcDrops, receiverDrops, new com.anpod.server.SourceFrameRanges.Range[0],
                180, true, true, true);
        assertEquals(10, map.settledPcDropFrames());
        assertEquals(20, map.settledReceiverDropFrames());
        assertEquals(40, map.settledTransportMissingFrames());
        map.accountSourceGaps(pcDrops, receiverDrops, new com.anpod.server.SourceFrameRanges.Range[0],
                200, true, true, true);
        assertEquals(60, map.settledTransportMissingFrames());
        map.recordWrite(210, 5, true);
        map.advancePlayed(25);
        map.accountSourceGaps(pcDrops, receiverDrops, new com.anpod.server.SourceFrameRanges.Range[0],
                215, true, true, true);
        assertEquals(60, map.settledTransportMissingFrames());
        assertEquals(15, map.settledPcDropFrames());
        assertEquals(20, map.settledGoodSourceFrames());
        assertEquals(115, map.settledEligibleSourceFrames());
    }

    @Test public void overlappingReceiverAndOutputDropsKeepForwardProgress() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(100, 10, false);
        map.advancePlayed(10);
        map.recordWrite(200, 10, false);
        map.advancePlayed(20);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(120, 40)},
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(140, 30)},
                210, true, true, true);
        assertEquals(40, map.settledReceiverDropFrames());
        assertEquals(10, map.settledOutputDropFrames());
        assertEquals(40, map.settledTransportMissingFrames());
        assertEquals(110, map.settledEligibleSourceFrames());
        assertTrue(map.complete());
    }
    @Test public void lateOriginalPacketReclassifiesPreviouslySettledPlcGap() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(0, 10, false);
        map.advancePlayed(10);
        map.recordWrite(10, 20, true);
        map.advancePlayed(30);
        com.anpod.server.SourceFrameRanges.Range[] none = new com.anpod.server.SourceFrameRanges.Range[0];
        map.accountSourceGaps(new AnpdLossLedger.Interval[0], none, none, 30, true, true, true);
        assertEquals(20, map.settledTransportMissingFrames());
        assertEquals(20, map.settledLossFrames());
        com.anpod.server.SourceFrameRanges.Range[] lateReceiverDrop = {
                new com.anpod.server.SourceFrameRanges.Range(15, 5)};
        map.accountSourceGaps(new AnpdLossLedger.Interval[0], lateReceiverDrop, none, 30, true, true, true);
        assertEquals(15, map.settledTransportMissingFrames());
        assertEquals(5, map.settledReceiverDropFrames());
        assertEquals(30, map.settledEligibleSourceFrames());
        assertEquals(20, map.settledLossFrames());
        assertTrue(map.complete());
    }    @Test public void sourceSettlementStopsAtPendingOutputWatermark() {
        SourceFrameOutputMap pending = new SourceFrameOutputMap();
        pending.beginSourceSegment();
        pending.recordWrite(0, 100, false);
        pending.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 1_000, true, true, true);
        assertEquals(0, pending.settledEligibleSourceThrough());

        SourceFrameOutputMap prefix = new SourceFrameOutputMap();
        prefix.beginSourceSegment();
        AnpdLossLedger.Interval[] pcPrefix = {new AnpdLossLedger.Interval(0, 10)};
        prefix.accountSourceGaps(pcPrefix, new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 1_000, true, true, true);
        prefix.recordWrite(10, 90, false);
        prefix.accountSourceGaps(pcPrefix, new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 1_000, true, true, true);
        assertEquals(10, prefix.settledEligibleSourceThrough());
    }
    @Test public void startupOutputDropIsCountedOnce() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(10, 90, false);
        map.advancePlayed(90);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(0, 10)},
                100, true, true, true);
        assertEquals(10, map.settledOutputDropFrames());
        assertEquals(10, map.settledLossFrames());
        assertEquals(100, map.settledEligibleSourceFrames());
        assertTrue(map.complete());
    }
    @Test public void startupPrefixPcDropCountsFromAuthenticatedSourceRange() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(100, 10, false);
        map.advancePlayed(10);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {
                new AnpdLossLedger.Interval(80, 20)},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 110, true, true, true);
        assertEquals(20, map.settledPcDropFrames());
        assertEquals(10, map.settledGoodSourceFrames());
        assertEquals(30, map.settledEligibleSourceFrames());
    }

    @Test public void firstSourceEvidenceCanBeAnEntirePcDroppedCaptureBatch() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {
                new AnpdLossLedger.Interval(400, 100)},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 500, true, true, true);
        assertEquals(100, map.settledPcDropFrames());
        assertEquals(100, map.settledEligibleSourceFrames());
        assertTrue(map.complete());
    }

    @Test public void firstSourceEvidenceCanBeAReceiverDroppedPacket() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(600, 100)},
                new com.anpod.server.SourceFrameRanges.Range[0], 700, true, true, true);
        assertEquals(100, map.settledReceiverDropFrames());
        assertEquals(100, map.settledEligibleSourceFrames());
        assertTrue(map.complete());
    }

    @Test public void duplicateReceiverDropDoesNotChargeAPlayedSourceRange() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(800, 10, false);
        map.advancePlayed(10);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(800, 10)},
                new com.anpod.server.SourceFrameRanges.Range[0], 810, true, true, true);
        assertEquals(10, map.settledGoodSourceFrames());
        assertEquals(0, map.settledReceiverDropFrames());
        assertEquals(10, map.settledEligibleSourceFrames());
        assertTrue(map.complete());
    }

    @Test public void firstTransportGapUsesKnownCaptureOrigin() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment(0);
        map.recordWrite(100, 900, false);
        map.advancePlayed(900);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 1_000, true, true, true);
        assertEquals(100, map.settledTransportMissingFrames());
        assertEquals(1_000, map.settledEligibleSourceFrames());
        assertEquals(10f, map.lossPercent(), 0.0001f);
    }

    @Test public void duplicateDropEvidenceRemainsPendingUntilOriginalOutputSettles() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment(0);
        map.recordWrite(0, 100, false);
        com.anpod.server.SourceFrameRanges.Range[] duplicate = {
                new com.anpod.server.SourceFrameRanges.Range(0, 100)};
        map.accountSourceGaps(new AnpdLossLedger.Interval[0], duplicate,
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertEquals(0, map.settledReceiverDropFrames());
        assertEquals(0, map.settledEligibleSourceThrough());
        map.advancePlayed(100);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0], duplicate,
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertEquals(0, map.settledReceiverDropFrames());
        assertEquals(100, map.settledGoodSourceFrames());
        assertEquals(100, map.settledEligibleSourceFrames());
        assertEquals(0f, map.lossPercent(), 0.0001f);
    }

    @Test public void driftSourceFramesStayUnknownInsteadOfBecomingTransportLossAndAgeOut() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment(0);
        map.recordWrite(0, 100, false);
        map.recordUnknownWrite(100, 100, 100);
        map.recordWrite(200, 100, false);
        map.advancePlayed(300);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 300, true, true, true);

        assertEquals(0, map.settledTransportMissingFrames());
        assertEquals(200, map.settledGoodSourceFrames());
        assertEquals(300, map.settledEligibleSourceThrough());
        assertTrue(map.rollingComplete());
        assertEquals(false, map.complete());

        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordSourceLossSnapshot(map.sourceSegmentId(), map.sourceSegmentLogicalStart(),
                map.sourceSegmentOrigin(), map.settledEligibleSourceThrough(), 100,
                map.settledLossRanges(), map.rollingComplete(), map.settledUnknownRanges());
        assertNull(sampler.recentSourceLossWindow(10_000_000_000L, 100));
        assertNull(sampler.recentSourceLossWindow(60_000_000_000L, 100));
        assertNull(map.lossPercent());

        map.recordWrite(300, 6_000, false);
        map.advancePlayed(6_300);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 6_300, true, true, true);
        sampler.recordSourceLossSnapshot(map.sourceSegmentId(), map.sourceSegmentLogicalStart(),
                map.sourceSegmentOrigin(), map.settledEligibleSourceThrough(), 100,
                map.settledLossRanges(), map.rollingComplete(), map.settledUnknownRanges());
        assertEquals(6_000, sampler.recentSourceLossWindow(60_000_000_000L, 100).eligibleFrames);
        assertEquals(0, map.settledTransportMissingFrames());
    }

    @Test public void rollingValidityCanRecoverAfterUnknownWithoutRepairingCumulativeHistory() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(900, 10, false);
        map.advancePlayed(10);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 910, true, true, true);
        assertTrue(map.rollingComplete());
        map.markIncomplete();
        assertEquals(false, map.rollingComplete());
        assertEquals(false, map.complete());
        assertNull(map.lossPercent());
        map.beginSourceSegment();
        map.recordWrite(0, 10, false);
        map.advancePlayed(10);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 10, true, true, true);
        assertTrue(map.rollingComplete());
        assertEquals(false, map.complete());
        assertNull(map.lossPercent());
    }

    @Test public void resetIndexStartsNewBoundedEpochButPreservesSettledHistory() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(1_000, 100, false);
        map.advancePlayed(100);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 1_100, true, true, true);
        assertEquals(100, map.settledGoodSourceFrames());

        map.beginSourceSegment();
        map.recordWrite(10, 90, false);
        map.advancePlayed(90);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {
                        new AnpdLossLedger.Interval(0, 10)},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertEquals(10, map.settledPcDropFrames());
        assertEquals(190, map.settledGoodSourceFrames());
        assertEquals(200, map.settledEligibleSourceFrames());
        assertEquals(100, map.settledSourceThrough());
        assertTrue(map.complete());
    }

    @Test public void repeatedCumulativePcLedgerIsClippedAtResumeFloor() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        AnpdLossLedger.Interval oldDrop = new AnpdLossLedger.Interval(0, 10);
        map.recordWrite(10, 90, false);
        map.advancePlayed(90);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {oldDrop},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertEquals(10, map.settledPcDropFrames());
        assertEquals(100, map.settledEligibleSourceFrames());

        map.beginSourceSegment(100);
        map.recordWrite(110, 90, false);
        map.advancePlayed(90);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {
                        oldDrop, new AnpdLossLedger.Interval(100, 10)},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 200, true, true, true);
        assertEquals(20, map.settledPcDropFrames());
        assertEquals(180, map.settledGoodSourceFrames());
        assertEquals(200, map.settledEligibleSourceFrames());
    }
    @Test public void acknowledgedUserPauseStartsFreshPeriodAndClipsOldLedger() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        AnpdLossLedger.Interval oldDrop = new AnpdLossLedger.Interval(0, 10);
        map.recordWrite(10, 90, false);
        map.advancePlayed(90);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {oldDrop},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertEquals(10, map.settledPcDropFrames());

        map.resetLossPeriod();
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {oldDrop},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertEquals(0, map.settledEligibleSourceFrames());
        map.beginSourceSegment(100);
        map.recordWrite(110, 90, false);
        map.advancePlayed(90);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {
                        oldDrop, new AnpdLossLedger.Interval(100, 10)},
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 200, true, true, true);
        assertEquals(10, map.settledPcDropFrames());
        assertEquals(90, map.settledGoodSourceFrames());
        assertEquals(100, map.settledEligibleSourceFrames());
    }
    @Test public void unsettledSourceTailRemainsUnknownAcrossResumeSegment() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(0, 100, false);
        map.advancePlayed(100);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        assertTrue(map.complete());

        map.beginSourceSegment(120, 100);
        assertEquals(100, map.sourceSegmentUnknownStart());
        assertEquals(120, map.sourceSegmentUnknownEnd());
        assertEquals(120, map.sourceSegmentLogicalStart());
        map.recordWrite(120, 80, false);
        map.advancePlayed(80);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 200, true, true, true);
        assertEquals(180, map.settledEligibleSourceFrames());
        assertEquals(false, map.complete());
    }
    @Test public void unmappedRecoveryTailIsUnknownUntilItAgesOutOfRollingWindows() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(0, 100, false);
        map.advancePlayed(100);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 100, true, true, true);
        map.beginSourceSegment(-1, -1, 1);
        assertEquals(40, map.sourceSegmentUnknownStart());
        assertEquals(100, map.sourceSegmentUnknownEnd());
        map.recordWrite(0, 40, false);
        map.advancePlayed(40);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 40, true, true, true);
        assertTrue(map.rollingComplete());

        AodMetricsSampler sampler = new AodMetricsSampler();
        sampler.recordSourceLossSnapshot(map.sourceSegmentId(), map.sourceSegmentLogicalStart(),
                map.sourceSegmentOrigin(), map.settledEligibleSourceThrough(), 1,
                map.settledLossRanges(), map.rollingComplete());
        sampler.recordUnknownSourceRange(map.sourceSegmentUnknownStart(),
                map.sourceSegmentUnknownEnd(), 1);
        assertEquals(10, sampler.recentSourceLossWindow(10_000_000_000L, 1).eligibleFrames);
        assertNull(sampler.recentSourceLossWindow(60_000_000_000L, 1));

        map.recordWrite(40, 80, false);
        map.advancePlayed(120);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 120, true, true, true);
        sampler.recordSourceLossSnapshot(map.sourceSegmentId(), map.sourceSegmentLogicalStart(),
                map.sourceSegmentOrigin(), map.settledEligibleSourceThrough(), 1,
                map.settledLossRanges(), map.rollingComplete());
        sampler.recordUnknownSourceRange(map.sourceSegmentUnknownStart(),
                map.sourceSegmentUnknownEnd(), 1);
        assertEquals(10, sampler.recentSourceLossWindow(10_000_000_000L, 1).eligibleFrames);
        assertEquals(60, sampler.recentSourceLossWindow(60_000_000_000L, 1).eligibleFrames);
    }
    @Test public void startupTrimAndPlayedSourceShareOneEligibleDenominator() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(1_140, 20, false);
        map.advancePlayed(20);
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(100, 1_040)},
                1_160, true, true, true);
        assertEquals(1_040, map.settledOutputDropFrames());
        assertEquals(20, map.settledGoodSourceFrames());
        assertEquals(1_060, map.settledEligibleSourceFrames());
        assertTrue(map.complete());
    }

    @Test public void unplayedOutputFramesWaitForPcSettlementBeforeCounting() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(500, 100, false);
        map.resetOutputEpoch(true);
        assertEquals(0, map.settledOutputDropFrames());
        assertEquals(false, map.complete());
        map.accountSourceGaps(new AnpdLossLedger.Interval[0],
                new com.anpod.server.SourceFrameRanges.Range[0],
                new com.anpod.server.SourceFrameRanges.Range[0], 600, true, true, true);
        assertEquals(100, map.settledOutputDropFrames());
        assertTrue(map.complete());
    }

    @Test public void requiredLossVectorCountsEachSourceFrameOnceAcrossPlc() {
        SourceFrameOutputMap map = new SourceFrameOutputMap();
        map.beginSourceSegment();
        map.recordWrite(0, 10_000, false);
        map.advancePlayed(10_000);
        map.recordWrite(10_000, 200, true);
        map.advancePlayed(10_200);
        map.recordWrite(10_400, 1_000, false);
        map.advancePlayed(11_200);
        map.accountSourceGaps(new AnpdLossLedger.Interval[] {
                        new AnpdLossLedger.Interval(10_000, 100)},
                new com.anpod.server.SourceFrameRanges.Range[] {
                        new com.anpod.server.SourceFrameRanges.Range(10_200, 200)},
                new com.anpod.server.SourceFrameRanges.Range[0], 11_400, true, true, true);
        assertEquals(100, map.settledPcDropFrames());
        assertEquals(100, map.settledTransportMissingFrames());
        assertEquals(200, map.settledReceiverDropFrames());
        assertEquals(11_000, map.settledGoodSourceFrames());
        assertEquals(200, map.settledSyntheticFrames());
        assertEquals(11_400, map.settledEligibleSourceFrames());
        assertEquals(400, map.settledPcDropFrames() + map.settledTransportMissingFrames()
                + map.settledReceiverDropFrames() + map.settledOutputDropFrames());
        assertEquals((float) (100.0 * 400 / 11_400), map.lossPercent(), 0.0001f);
        assertTrue(map.complete());
    }
    @Test public void receiverDropRangesCoalesceDuplicatesAndBoundOverflow() {
        com.anpod.server.SourceFrameRanges ranges = new com.anpod.server.SourceFrameRanges();
        assertTrue(ranges.add(20, 10));
        assertTrue(ranges.add(10, 10));
        assertTrue(ranges.add(25, 10));
        assertTrue(ranges.add(10, 25));
        assertEquals(1, ranges.size());
        assertEquals(10, ranges.start(0));
        assertEquals(35, ranges.end(0));
        assertEquals(25, ranges.coveredFrames());
        for (int i = 0; i < com.anpod.server.SourceFrameRanges.CAPACITY - 1; i++)
            ranges.add(100 + 2L * i, 1);
        assertTrue(ranges.complete());
        ranges.add(1_000, 1);
        assertEquals(false, ranges.complete());
    }

    @Test public void receiverQueuesRetainIndexedDropsButExcludePause() throws Exception {
        UdpPcmQueue pcm = new UdpPcmQueue(48_000, 2, "balanced");
        UdpPacketDecoder decoder = new UdpPacketDecoder(7, new byte[32]);
        pcm.acceptAt(new UdpPacketDecoder.Packet(UdpPacketDecoder.AUDIO, 7, 1, 100,
                new byte[40]), decoder, 1);
        pcm.acceptAt(new UdpPacketDecoder.Packet(UdpPacketDecoder.AUDIO, 7, 2, 100,
                new byte[40]), decoder, 2);
        com.anpod.server.SourceFrameRanges.Range[] pcmDrops = pcm.receiverDropRanges();
        assertEquals(0, pcmDrops.length);
        assertTrue(pcm.receiverDropTrackingComplete());
        pcm.resetReceiverDropTrackingForPeriod();
        assertEquals(0, pcm.receiverDropRanges().length);
        assertTrue(pcm.receiverDropTrackingComplete());

        com.anpod.server.FloatPcmFrameQueue floats =
                new com.anpod.server.FloatPcmFrameQueue(48_000, false);
        float[] samples = new float[20];
        assertTrue(floats.offer(200, samples, 10, 1));
        assertEquals(false, floats.offer(200, samples, 10, 2));
        com.anpod.server.SourceFrameRanges.Range[] floatDrops = floats.receiverDropRanges();
        assertEquals(0, floatDrops.length);
        floats.setPaused(true);
        assertEquals(false, floats.offer(300, samples, 10, 3));
        assertEquals(0, floats.receiverDropRanges().length);
        floats.resetReceiverDropTrackingForPeriod();
        assertEquals(0, floats.receiverDropRanges().length);
        assertEquals(0, floats.startupOutputDropRanges().length);
        assertTrue(floats.receiverDropTrackingComplete());
        assertTrue(floats.startupOutputDropTrackingComplete());
    }

    @Test public void startupQueueTrimIsClassifiedAsOutputDrop() throws Exception {
        com.anpod.server.FloatPcmFrameQueue queue =
                new com.anpod.server.FloatPcmFrameQueue(48_000, true, 3_000);
        float[] input = new float[4_000];
        assertTrue(queue.offer(100, input, 2_000, 1));
        assertEquals(20, queue.readFrames(new float[40], 0, 20));
        com.anpod.server.SourceFrameRanges.Range[] outputDrops = queue.startupOutputDropRanges();
        assertEquals(1, outputDrops.length);
        assertEquals(100, outputDrops[0].start);
        assertEquals(1_040, outputDrops[0].frameCount);
        assertEquals(0, queue.receiverDropRanges().length);
    }

    private static byte[] packet(byte[] key, long sessionId, long sequence, int type,
            long frameIndex, byte[] payload) throws Exception {
        byte[] header = ByteBuffer.allocate(UdpPacketDecoder.HEADER_BYTES).order(ByteOrder.BIG_ENDIAN)
                .put(new byte[] {'A', 'N', 'P', 'D'}).put((byte) 2).put((byte) type).putShort((short) 0)
                .putLong(sessionId).putLong(sequence).putLong(frameIndex).putShort((short) payload.length).array();
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN)
                .putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] sealed = cipher.doFinal(payload);
        byte[] datagram = new byte[header.length + sealed.length];
        System.arraycopy(header, 0, datagram, 0, header.length);
        System.arraycopy(sealed, 0, datagram, header.length, sealed.length);
        return datagram;
    }
    private static byte[] snapshot(long revision, long captureTail, long settledThrough, long[][] intervals) {
        return snapshot(0, revision, captureTail, settledThrough, intervals, true);
    }
    private static byte[] snapshot(long epoch, long revision, long captureTail, long settledThrough,
            long[][] intervals) {
        return snapshot(epoch, revision, captureTail, settledThrough, intervals, true);
    }
    private static byte[] snapshotV3(long revision, long captureTail, long settledThrough,
            long generation, long segmentStart, long[][] intervals) {
        ByteBuffer bytes = ByteBuffer.allocate(52 + intervals.length * 16).order(ByteOrder.BIG_ENDIAN);
        bytes.put((byte) 3).put((byte) 1).put((byte) intervals.length).put((byte) 0);
        bytes.putLong(0).putLong(revision).putLong(captureTail).putLong(settledThrough);
        bytes.putLong(generation).putLong(segmentStart);
        for (long[] interval : intervals) bytes.putLong(interval[0]).putLong(interval[1]);
        return bytes.array();
    }
    private static byte[] snapshot(long epoch, long revision, long captureTail, long settledThrough,
            long[][] intervals, boolean complete) {
        ByteBuffer bytes = ByteBuffer.allocate(36 + intervals.length * 16).order(ByteOrder.BIG_ENDIAN);
        bytes.put((byte) 2).put((byte) (complete ? 1 : 0)).put((byte) intervals.length).put((byte) 0);
        bytes.putLong(epoch).putLong(revision).putLong(captureTail).putLong(settledThrough);
        for (long[] interval : intervals) bytes.putLong(interval[0]).putLong(interval[1]);
        return bytes.array();
    }    private static byte[] hex(String value) {
        byte[] bytes = new byte[value.length() / 2];
        for (int i = 0; i < bytes.length; i++)
            bytes[i] = (byte) Integer.parseInt(value.substring(i * 2, i * 2 + 2), 16);
        return bytes;
    }
}
