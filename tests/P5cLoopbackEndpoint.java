package app.anpod.receiver;

import java.io.IOException;
import java.io.InputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.util.Arrays;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/** Test-only Java endpoint used by Rust's merged P5-C localhost check. */
public final class P5cLoopbackEndpoint {
    public static void main(String[] args) throws Exception {
        if (args.length == 1 && "--verify-coordinator-discovery-recovery".equals(args[0])) {
            verifyCoordinatorDiscoveryRecovery();
            System.out.println("JAVA_P5C_DISCOVERY_RECOVERY=PASS");
            return;
        }
        if (args.length == 2 && "--verify-v2-discovery".equals(args[0])) {
            verifyV2DiscoveryLoopback(InetAddress.getByName(args[1]));
            System.out.println("JAVA_V2_DISCOVERY=PASS");
            return;
        }
        verifyCapabilityPayloadVectors();
        verifyV2DiscoveryVectors();
        verifyFragmentedControlFrame();
        verifyHeartbeatDeadline();
        verifyAuthenticatedParametersQueueWithoutFormat();
        verifyQueueAgeBurstAndLayeredBudget();
        verifyPlaybackActivityWindow();
        if (Arrays.asList(args).contains("--self-check-only")) {
            System.out.println("P5-C queue and playback activity checks passed.");
            return;
        }
        PairingProtocol.BindingMaterial binding = new PairingProtocol.BindingMaterial(
                repeated((byte) 0x11, 16), repeated((byte) 0x22, 16), repeated((byte) 0x33, 32));
        boolean verifyModeSwitch = Arrays.asList(args).contains("--verify-mode-switch");
        boolean verifyAudioWindow = Arrays.asList(args).contains("--verify-audio-window");
        FakePlayback playback = new FakePlayback(Arrays.asList(args).contains("--fail-resume"),
                verifyModeSwitch, verifyAudioWindow);
        String capabilityMode = Arrays.stream(args).filter(value -> value.startsWith("--capabilities-"))
                .map(value -> value.substring("--capabilities-".length())).findFirst().orElse("");
        P5cReceiverCoordinator opened = Arrays.asList(args).contains("--connection-intent")
                ? P5cReceiverCoordinator.openLoopbackIntentTest(binding, playback, repeated((byte) 0x5a, 16))
                : !capabilityMode.isEmpty()
                ? P5cReceiverCoordinator.openLoopbackCapabilityTest(binding, playback, capabilityMode)
                : Arrays.asList(args).contains("--stop-ack-drop")
                ? P5cReceiverCoordinator.openLoopbackDropStopAckForTest(binding, playback)
                : Arrays.asList(args).contains("--discovery")
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"), 32)
                : P5cReceiverCoordinator.openLoopback(binding, playback);
        try (P5cReceiverCoordinator endpoint = opened) {
            if (Arrays.asList(args).contains("--cancel-before-handoff")) playback.beforeStart = endpoint::cancelPendingConnectionIntent;
            endpoint.start();
            if (Arrays.asList(args).contains("--discovery")) {
                endpoint.startDiscovery();
                verifyInvalidDiscoveryDoesNotStartPlayback(playback);
            }
            System.out.println("PORT=" + endpoint.port());
            System.out.flush();
            if (Arrays.asList(args).contains("--cancel-intent-on-stdin")) {
                Thread cancel = new Thread(() -> {
                    try {
                        if (System.in.read() < 0) throw new IOException("missing cancel signal");
                        endpoint.cancelPendingConnectionIntent();
                        System.out.println("INTENT_CANCELLED");
                        System.out.flush();
                    } catch (IOException error) { throw new RuntimeException(error); }
                });
                cancel.setDaemon(true);
                cancel.start();
            }
            if (args.length > 0 && "--drop-on-stdin".equals(args[0])) {
                if (System.in.read() < 0) throw new IOException("P5-D test disconnect signal was not supplied");
                endpoint.close();
            }
            if (!endpoint.await(15, TimeUnit.SECONDS)) throw new IOException("P5-C test endpoint timed out");
            endpoint.close();
            if (verifyModeSwitch) playback.verifyModeSwitch();
            System.out.println("RESULT=" + endpoint.failure() + "," + playback.starts.get() + ","
                    + playback.pauses.get() + "," + playback.resumes.get() + "," + playback.stops.get() + ","
                    + playback.pcmBytes.get() + "," + playback.udpClosed.get() + "," + playback.shutdowns.get());
            System.out.flush();
        }
    }

    private static void verifyCapabilityPayloadVectors() {
        byte[] query = P5cControlProtocol.capabilitiesQueryPayload(3, 1,
                P5cControlProtocol.CAP_AUTHENTICATED_STATUS);
        byte[] expectedQuery = {0, 3, 1, 0, 0, 0, 0, 2};
        if (!Arrays.equals(query, expectedQuery)) throw new AssertionError("CAPABILITIES_QUERY vector changed");
        byte[] info = P5cControlProtocol.capabilitiesInfoPayload(0, 3, 1, 0, 3, 4);
        byte[] expectedInfo = {0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 1, 0,
                0, 0, 0, 3, 0, 0, 0, 4};
        if (!Arrays.equals(info, expectedInfo)) throw new AssertionError("CAPABILITIES_INFO vector changed");
    }

    private static void verifyV2DiscoveryVectors() {
        byte[] nonce = repeated((byte) 0x5a, 16);
        byte[] probe = P5cDiscovery.hostProbe(nonce);
        byte[] expectedProbe = new byte[24];
        expectedProbe[0] = 'A'; expectedProbe[1] = 'N'; expectedProbe[2] = 'P'; expectedProbe[3] = 'D';
        expectedProbe[4] = 2; expectedProbe[5] = 3; System.arraycopy(nonce, 0, expectedProbe, 8, 16);
        if (!Arrays.equals(probe, expectedProbe)) throw new AssertionError("V2 HOST_PROBE vector changed");
        byte[] candidate = P5cDiscovery.hostPacket(4, nonce, P5cDiscovery.PORT);
        byte[] expectedCandidate = new byte[26]; System.arraycopy(expectedProbe, 0, expectedCandidate, 0, 24);
        expectedCandidate[5] = 4; expectedCandidate[24] = (byte) (P5cDiscovery.PORT >>> 8);
        expectedCandidate[25] = (byte) P5cDiscovery.PORT;
        if (!Arrays.equals(candidate, expectedCandidate)) throw new AssertionError("V2 HOST_CANDIDATE vector changed");
        byte[] offer = P5cDiscovery.hostPacket(5, nonce, P5cDiscovery.PORT);
        expectedCandidate[5] = 5;
        if (!Arrays.equals(offer, expectedCandidate)) throw new AssertionError("V2 CONNECT_OFFER vector changed");
    }

    private static void verifyV2DiscoveryLoopback(InetAddress hostAddress) throws Exception {
        try (P5cDiscovery discovery = P5cDiscovery.openLoopbackForTest(
                InetAddress.getByName("127.0.0.1"), 8, 45_678)) {
            discovery.start();
            P5cDiscovery.SearchSnapshot search = discovery.startSearchForTest(hostAddress, true);
            if (!search.active || !search.candidates.isEmpty()) throw new AssertionError("loopback search did not start cleanly");
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
            P5cDiscovery.SearchCandidate candidate = null;
            while (System.nanoTime() < deadline) {
                P5cDiscovery.SearchSnapshot current = discovery.searchSnapshot();
                if (!current.candidates.isEmpty()) { candidate = current.candidates.get(0); break; }
                Thread.sleep(10);
            }
            if (candidate == null || !hostAddress.getHostAddress().equals(candidate.address)
                    || candidate.offerPort != P5cDiscovery.PORT)
                throw new AssertionError("Rust PC candidate was not received by the Java search path");
            if (!"sent".equals(discovery.searchSnapshot().lastRequestStatus))
                throw new AssertionError("Java automatic CONNECT_OFFER was not sent");
            try { discovery.requestConnection(candidate.id); throw new AssertionError("duplicate offer accepted"); }
            catch (IOException expected) { }
            P5cDiscovery.SearchSnapshot cancelled = discovery.stopSearch();
            if (cancelled.active || !cancelled.candidates.isEmpty() || !"stopped".equals(cancelled.lastRequestStatus))
                throw new AssertionError("Java search cancellation did not clear candidates");
        }
    }

    private static void verifyCoordinatorDiscoveryRecovery() throws Exception {
        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        AtomicInteger socketBinds = new AtomicInteger();
        PairingProtocol.BindingMaterial binding = new PairingProtocol.BindingMaterial(
                repeated((byte) 0x11, 16), repeated((byte) 0x22, 16), repeated((byte) 0x33, 32));
        FakePlayback playback = new FakePlayback(false, false, false);
        try (P5cReceiverCoordinator endpoint = P5cReceiverCoordinator.openOnInterface(
                binding, playback, loopback, 32)) {
            endpoint.start();
            try (Socket first = new Socket(loopback, endpoint.port())) {
                awaitActiveSocket(endpoint);
                endpoint.startDiscovery(socket -> socketBinds.incrementAndGet());
                if (socketBinds.get() != 0)
                    throw new AssertionError("discovery started while the accepted session was active");
            }
            awaitDiscoveryBinds(endpoint, socketBinds, 2);

            try (Socket second = new Socket(loopback, endpoint.port())) {
                awaitActiveSocket(endpoint);
            }
            awaitDiscoveryBinds(endpoint, socketBinds, 4);
            if (endpoint.failure().startsWith("listener_failed"))
                throw new AssertionError("discovery listener did not recover: " + endpoint.failure());
        }
    }

    private static void awaitActiveSocket(P5cReceiverCoordinator endpoint) throws Exception {
        java.lang.reflect.Field activeSocket = P5cReceiverCoordinator.class.getDeclaredField("activeSocket");
        activeSocket.setAccessible(true);
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
        while (System.nanoTime() < deadline) {
            if (activeSocket.get(endpoint) != null) return;
            Thread.sleep(5);
        }
        throw new AssertionError("coordinator did not accept the localhost session");
    }

    private static void awaitDiscoveryBinds(P5cReceiverCoordinator endpoint, AtomicInteger socketBinds,
            int expected) throws Exception {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
        while (System.nanoTime() < deadline) {
            if (socketBinds.get() >= expected) {
                if (endpoint.failure().startsWith("listener_failed"))
                    throw new AssertionError("discovery listener did not recover: " + endpoint.failure());
                return;
            }
            Thread.sleep(5);
        }
        throw new AssertionError("discovery sockets were not rebound after the session ended: "
                + endpoint.failure() + ", binds=" + socketBinds.get());
    }

    private static void verifyInvalidDiscoveryDoesNotStartPlayback(FakePlayback playback) throws IOException {
        try {
            P5cReceiverCoordinator.openOnInterface(null, playback, InetAddress.getByName("127.0.0.1"), 24);
            throw new AssertionError("loopback test interface accepted a mismatched prefix");
        } catch (IOException expected) { }
        try {
            if (!P5cReceiverCoordinator.validInterfaceAddress(InetAddress.getByName("192.168.41.12"), 24))
                throw new AssertionError("synthetic private IPv4 address/prefix was rejected");
        } catch (IOException unexpected) {
            throw new AssertionError("synthetic interface contract check failed", unexpected);
        }
        if (!UdpSessionContext.isPrivateIpv4(InetAddress.getByName("192.168.41.12"))
                || UdpSessionContext.isPrivateIpv4(InetAddress.getByName("203.0.113.8")))
            throw new AssertionError("authenticated UDP context address contract changed");
        if (!P5cDiscovery.isInSelectedPrefix(InetAddress.getByName("192.168.41.12"), 24,
                InetAddress.getByName("192.168.41.200"))
                || P5cDiscovery.isInSelectedPrefix(InetAddress.getByName("192.168.41.12"), 24,
                        InetAddress.getByName("192.168.42.1")))
            throw new AssertionError("discovery accepted a source outside the selected prefix");
        try (DatagramSocket probe = new DatagramSocket(0, InetAddress.getByName("127.0.0.1"))) {
            probe.setSoTimeout(100);
            byte[] malformed = new byte[26];
            probe.send(new DatagramPacket(malformed, malformed.length,
                    new InetSocketAddress(InetAddress.getByName("127.0.0.1"), P5cDiscovery.PORT)));
            try {
                probe.receive(new DatagramPacket(new byte[64], 64));
                throw new AssertionError("malformed discovery probe received a response");
            } catch (SocketTimeoutException expected) { }
        }
        if (playback.starts.get() != 0 || playback.stops.get() != 0)
            throw new AssertionError("discovery or malformed traffic changed playback before authentication");
    }

    private static void verifyFragmentedControlFrame() throws Exception {
        byte[] key = new byte[32];
        byte[] frame = P5cControlProtocol.encode(key, P5cControlProtocol.HOST_TO_RECEIVER,
                7, 0, P5cControlProtocol.HEARTBEAT, new byte[0]);
        AtomicReference<Throwable> failure = new AtomicReference<>();
        try (ServerSocket listener = new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))) {
            Thread reader = new Thread(() -> {
                try (Socket socket = listener.accept()) {
                    socket.setSoTimeout(25);
                    P5cControlProtocol.Frame parsed = P5cControlProtocol.read(
                            new DataInputStream(socket.getInputStream()), key,
                            P5cControlProtocol.HOST_TO_RECEIVER, 7);
                    if (parsed.type != P5cControlProtocol.HEARTBEAT || parsed.sequence != 0)
                        throw new AssertionError("fragmented heartbeat changed");
                } catch (Throwable error) { failure.set(error); }
            }, "P5-C fragmented-frame parser check");
            reader.start();
            try (Socket client = new Socket(InetAddress.getByName("127.0.0.1"), listener.getLocalPort())) {
                DataOutputStream output = new DataOutputStream(client.getOutputStream());
                output.writeByte(frame.length >>> 8);
                output.flush();
                Thread.sleep(100);
                output.writeByte(frame.length);
                output.write(frame);
                output.flush();
            }
            reader.join(2_000);
            if (reader.isAlive()) throw new AssertionError("fragmented-frame parser did not finish");
        }
        if (failure.get() != null) throw new AssertionError("fragmented-frame parser failed", failure.get());
    }

    private static void verifyHeartbeatDeadline() throws Exception {
        P5cReceiverCoordinator.checkHeartbeatDeadline(5_000_000_000L, 0);
        try {
            P5cReceiverCoordinator.checkHeartbeatDeadline(5_000_000_001L, 0);
            throw new AssertionError("expired authenticated heartbeat was accepted");
        } catch (IOException expected) { }
    }

    private static void verifyAuthenticatedParametersQueueWithoutFormat() throws Exception {
        byte[] key = repeated((byte) 0x5a, 32);
        long sessionId = 0x1122334455667788L;
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        UdpPcmQueue queue = new UdpPcmQueue(parameters);
        UdpPacketDecoder decoder = new UdpPacketDecoder(sessionId, key);
        try {
            InputStream pcm = queue.asPcmInputStream();
            byte[] header = new byte[8];
            DataInputStream pcmInput = new DataInputStream(pcm);
            pcmInput.readFully(header);
            if (littleInt(header, 0) != 48_000 || littleShort(header, 4) != 2 || littleShort(header, 6) != 16)
                throw new AssertionError("authenticated parameters did not initialize the PCM header");

            long receivedAt = System.nanoTime();
            byte[] sequenceZeroAudio = encodeUdp(key, sessionId, 0, 0,
                    UdpPacketDecoder.AUDIO, pcmPayload(240));
            try {
                queue.accept(decoder.open(sequenceZeroAudio, sequenceZeroAudio.length), decoder);
                throw new AssertionError("AUDIO reused FORMAT's reserved sequence zero");
            } catch (UdpPacketDecoder.ProtocolException expected) {
                if (expected.reason != UdpPacketDecoder.ProtocolException.FORMAT) throw expected;
            }
            for (int index = 0; index < 3; index++) {
                byte[] audio = encodeUdp(key, sessionId, index + 1, index * 240L,
                        UdpPacketDecoder.AUDIO, pcmPayload(240));
                queue.acceptAt(decoder.open(audio, audio.length), decoder, receivedAt + index);
            }
            long lastReceiveAt = receivedAt + 2;
            if (!queue.hasAuthenticatedAudio() || queue.lastAuthenticatedAudioAtNanos() != lastReceiveAt
                    || queue.size() != 3)
                throw new AssertionError("authenticated AUDIO did not start the parameterized queue without FORMAT");

            int targetBefore = queue.targetFrames();
            byte[] mismatch = encodeUdp(key, sessionId, 0, 0, UdpPacketDecoder.FORMAT, formatPayload(44_100, 2));
            expectQueueFormatRejected(queue, decoder, mismatch);
            if (queue.size() != 3 || queue.targetFrames() != targetBefore
                    || queue.lastAuthenticatedAudioAtNanos() != lastReceiveAt)
                throw new AssertionError("mismatched late FORMAT changed queue, budget, or receive activity");

            byte[] matching = encodeUdp(key, sessionId, 0, 0, UdpPacketDecoder.FORMAT, formatPayload(48_000, 2));
            UdpPacketDecoder.Packet matchingPacket = decoder.open(matching, matching.length);
            queue.accept(matchingPacket, decoder);
            if (queue.size() != 3 || queue.targetFrames() != targetBefore
                    || queue.format().sampleRate != 48_000 || queue.format().channels != 2)
                throw new AssertionError("matching late FORMAT reset playback or the authenticated format");
            try {
                decoder.open(matching, matching.length);
                throw new AssertionError("late FORMAT replay was accepted");
            } catch (UdpPacketDecoder.ProtocolException expected) {
                if (expected.reason != UdpPacketDecoder.ProtocolException.REPLAY) throw expected;
            }

            byte[] payload = new byte[2_880];
            pcmInput.readFully(payload);
            if (payload[0] != 0x34 || payload[1] != 0x12)
                throw new AssertionError("late FORMAT reset or corrupted the active PCM stream");

            byte[] badTag = encodeUdp(key, sessionId, 4, 720, UdpPacketDecoder.AUDIO, pcmPayload(240));
            badTag[badTag.length - 1] ^= 0x40;
            try {
                decoder.open(badTag, badTag.length);
                throw new AssertionError("bad audio tag was accepted");
            } catch (UdpPacketDecoder.ProtocolException expected) {
                if (expected.reason != UdpPacketDecoder.ProtocolException.AUTHENTICATION) throw expected;
            }
            byte[] wrongSession = encodeUdp(key, sessionId + 1, 4, 720, UdpPacketDecoder.AUDIO, pcmPayload(240));
            try {
                decoder.open(wrongSession, wrongSession.length);
                throw new AssertionError("wrong-session audio was accepted");
            } catch (UdpPacketDecoder.ProtocolException expected) {
                if (expected.reason != UdpPacketDecoder.ProtocolException.IDENTITY) throw expected;
            }
            if (queue.size() != 0 || queue.lastAuthenticatedAudioAtNanos() != lastReceiveAt)
                throw new AssertionError("unauthenticated input changed queue or receive activity");
        } finally {
            queue.stop();
            decoder.destroy();
            Arrays.fill(key, (byte) 0);
        }
    }

    private static void expectQueueFormatRejected(UdpPcmQueue queue, UdpPacketDecoder decoder, byte[] datagram)
            throws Exception {
        UdpPacketDecoder.Packet packet = decoder.open(datagram, datagram.length);
        try {
            queue.accept(packet, decoder);
            throw new AssertionError("mismatched late FORMAT was accepted");
        } catch (UdpPacketDecoder.ProtocolException expected) {
            if (expected.reason != UdpPacketDecoder.ProtocolException.FORMAT) throw expected;
        }
    }

    private static void verifyPlaybackActivityWindow() {
        UdpPlaybackActivity activity = new UdpPlaybackActivity();
        long now = 1_000_000_000L;
        activity.resetPlaybackHead(0);
        activity.noteSuccessfulWrite(now);
        if (activity.isActive(now + 10, 0, true, now, false))
            throw new AssertionError("writes without AudioTrack consumption were reported as streaming");
        if (!activity.isActive(now + 20, 240, true, now, false))
            throw new AssertionError("recent packet, successful write, and head advance were not reported active");
        if (activity.isActive(now + 20, 480, true, now, true))
            throw new AssertionError("paused playback was reported streaming");
        if (activity.isActive(now + 9_000_000_000L, 480, true, now, false))
            throw new AssertionError("stale audio activity remained streaming after the waiting window");

        UdpPlaybackActivity wrapped = new UdpPlaybackActivity();
        wrapped.resetPlaybackHead(-2);
        wrapped.noteSuccessfulWrite(now);
        if (!wrapped.isActive(now + 20, 2, true, now, false))
            throw new AssertionError("unsigned AudioTrack head wrap was treated as a reset");
        UdpPlaybackActivity freshSession = new UdpPlaybackActivity();
        freshSession.resetPlaybackHead(0);
        if (freshSession.isActive(now + 20, 0, true, now, false))
            throw new AssertionError("playback activity leaked across audio sessions");
    }

    private static void verifyQueueAgeBurstAndLayeredBudget() throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        UdpPcmQueue budget = new UdpPcmQueue(parameters);
        budget.configureAudioTrack(3_840);
        if (budget.targetFrames() != 720)
            throw new AssertionError("80 ms AudioTrack pre-roll consumed the 15 ms network jitter target");
        budget.reportTrackUnderruns(0);
        budget.reportTrackUnderruns(100);
        if (budget.targetFrames() != 1_920)
            throw new AssertionError("balanced network jitter target did not retain its independent 40 ms ceiling");
        budget.stop();

        byte[] key = repeated((byte) 0x6a, 32);
        long sessionId = 0x1020304050607080L;
        UdpPcmQueue queue = new UdpPcmQueue(parameters);
        UdpPacketDecoder decoder = new UdpPacketDecoder(sessionId, key);
        try {
            long receivedAt = System.nanoTime();
            byte[] small = encodeUdp(key, sessionId, 1, 0, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(120, (short) 0x1111));
            queue.acceptAt(decoder.open(small, small.length), decoder, receivedAt);
            byte[] medium = encodeUdp(key, sessionId, 2, 120, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(240, (short) 0x2222));
            queue.acceptAt(decoder.open(medium, medium.length), decoder, receivedAt + 1);
            byte[] large = encodeUdp(key, sessionId, 3, 360, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(240, (short) 0x3333));
            queue.acceptAt(decoder.open(large, large.length), decoder, receivedAt + 2);
            queue.expireStalePackets(receivedAt + 41_000_003L);
            if (queue.size() != 0 || queue.droppedFrames() != 600 || queue.expiredFrames() != 600)
                throw new AssertionError("stalled playback did not expire variable-size old audio by exact frame count");

            byte[] fresh = encodeUdp(key, sessionId, 4, 600, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(240, (short) 0x1234));
            queue.accept(decoder.open(fresh, fresh.length), decoder);
            InputStream pcm = queue.asPcmInputStream();
            DataInputStream pcmInput = new DataInputStream(pcm);
            byte[] header = new byte[8];
            pcmInput.readFully(header);
            byte[] first = new byte[960];
            pcmInput.readFully(first);

            byte[] afterGap = encodeUdp(key, sessionId, 5, 1_080, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(240, (short) 0x1234));
            queue.accept(decoder.open(afterGap, afterGap.length), decoder);
            byte[] recovered = new byte[960];
            pcmInput.readFully(recovered);
            if (littleShort(recovered, 0) == 0x1234 || littleShort(recovered, 600) != 0x1234)
                throw new AssertionError("burst gap was not faded into the newest packet");
            pcm.close();
        } finally {
            queue.stop();
            decoder.destroy();
            Arrays.fill(key, (byte) 0);
        }
    }

    private static byte[] encodeUdp(byte[] key, long sessionId, long sequence, long frameIndex,
            int type, byte[] payload) throws Exception {
        byte[] header = ByteBuffer.allocate(UdpPacketDecoder.HEADER_BYTES).order(ByteOrder.BIG_ENDIAN)
                .put(new byte[] {'A', 'N', 'P', 'D'}).put((byte) 1).put((byte) type).putShort((short) 0)
                .putLong(sessionId).putLong(sequence).putLong(frameIndex).putShort((short) payload.length).array();
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN).putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] encrypted = cipher.doFinal(payload);
        return ByteBuffer.allocate(header.length + encrypted.length).put(header).put(encrypted).array();
    }

    private static byte[] formatPayload(int sampleRate, int channels) {
        return ByteBuffer.allocate(7).order(ByteOrder.BIG_ENDIAN).put((byte) 1).putShort((short) channels)
                .putInt(sampleRate).array();
    }

    private static byte[] pcmPayload(int frames) {
        byte[] payload = new byte[frames * 4];
        for (int i = 0; i < payload.length; i += 2) {
            payload[i] = 0x34;
            payload[i + 1] = 0x12;
        }
        return payload;
    }

    private static byte[] pcmPayloadValue(int frames, short sample) {
        byte[] payload = new byte[frames * 4];
        for (int i = 0; i < payload.length; i += 2) {
            payload[i] = (byte) sample;
            payload[i + 1] = (byte) (sample >>> 8);
        }
        return payload;
    }

    private static int littleInt(byte[] data, int offset) {
        return (data[offset] & 0xff) | ((data[offset + 1] & 0xff) << 8)
                | ((data[offset + 2] & 0xff) << 16) | (data[offset + 3] << 24);
    }

    private static int littleShort(byte[] data, int offset) {
        return (data[offset] & 0xff) | ((data[offset + 1] & 0xff) << 8);
    }

    private static byte[] repeated(byte value, int length) {
        byte[] bytes = new byte[length];
        Arrays.fill(bytes, value);
        return bytes;
    }

    private static final class FakePlayback implements P5cReceiverCoordinator.PlaybackEndpoint {
        final AtomicInteger starts = new AtomicInteger();
        final AtomicInteger pauses = new AtomicInteger();
        final AtomicInteger resumes = new AtomicInteger();
        final AtomicInteger stops = new AtomicInteger();
        final AtomicInteger shutdowns = new AtomicInteger();
        final AtomicInteger pcmBytes = new AtomicInteger();
        final AtomicBoolean udpClosed = new AtomicBoolean();
        final AtomicBoolean stopped = new AtomicBoolean();
        final AtomicBoolean failResume;
        final AtomicInteger modeSequenceErrors = new AtomicInteger();
        final AtomicReference<String> queueBudgetFailure = new AtomicReference<>();
        final boolean verifyModeSwitch;
        final boolean verifyAudioWindow;
        final AtomicInteger playbackHead = new AtomicInteger();
        final AtomicBoolean paused = new AtomicBoolean();
        final AtomicBoolean audioWindowAged = new AtomicBoolean();
        volatile UdpPlaybackActivity playbackActivity = new UdpPlaybackActivity();
        volatile long activityClockOffsetNanos;
        volatile UdpPcmQueue queue;
        volatile UdpReceiver receiver;
        volatile UdpSessionContext context;
        volatile Thread receiverThread;
        volatile Thread sinkThread;
        Runnable beforeStart = () -> {};

        FakePlayback(boolean failResume, boolean verifyModeSwitch, boolean verifyAudioWindow) {
            this.failResume = new AtomicBoolean(failResume);
            this.verifyModeSwitch = verifyModeSwitch;
            this.verifyAudioWindow = verifyAudioWindow;
        }

        @Override public boolean supportsPlaybackProgress() { return true; }

        @Override public boolean start(UdpSessionContext context, P5cReceiverCoordinator.StartPermission permission) throws IOException {
            beforeStart.run();
            if (!permission.allowed()) return false;
            int startNumber = starts.incrementAndGet();
            if (verifyModeSwitch) {
                int expectedMode = startNumber == 1 ? 1 : 0;
                if (context.parameters.mode != expectedMode) modeSequenceErrors.incrementAndGet();
            }
            stopped.set(false);
            paused.set(false);
            playbackHead.set(0);
            playbackActivity = new UdpPlaybackActivity();
            playbackActivity.resetPlaybackHead(0);
            audioWindowAged.set(false);
            activityClockOffsetNanos = 0;
            this.context = context;
            queue = new UdpPcmQueue(context.parameters);
            UdpPcmQueue sessionQueue = queue;
            receiver = new UdpReceiver(context, queue);
            receiverThread = new Thread(receiver, "P5-C test UDP receiver");
            receiverThread.start();
            InputStream pcm = queue.asPcmInputStream();
            sinkThread = new Thread(() -> {
                try (pcm) {
                    byte[] header = new byte[8];
                    readFully(pcm, header);
                    int preRollFrames = context.parameters.mode == 1 ? 240 : 480;
                    sessionQueue.configureAudioTrack(preRollFrames);
                    if (sessionQueue.targetFrames() < 240)
                        queueBudgetFailure.compareAndSet(null, context.playbackMode + " queue had no 5 ms startup headroom");
                    byte[] buffer = new byte[1024];
                    int count;
                    while ((count = pcm.read(buffer)) >= 0) {
                        if (count != 0) {
                            pcmBytes.addAndGet(count);
                            playbackActivity.noteSuccessfulWrite(System.nanoTime());
                            playbackHead.addAndGet(count / (context.parameters.channels * 2));
                        }
                    }
                } catch (IOException ignored) { }
            }, "P5-C test synthetic PCM sink");
            sinkThread.start();
            return true;
        }

        @Override public void pause() {
            pauses.incrementAndGet(); paused.set(true);
            if (queue != null) queue.setPaused(true);
        }
        @Override public void resume() throws IOException {
            resumes.incrementAndGet();
            if (failResume.compareAndSet(true, false)) throw new IOException("P5-D test RESUME failure");
            paused.set(false);
            if (queue != null) queue.setPaused(false);
        }
        @Override public boolean streaming() {
            UdpPcmQueue current = queue;
            if (current == null) return false;
            long now = System.nanoTime() + activityClockOffsetNanos;
            boolean active = playbackActivity.isActive(now, playbackHead.get(), current.hasAuthenticatedAudio(),
                    current.lastAuthenticatedAudioAtNanos(), paused.get());
            if (active && verifyAudioWindow && audioWindowAged.compareAndSet(false, true))
                activityClockOffsetNanos = 9_000_000_000L;
            return active;
        }
        @Override public void shutdown() { shutdowns.incrementAndGet(); }

        void verifyModeSwitch() throws IOException {
            if (starts.get() != 3 || modeSequenceErrors.get() != 0)
                throw new IOException("initial low-latency session, balanced mode rebuild, and balanced reconnect were not observed");
            String budgetFailure = queueBudgetFailure.get();
            if (budgetFailure != null) throw new IOException(budgetFailure);
        }

        @Override public void stop() {
            stops.incrementAndGet();
            if (!stopped.compareAndSet(false, true)) return;
            UdpReceiver currentReceiver = receiver;
            if (currentReceiver != null) currentReceiver.close();
            UdpSessionContext currentContext = context;
            if (currentContext != null) currentContext.destroy();
            UdpPcmQueue currentQueue = queue;
            if (currentQueue != null) currentQueue.stop();
            join(receiverThread);
            join(sinkThread);
            udpClosed.set(currentReceiver == null || currentReceiver.isClosed());
        }

        private static void join(Thread thread) {
            if (thread == null || thread == Thread.currentThread()) return;
            try { thread.join(1_000); }
            catch (InterruptedException error) { Thread.currentThread().interrupt(); }
        }

        private static void readFully(InputStream input, byte[] bytes) throws IOException {
            int offset = 0;
            while (offset < bytes.length) {
                int count = input.read(bytes, offset, bytes.length - offset);
                if (count < 0) throw new IOException("truncated PCM header");
                offset += count;
            }
        }
    }
}
