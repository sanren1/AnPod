package app.anpod.receiver;

import static org.junit.Assert.*;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.AsynchronousCloseException;
import java.nio.channels.ReadableByteChannel;
import java.nio.channels.WritableByteChannel;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;
import org.junit.Test;

public class AoaReceiverCoordinatorTest {
    @Test public void unexpectedRecoveryRequiresStartedSessionAndNoTerminalStop() {
        assertTrue(AoaReceiverCoordinator.shouldRecoverUnexpectedFailure(true, false, false, false, true));
        assertFalse(AoaReceiverCoordinator.shouldRecoverUnexpectedFailure(false, false, false, false, true));
        assertFalse(AoaReceiverCoordinator.shouldRecoverUnexpectedFailure(true, true, false, false, true));
        assertFalse(AoaReceiverCoordinator.shouldRecoverUnexpectedFailure(true, false, true, false, true));
        assertFalse(AoaReceiverCoordinator.shouldRecoverUnexpectedFailure(true, false, false, true, false));
        assertFalse(AoaReceiverCoordinator.shouldRecoverUnexpectedFailure(true, false, false, false, false));
    }

    @Test public void scheduledRecoveryStopsForManualStopStaleOwnerOrExpiredWindow() {
        assertTrue(ReceiverService.shouldAttemptAoaRecovery(true, false, 100, 200));
        assertFalse(ReceiverService.shouldAttemptAoaRecovery(false, false, 100, 200));
        assertFalse(ReceiverService.shouldAttemptAoaRecovery(true, true, 100, 200));
        assertFalse(ReceiverService.shouldAttemptAoaRecovery(true, false, 200, 200));
    }

    @Test public void authenticatedProfilesRegisterBeforeStartAckAndStopAfterAck() throws Exception {
        for (int profile = 1; profile <= 5; profile++) runProfile(profile);
    }

    @Test public void authenticatedAdbStreamUsesItsOwnCapabilityAndRealTransportTag() throws Exception {
        runProfile(4, false, AoaReceiverCoordinator.TransportTag.USB_ADB);
        runProfile(5, false, AoaReceiverCoordinator.TransportTag.WIFI_ADB);
    }

    @Test public void phoneStopIsReportedOnNextNonHeartbeatAck() throws Exception {
        runProfile(1, true);
    }

    @Test public void adbPhoneStopIsReportedWithoutAoaRestartSemantics() throws Exception {
        runProfile(4, true, AoaReceiverCoordinator.TransportTag.USB_ADB);
        runProfile(5, true, AoaReceiverCoordinator.TransportTag.WIFI_ADB);
    }

    @Test public void adbHostCleanupStopDoesNotLookLikePhoneManualStop() throws Exception {
        runProfile(4, false, AoaReceiverCoordinator.TransportTag.USB_ADB, true);
        runProfile(5, false, AoaReceiverCoordinator.TransportTag.WIFI_ADB, true);
    }

    @Test public void adbBindingDerivationMatchesFrozenVectorAndRejectsAoaDomain() throws Exception {
        byte[] hostId = filled(16, 1), receiverId = filled(16, 2), secret = filled(32, 3);
        try (PairingProtocol.BindingMaterial saved = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
                PairingProtocol.BindingMaterial derived = saved.deriveAdbStream()) {
            byte[] record = derived.encodedRecord();
            try {
                assertArrayEquals(decodeHex("cb76cb157e4c5446d03232534f61486a750cb61ff29ccf8882b09c3ffe1d0c37"),
                        Arrays.copyOfRange(record, 37, 69));
            } finally { Arrays.fill(record, (byte) 0); }
            assertTrue(saved.hostIdMatchesHex("01010101010101010101010101010101"));
            assertFalse(saved.hostIdMatchesHex("0101010101010101010101010101010a"));
            assertFalse(saved.hostIdMatchesHex("0101010101010101010101010101010A"));
        }

        try (PairingProtocol.BindingMaterial saved = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
                PairingProtocol.BindingMaterial hostBinding = saved.deriveAdbStream()) {
            UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(4, 0);
            PairingProtocol.V2.HostStart hostStart = PairingProtocol.V2.Host.start(hostBinding, format);
            try (hostStart.host; PairingProtocol.BindingMaterial wrongDomain = saved.deriveAoa();
                    PairingProtocol.V2.ReceiverAwaitFinish receiver =
                            PairingProtocol.V2.Receiver.acceptHello(wrongDomain, hostStart.hello)) {
                assertThrows(PairingProtocol.Failure.class,
                        () -> hostStart.host.acceptChallenge(receiver.challenge));
            }
        }
    }

    @Test public void noHelloTimesOutAndReleasesTheSingleConnectionOwner() throws Exception {
        PairingProtocol.BindingMaterial saved = new PairingProtocol.BindingMaterial(
                sequence(0x10, 16), sequence(0x20, 16), sequence(0, 32));
        Duplex io = new Duplex();
        Endpoint endpoint = new Endpoint();
        AoaReceiverCoordinator coordinator = AoaReceiverCoordinator.start(
                io.owner(), saved, endpoint, "timeout-owner", 1, 40);
        assertFalse(AoaReceiverCoordinator.isAvailable());
        assertTrue(coordinator.awaitTermination(2, TimeUnit.SECONDS));
        assertEquals("AOA startup timeout", coordinator.failureMessage());
        assertNull(endpoint.context);
        assertTrue(io.ownerClosed);
        assertTrue(AoaReceiverCoordinator.isAvailable());
        saved.close();
    }

    @Test public void localCancelDoesNotBecomeAProtocolFailure() throws Exception {
        PairingProtocol.BindingMaterial saved = new PairingProtocol.BindingMaterial(
                sequence(0x10, 16), sequence(0x20, 16), sequence(0, 32));
        Duplex io = new Duplex();
        AoaReceiverCoordinator coordinator = AoaReceiverCoordinator.start(
                io.owner(), saved, new Endpoint(), "cancel-owner", 2, 5000);
        coordinator.cancel("user-stop");
        assertTrue(coordinator.awaitTermination(2, TimeUnit.SECONDS));
        assertEquals("", coordinator.failureMessage());
        assertTrue(io.ownerClosed);
        assertTrue(AoaReceiverCoordinator.isAvailable());
        saved.close();
    }

    private static void runProfile(int profile) throws Exception {
        runProfile(profile, false);
    }

    private static void runProfile(int profile, boolean stopFromPhone) throws Exception {
        runProfile(profile, stopFromPhone, AoaReceiverCoordinator.TransportTag.USB_DIRECT);
    }

    private static void runProfile(int profile, boolean stopFromPhone,
            AoaReceiverCoordinator.TransportTag transport) throws Exception {
        runProfile(profile, stopFromPhone, transport, false);
    }

    private static void runProfile(int profile, boolean stopFromPhone,
            AoaReceiverCoordinator.TransportTag transport, boolean stopFromHost) throws Exception {
        byte[] hostId = sequence(0x10, 16), receiverId = sequence(0x20, 16), secret = sequence(0, 32);
        PairingProtocol.BindingMaterial saved = new PairingProtocol.BindingMaterial(hostId, receiverId, secret);
        PairingProtocol.BindingMaterial hostBinding = transport.isAdbStream()
                ? saved.deriveAdbStream() : saved.deriveAoa();
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(profile, 0);
        PairingProtocol.V2.HostStart hostStart = PairingProtocol.V2.Host.start(hostBinding, format);
        Duplex io = new Duplex();
        Endpoint endpoint = new Endpoint();
        AoaReceiverCoordinator coordinator = transport.isAdbStream()
                ? AoaReceiverCoordinator.startAdbStream(io.owner(), saved, endpoint,
                        "test-owner", profile, transport)
                : AoaReceiverCoordinator.start(io.owner(), saved, endpoint, "test-owner", profile);
        PairingProtocol.SessionKeys keys = null;
        try {
            io.send(AnpuRecordCodec.HANDSHAKE, hostStart.hello);
            PairingProtocol.V2.HostAwaitReady awaiting;
            try (hostStart.host) {
                AnpuRecordCodec.Record challenge = io.receive();
                assertEquals(AnpuRecordCodec.HANDSHAKE, challenge.type);
                awaiting = hostStart.host.acceptChallenge(challenge.body);
            }
            io.send(AnpuRecordCodec.HANDSHAKE, awaiting.finish);
            PairingProtocol.V2.AuthenticatedSessionV2 hostSession;
            try (awaiting) {
                AnpuRecordCodec.Record ready = io.receive();
                hostSession = awaiting.acceptReady(ready.body);
            }
            try (hostSession) {
                keys = hostSession.takeKeys();
                int requiredCapabilities = P5cControlProtocol.CAP_AUDIO_V2
                        | (format.sampleFormat == 3 ? P5cControlProtocol.CAP_AUDIO_FLOAT32 : 0)
                        | (transport.isAdbStream() ? P5cControlProtocol.CAP_AUDIO_ADB_STREAM : 0)
                        | P5cControlProtocol.CAP_PROGRESS_EXT;
                sendControl(io, keys, hostSession.sessionId, 0, P5cControlProtocol.CAPABILITIES_QUERY,
                        P5cControlProtocol.capabilitiesQueryPayload(3, 1, requiredCapabilities));
                P5cControlProtocol.Frame caps = receiveControl(io, keys, hostSession.sessionId,
                        P5cControlProtocol.CAPABILITIES_INFO);
                assertEquals(0, caps.payload[11]);
                assertEquals(P5cControlProtocol.CAP_AUDIO_V2,
                        P5cControlProtocol.getUnsignedInt(caps.payload, 12) & P5cControlProtocol.CAP_AUDIO_V2);
                assertEquals(format.sampleFormat == 3 ? P5cControlProtocol.CAP_AUDIO_FLOAT32 : 0,
                        P5cControlProtocol.getUnsignedInt(caps.payload, 12) & P5cControlProtocol.CAP_AUDIO_FLOAT32);
                assertEquals(P5cControlProtocol.CAP_PROGRESS_EXT,
                        P5cControlProtocol.getUnsignedInt(caps.payload, 12) & P5cControlProtocol.CAP_PROGRESS_EXT);
                assertEquals(P5cControlProtocol.CAP_BUFFER_METRICS,
                        P5cControlProtocol.getUnsignedInt(caps.payload, 12) & P5cControlProtocol.CAP_BUFFER_METRICS);
                assertEquals(transport.isAdbStream() ? P5cControlProtocol.CAP_AUDIO_ADB_STREAM : 0,
                        P5cControlProtocol.getUnsignedInt(caps.payload, 12) & P5cControlProtocol.CAP_AUDIO_ADB_STREAM);
                assertEquals(transport.isAdbStream() ? 0 : P5cControlProtocol.CAP_AOA_SESSION_RESTART,
                        P5cControlProtocol.getUnsignedInt(caps.payload, 12) & P5cControlProtocol.CAP_AOA_SESSION_RESTART);

                sendControl(io, keys, hostSession.sessionId, 1, P5cControlProtocol.BIND, new byte[] {0, 0});
                assertAck(io, keys, hostSession.sessionId, 1, P5cControlProtocol.BIND,
                        P5cReceiverCoordinator.STATE_TRANSPORT_CONNECTED);
                sendControl(io, keys, hostSession.sessionId, 2, P5cControlProtocol.SELECT_MODE, new byte[] {0});
                assertAck(io, keys, hostSession.sessionId, 2, P5cControlProtocol.SELECT_MODE,
                        P5cReceiverCoordinator.STATE_TRANSPORT_CONNECTED);
                sendControl(io, keys, hostSession.sessionId, 3, P5cControlProtocol.START, new byte[0]);
                assertAck(io, keys, hostSession.sessionId, 3, P5cControlProtocol.START,
                        P5cReceiverCoordinator.STATE_RECEIVER_RUNNING);
                assertNotNull("START ACK must follow endpoint registration", endpoint.context);
                assertEquals(profile, endpoint.context.formatV2.profile);
                assertEquals(transport.telemetry, endpoint.context.transport);
                assertEquals(profile == 1, endpoint.context.pcmQueue != null);
                assertEquals(profile > 1, endpoint.context.floatQueue != null);

                io.send(AnpuRecordCodec.AUDIO, packet(hostSession.sessionId, keys.hostToReceiverAudio,
                        0, 0, format.encode()));
                io.send(AnpuRecordCodec.AUDIO, packet(hostSession.sessionId, keys.hostToReceiverAudio,
                        1, 0, new byte[format.frameBytes]));
                long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
                while (!endpoint.context.ingress.hasAuthenticatedAudio() && System.nanoTime() < deadline)
                    Thread.sleep(2);
                assertTrue("authenticated format/audio must enter the profile queue", endpoint.context.ingress.hasAuthenticatedAudio());

                sendControl(io, keys, hostSession.sessionId, 4, P5cControlProtocol.PAUSE, new byte[0]);
                assertAck(io, keys, hostSession.sessionId, 4, P5cControlProtocol.PAUSE,
                        P5cReceiverCoordinator.STATE_PAUSED);
                sendControl(io, keys, hostSession.sessionId, 5, P5cControlProtocol.RESUME, new byte[0]);
                assertAck(io, keys, hostSession.sessionId, 5, P5cControlProtocol.RESUME,
                        P5cReceiverCoordinator.STATE_RECEIVER_RUNNING);
                if (stopFromPhone || stopFromHost) {
                    if (stopFromPhone) assertTrue(coordinator.requestUserStop());
                    else {
                        assertTrue(coordinator.requestHostStop());
                        assertFalse(coordinator.shouldRecoverUnexpectedFailure());
                    }
                    assertFalse("authenticated stop waits for an ACK before stopping playback", endpoint.stopped);
                    sendControl(io, keys, hostSession.sessionId, 6,
                            P5cControlProtocol.PLAYBACK_PROGRESS_QUERY, new byte[0]);
                    P5cControlProtocol.Frame stopped = receiveControl(io, keys, hostSession.sessionId,
                            P5cControlProtocol.ACK);
                    assertEquals(P5cControlProtocol.PLAYBACK_PROGRESS_QUERY, stopped.payload[0] & 0xff);
                    assertEquals(6, P5cControlProtocol.getLong(stopped.payload, 1));
                    assertEquals(stopFromPhone ? 3 : P5cReceiverCoordinator.RESULT_OK, stopped.payload[9] & 0xff);
                    assertEquals(P5cReceiverCoordinator.STATE_STOPPED, stopped.payload[10] & 0xff);
                } else {
                    sendControl(io, keys, hostSession.sessionId, 6,
                            P5cControlProtocol.PLAYBACK_PROGRESS_QUERY, new byte[0]);
                    assertEquals(13 + 78, receiveControl(io, keys, hostSession.sessionId,
                            P5cControlProtocol.ACK).payload.length);
                    sendControl(io, keys, hostSession.sessionId, 7, P5cControlProtocol.PLAYBACK_PROGRESS_QUERY,
                            new byte[] {2});
                    assertEquals(13 + 204, receiveControl(io, keys, hostSession.sessionId,
                            P5cControlProtocol.ACK).payload.length);
                    sendControl(io, keys, hostSession.sessionId, 8, P5cControlProtocol.PLAYBACK_PROGRESS_QUERY,
                            new byte[] {3});
                    assertEquals(13 + 221, receiveControl(io, keys, hostSession.sessionId,
                            P5cControlProtocol.ACK).payload.length);
                    sendControl(io, keys, hostSession.sessionId, 9, P5cControlProtocol.STOP, new byte[0]);
                    assertAck(io, keys, hostSession.sessionId, 9, P5cControlProtocol.STOP,
                            P5cReceiverCoordinator.STATE_STOPPED);
                }
                assertTrue(coordinator.awaitTermination(3, TimeUnit.SECONDS));
                assertTrue(endpoint.stopped);
                assertTrue(io.ownerClosed);
            }
        } finally {
            if (keys != null) keys.close();
            coordinator.close();
            coordinator.awaitTermination(3, TimeUnit.SECONDS);
            saved.close();
        }
    }

    private static void sendControl(Duplex io, PairingProtocol.SessionKeys keys, long sid,
            long sequence, int type, byte[] payload) throws Exception {
        io.send(AnpuRecordCodec.CONTROL, P5cControlProtocol.encode(keys.hostToReceiverControl,
                P5cControlProtocol.HOST_TO_RECEIVER, sid, sequence, type, payload));
    }

    private static P5cControlProtocol.Frame receiveControl(Duplex io, PairingProtocol.SessionKeys keys,
            long sid, int expectedType) throws Exception {
        AnpuRecordCodec.Record record = io.receive();
        assertEquals(AnpuRecordCodec.CONTROL, record.type);
        P5cControlProtocol.Frame frame = P5cControlProtocol.decodeRaw(record.body,
                keys.receiverToHostControl, P5cControlProtocol.RECEIVER_TO_HOST, sid);
        assertEquals(expectedType, frame.type);
        return frame;
    }

    private static void assertAck(Duplex io, PairingProtocol.SessionKeys keys, long sid,
            long sequence, int command, int state) throws Exception {
        P5cControlProtocol.Frame ack = receiveControl(io, keys, sid, P5cControlProtocol.ACK);
        assertEquals(command, ack.payload[0] & 0xff);
        assertEquals(sequence, P5cControlProtocol.getLong(ack.payload, 1));
        assertEquals(P5cReceiverCoordinator.RESULT_OK, ack.payload[9] & 0xff);
        assertEquals(state, ack.payload[10] & 0xff);
        assertEquals(0, P5cControlProtocol.getUnsignedShort(ack.payload, 11));
    }

    private static byte[] packet(long sid, byte[] key, long sequence, long frameIndex, byte[] plaintext) throws Exception {
        byte[] header = new byte[UdpPacketDecoder.HEADER_BYTES];
        ByteBuffer b = ByteBuffer.wrap(header).order(ByteOrder.BIG_ENDIAN);
        b.put(new byte[] {'A', 'N', 'P', 'D'}).put((byte) 2)
                .put((byte) (sequence == 0 ? UdpPacketDecoder.FORMAT : UdpPacketDecoder.AUDIO))
                .putShort((short) 0).putLong(sid).putLong(sequence).putLong(frameIndex).putShort((short) plaintext.length);
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN).putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] encrypted = cipher.doFinal(plaintext);
        byte[] datagram = Arrays.copyOf(header, header.length + encrypted.length);
        System.arraycopy(encrypted, 0, datagram, header.length, encrypted.length);
        return datagram;
    }

    private static byte[] sequence(int start, int length) {
        byte[] bytes = new byte[length];
        for (int i = 0; i < length; i++) bytes[i] = (byte) (start + i);
        return bytes;
    }

    private static byte[] filled(int length, int value) {
        byte[] output = new byte[length];
        Arrays.fill(output, (byte) value);
        return output;
    }

    private static byte[] decodeHex(String value) {
        byte[] output = new byte[value.length() / 2];
        for (int index = 0; index < output.length; index++)
            output[index] = (byte) Integer.parseInt(value.substring(index * 2, index * 2 + 2), 16);
        return output;
    }

    private static final class Endpoint implements AoaReceiverCoordinator.PlaybackEndpoint {
        volatile AoaSessionContext context;
        volatile boolean stopped;
        @Override public boolean isCurrent(String ownerId, long generation) { return true; }
        @Override public boolean supportsAudio(UdpPacketDecoder.AudioFormatV2 format) { return format.profile >= 1 && format.profile <= 5; }
        @Override public void start(AoaSessionContext context) { this.context = context; }
        @Override public void pause() { }
        @Override public void resume() { }
        @Override public void stop() { stopped = true; }
        @Override public boolean streaming() { return false; }
        @Override public boolean supportsPlaybackProgressExtension() { return true; }
        @Override public byte[] playbackProgressPayloadV2() { return new byte[78]; }
        @Override public byte[] playbackProgressPayloadExtendedV2() { return new byte[204]; }
        @Override public byte[] playbackProgressPayloadExtendedV2(int schema) {
            return schema == 2 ? new byte[204] : new byte[221];
        }
    }

    static final class Duplex {
        final Input input = new Input();
        final Output output = new Output();
        volatile boolean ownerClosed;
        AoaAccessoryIo owner() { return new AoaAccessoryIo(input, output, () -> ownerClosed = true, 1000); }
        void send(int type, byte[] body) throws IOException { input.offer(AnpuRecordCodec.encode(type, body)); }
        AnpuRecordCodec.Record receive() throws Exception {
            byte[] record = output.records.poll(2, TimeUnit.SECONDS);
            assertNotNull("timed out awaiting AOA output", record);
            return AnpuRecordCodec.decode(record);
        }
    }

    private static final class Input implements ReadableByteChannel {
        private final ByteArrayOutputStream pending = new ByteArrayOutputStream();
        private boolean open = true;
        synchronized void offer(byte[] bytes) { pending.write(bytes, 0, bytes.length); notifyAll(); }
        @Override public synchronized int read(ByteBuffer dst) throws IOException {
            while (open && pending.size() == 0) try { wait(); }
            catch (InterruptedException error) { Thread.currentThread().interrupt(); throw new IOException(error); }
            if (!open) throw new AsynchronousCloseException();
            byte[] bytes = pending.toByteArray();
            int count = Math.min(dst.remaining(), bytes.length);
            dst.put(bytes, 0, count);
            pending.reset();
            pending.write(bytes, count, bytes.length - count);
            return count;
        }
        @Override public synchronized boolean isOpen() { return open; }
        @Override public synchronized void close() { open = false; notifyAll(); }
    }

    private static final class Output implements WritableByteChannel {
        final BlockingQueue<byte[]> records = new LinkedBlockingQueue<>();
        private final ByteArrayOutputStream pending = new ByteArrayOutputStream();
        private boolean open = true;
        @Override public synchronized int write(ByteBuffer src) throws IOException {
            if (!open) throw new AsynchronousCloseException();
            int count = src.remaining();
            while (src.hasRemaining()) pending.write(src.get());
            parseRecords();
            return count;
        }
        private void parseRecords() throws IOException {
            byte[] bytes = pending.toByteArray();
            int offset = 0;
            while (bytes.length - offset >= 8) {
                int body = ((bytes[offset + 6] & 0xff) << 8) | (bytes[offset + 7] & 0xff);
                int size = 8 + body;
                if (bytes.length - offset < size) break;
                records.offer(Arrays.copyOfRange(bytes, offset, offset + size));
                offset += size;
            }
            pending.reset();
            pending.write(bytes, offset, bytes.length - offset);
        }
        @Override public synchronized boolean isOpen() { return open; }
        @Override public synchronized void close() { open = false; }
    }
}
