package app.anpod.receiver;

import com.anpod.server.FloatPcmFrameQueue;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import java.net.InetSocketAddress;
import java.net.Socket;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Assume;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.annotation.Config;

/** Optional Rust↔Java loopback test that exercises the production ADB record coordinator. */
@RunWith(RobolectricTestRunner.class)
@Config(sdk = 34)
public final class AdbRecordStreamInteropTest {
    private static final int[] FLOAT_BITS = {
            0x3f800000, 0xbf000000, 0x00000001, 0x80000000,
            0x7f7fffff, 0x80000001, 0x3eaaaaab, 0x00800000
    };
    private static final int FRAMES_PER_PACKET = 143;
    private static final int PACKET_COUNT = 14;

    @Test public void productionCoordinatorAuthenticatesBothProfilesAndAdbTransportTags() throws Exception {
        String portValue = System.getenv("ANPOD_ADB_LOOPBACK_PORT");
        Assume.assumeTrue("Rust loopback fixture did not provide ANPOD_ADB_LOOPBACK_PORT",
                portValue != null && !portValue.isEmpty());
        int port = Integer.parseInt(portValue);
        ReceiverService.currentAudio = null;

        int[][] profilesAndTags = {{4, 1}, {5, 2}, {4, 2}, {5, 1}};
        for (int cycle = 0; cycle < profilesAndTags.length; cycle++) {
            AoaReceiverCoordinator.TransportTag transport = profilesAndTags[cycle][1] == 1
                    ? AoaReceiverCoordinator.TransportTag.USB_ADB
                    : AoaReceiverCoordinator.TransportTag.WIFI_ADB;
            runCycle(port, profilesAndTags[cycle][0], transport, cycle);
        }
    }

    private static void runCycle(int port, int profile, AoaReceiverCoordinator.TransportTag transport,
            int cycle) throws Exception {
        byte[] hostId = filled(16, 1);
        byte[] receiverId = filled(16, 2);
        byte[] secret = filled(32, 3);
        PairingProtocol.BindingMaterial saved = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        Arrays.fill(hostId, (byte) 0);
        Arrays.fill(receiverId, (byte) 0);
        Arrays.fill(secret, (byte) 0);
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(profile, 1);
        TestEndpoint endpoint = new TestEndpoint();
        Socket socket = new Socket();
        AoaAccessoryIo io = null;
        AoaReceiverCoordinator coordinator = null;
        try {
            socket.connect(new InetSocketAddress("127.0.0.1", port), 5_000);
            socket.setSoTimeout(5_000);
            io = AoaAccessoryIo.open(socket);
            coordinator = AoaReceiverCoordinator.startAdbStream(io, saved, endpoint,
                    "rust-loopback-" + cycle, cycle + 1L, transport);
            io = null;
            assertTrue(coordinator.isAdbStream());
            assertEquals(transport, coordinator.transportTag());
            assertTrue("cycle " + cycle + ": Float32 frames were not verified",
                    endpoint.samplesVerified.await(30, TimeUnit.SECONDS));
            if (endpoint.verificationFailure.get() != null)
                throw new AssertionError("cycle " + cycle + ": Float32 sample mismatch",
                        endpoint.verificationFailure.get());
            assertTrue("P5-A/P5-C/ANPD stream must terminate cleanly", coordinator.awaitTermination(60, TimeUnit.SECONDS));
            assertEquals("cycle " + cycle + ": " + coordinator.failureMessage(), "", coordinator.failureMessage());
            assertNotNull(endpoint.context.get());
            assertEquals(PACKET_COUNT * FRAMES_PER_PACKET, endpoint.context.get().ingress.authenticatedAudioFrames());
        } finally {
            if (coordinator != null) {
                coordinator.cancel("interop cleanup");
                coordinator.awaitTermination(5, TimeUnit.SECONDS);
            }
            if (io != null) io.cancel();
            if (coordinator == null) {
                try { socket.close(); } catch (Exception ignored) { }
                saved.close();
            }
            AoaSessionContext context = endpoint.context.get();
            if (context != null) context.destroy();
        }
    }

    private static byte[] filled(int length, int value) {
        byte[] bytes = new byte[length];
        Arrays.fill(bytes, (byte) value);
        return bytes;
    }

    private static final class TestEndpoint implements AoaReceiverCoordinator.PlaybackEndpoint {
        final AtomicReference<AoaSessionContext> context = new AtomicReference<>();
        final CountDownLatch samplesVerified = new CountDownLatch(1);
        final AtomicReference<Throwable> verificationFailure = new AtomicReference<>();

        @Override public boolean isCurrent(String ownerId, long generation) { return true; }
        @Override public boolean supportsAudio(UdpPacketDecoder.AudioFormatV2 format) {
            return format != null && (format.profile == 4 || format.profile == 5) && format.sampleFormat == 3;
        }
        @Override public void start(AoaSessionContext next) {
            context.set(next);
            Thread verifier = new Thread(() -> {
                try {
                    int fadeFrames = Math.max(1,
                            next.formatV2.sampleRate * FloatPcmFrameQueue.FADE_MS / 1000);
                    int framesToRead = fadeFrames + 10;
                    float[] decoded = new float[framesToRead * 2];
                    int framesRead = 0;
                    while (framesRead < framesToRead) {
                        framesRead += next.floatQueue.readFrames(
                                decoded, framesRead, framesToRead - framesRead);
                    }
                    long droppedFrames = next.floatQueue.droppedFrames();
                    int firstVerifiedFrame = (int) ((droppedFrames + fadeFrames) % FRAMES_PER_PACKET);
                    for (int frame = 0; frame < 10; frame++) {
                        int packetSample = (firstVerifiedFrame + frame) * 2;
                        for (int channel = 0; channel < 2; channel++) {
                            int sample = fadeFrames * 2 + frame * 2 + channel;
                            assertEquals(FLOAT_BITS[(packetSample + channel) % FLOAT_BITS.length],
                                    Float.floatToRawIntBits(decoded[sample]));
                        }
                    }
                } catch (Throwable failure) {
                    verificationFailure.set(failure);
                } finally {
                    samplesVerified.countDown();
                }
            }, "AnPod interop sample verifier");
            verifier.setDaemon(true);
            verifier.start();
        }
        @Override public void pause() {
            AoaSessionContext current = context.get();
            if (current != null) current.floatQueue.setPaused(true);
        }
        @Override public void resume() {
            AoaSessionContext current = context.get();
            if (current != null) current.floatQueue.setPaused(false);
        }
        @Override public void stop() {
            AoaSessionContext current = context.get();
            if (current != null) current.stopQueue();
        }
        @Override public boolean streaming() {
            AoaSessionContext current = context.get();
            return current != null && current.ingress.hasAuthenticatedAudio();
        }
        @Override public byte[] playbackProgressPayloadV2() {
            return P5cControlProtocol.playbackProgressPayloadV2(ReceiverService.playbackProgressPayload(), 0, 0, 0);
        }
        @Override public boolean supportsPlaybackProgressExtension() { return true; }
        @Override public byte[] playbackProgressPayloadExtendedV2() {
            return ReceiverService.playbackProgressPayloadExtended(true, 2);
        }
        @Override public byte[] playbackProgressPayloadExtendedV2(int schema) {
            return ReceiverService.playbackProgressPayloadExtended(true, schema);
        }
    }
}
