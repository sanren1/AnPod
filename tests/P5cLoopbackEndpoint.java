package app.anpod.receiver;

import java.io.IOException;
import java.io.InputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicLong;
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
import com.anpod.server.FloatPcmFrameQueue;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/** Test-only Java endpoint used by Rust's merged P5-C localhost check. */
public final class P5cLoopbackEndpoint {
    public static void main(String[] args) throws Exception {
        if (Arrays.asList(args).contains("--audio-v2-upgrade")) {
            runAudioV2UpgradeFixture(args);
            return;
        }
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
        boolean skipQueueSelfChecks = Arrays.asList(args).contains("--background-auth")
                || Arrays.asList(args).contains("--skip-queue-self-checks");
        verifyCapabilityPayloadVectors();
        verifyV2DiscoveryVectors();
        if (!skipQueueSelfChecks) {
            verifyFragmentedControlFrame();
            verifyHeartbeatDeadline();
            verifyAuthenticatedParametersQueueWithoutFormat();
            verifyQueueAgeBurstAndLayeredBudget();
            verifyPlaybackActivityWindow();
        }
        if (Arrays.asList(args).contains("--self-check-only")) {
            System.out.println("P5-C queue and playback activity checks passed.");
            return;
        }
        PairingProtocol.BindingMaterial binding = new PairingProtocol.BindingMaterial(
                repeated((byte) 0x11, 16), repeated((byte) 0x22, 16), repeated((byte) 0x33, 32));
        byte[] fixtureBindingCommitment = binding.commitment();
        boolean verifyAutoRenewAfterAuth = Arrays.asList(args).contains("--verify-auto-renew-after-auth");
        boolean verifyIntentRenewal = Arrays.asList(args).contains("--verify-intent-renewal");
        boolean verifyModeSwitch = Arrays.asList(args).contains("--verify-mode-switch");
        boolean verifyAudioWindow = Arrays.asList(args).contains("--verify-audio-window");
        FakePlayback playback = new FakePlayback(Arrays.asList(args).contains("--fail-resume"),
                verifyModeSwitch, verifyAudioWindow, Arrays.asList(args).contains("--fail-start-once"),
                Arrays.asList(args).contains("--throw-start-once"), Arrays.asList(args).contains("--throw-stop-once"),
                Arrays.stream(args).filter(value -> value.startsWith("--background-auth-mode="))
                        .map(value -> value.substring("--background-auth-mode=".length())).findFirst().orElse(""));
        String capabilityMode = Arrays.stream(args).filter(value -> value.startsWith("--capabilities-"))
                .map(value -> value.substring("--capabilities-".length())).findFirst().orElse("");
        playback.keepReceiveIntent = verifyAutoRenewAfterAuth || verifyIntentRenewal || verifyModeSwitch;
        playback.backgroundTrustManagement = Arrays.asList(args).contains("--background-trust-management")
                || Arrays.asList(args).contains("--background-trust-wrong-binding");
        playback.delayBackgroundTrustCapability = Arrays.asList(args).contains("--delay-background-trust-capability");
        P5cReceiverCoordinator opened = verifyIntentRenewal || verifyAutoRenewAfterAuth
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"), 32)
                : Arrays.asList(args).contains("--connection-intent")
                ? P5cReceiverCoordinator.openLoopbackIntentTest(binding, playback, repeated((byte) 0x5a, 16))
                : Arrays.asList(args).contains("--background-auth")
                ? P5cReceiverCoordinator.openLoopbackBackgroundAuthorizationTest(binding, playback,
                        Arrays.stream(args).filter(value -> value.startsWith("--background-auth-test="))
                                .map(value -> value.substring("--background-auth-test=".length())).findFirst().orElse(""))
                : !capabilityMode.isEmpty()
                ? P5cReceiverCoordinator.openLoopbackCapabilityTest(binding, playback, capabilityMode)
                : Arrays.asList(args).contains("--stop-ack-drop")
                ? P5cReceiverCoordinator.openLoopbackDropStopAckForTest(binding, playback)
                : Arrays.asList(args).contains("--discovery")
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"), 32)
                : Arrays.asList(args).contains("--fail-start-once")
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"), 32)
                : Arrays.asList(args).contains("--throw-start-once")
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"), 32)
                : Arrays.stream(args).anyMatch(value -> value.startsWith("--drop-after-ms="))
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"), 32)
                : P5cReceiverCoordinator.openLoopback(binding, playback);
        try (P5cReceiverCoordinator endpoint = opened) {
            if (Arrays.asList(args).contains("--background-trust-wrong-binding"))
                endpoint.backgroundTrustTestMode = "wrong-binding";
            if (verifyAutoRenewAfterAuth || verifyIntentRenewal) {
                java.lang.reflect.Method beginIntent = P5cReceiverCoordinator.class
                        .getDeclaredMethod("beginConnectionIntent", byte[].class);
                beginIntent.setAccessible(true);
                beginIntent.invoke(endpoint, repeated((byte) 0x5a, 16));
            }
            if (Arrays.asList(args).contains("--require-connection-intent")) {
                java.lang.reflect.Method beginIntent = P5cReceiverCoordinator.class
                        .getDeclaredMethod("beginConnectionIntent", byte[].class);
                beginIntent.setAccessible(true);
                beginIntent.invoke(endpoint, new Object[] { null });
            }
            if (Arrays.asList(args).contains("--cancel-before-handoff")) playback.beforeStart = endpoint::cancelPendingConnectionIntent;
            endpoint.start();
            P5cDiscovery initialDiscovery = null;
            if (Arrays.asList(args).contains("--discovery") || verifyAutoRenewAfterAuth || verifyIntentRenewal) {
                endpoint.startDiscovery();
                if (Arrays.asList(args).contains("--discovery")) verifyInvalidDiscoveryDoesNotStartPlayback(playback);
                if (verifyAutoRenewAfterAuth) {
                    java.lang.reflect.Field discoveryField = P5cReceiverCoordinator.class.getDeclaredField("discovery");
                    discoveryField.setAccessible(true);
                    initialDiscovery = (P5cDiscovery) discoveryField.get(endpoint);
                }
            }
            System.out.println("PORT=" + endpoint.port());
            System.out.flush();
            if (verifyIntentRenewal) {
                long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15);
                while (!playback.stopped.get() || playback.starts.get() < 1) {
                    if (System.nanoTime() >= deadline) throw new IOException("intent-renewal fixture did not receive its first STOP");
                    Thread.sleep(5);
                }
                java.lang.reflect.Field authenticatedField = P5cReceiverCoordinator.class.getDeclaredField("authenticatedSocket");
                authenticatedField.setAccessible(true);
                while (authenticatedField.get(endpoint) != null) {
                    if (System.nanoTime() >= deadline) throw new IOException("first authenticated session did not close");
                    Thread.sleep(5);
                }
                P5cDiscovery.SearchSnapshot renewed = endpoint.maintainAutomaticSearch();
                if (!renewed.active || !"searching".equals(renewed.lastRequestStatus))
                    throw new AssertionError("receive intent did not begin a fresh search between sessions");
                System.out.println("INTENT_RENEWED");
                System.out.flush();
                while (playback.starts.get() < 2 || !playback.stopped.get()) {
                    if (System.nanoTime() >= deadline) throw new IOException("established receive intent rejected reauthentication after search renewal");
                    Thread.sleep(5);
                }
                while (authenticatedField.get(endpoint) != null) {
                    if (System.nanoTime() >= deadline) throw new IOException("second authenticated session did not close");
                    Thread.sleep(5);
                }
                endpoint.stopSearch();
                java.lang.reflect.Method acceptsIntent = P5cReceiverCoordinator.class
                        .getDeclaredMethod("acceptsConnectionIntent", byte[].class);
                acceptsIntent.setAccessible(true);
                if ((Boolean) acceptsIntent.invoke(endpoint, repeated((byte) 0x5a, 16)))
                    throw new AssertionError("manual STOP must revoke the established receive intent");
                endpoint.close();
                System.out.println("RESULT=" + endpoint.failure() + "," + playback.starts.get() + ","
                        + playback.pauses.get() + "," + playback.resumes.get() + "," + playback.stops.get() + ","
                        + playback.pcmBytes.get() + "," + playback.udpClosed.get() + "," + playback.shutdowns.get());
                System.out.flush();
                return;
            }
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
            if (Arrays.asList(args).contains("--close-on-stdin")) {
                Thread close = new Thread(() -> {
                    try { if (System.in.read() < 0) throw new IOException("missing close signal"); endpoint.close(); }
                    catch (IOException error) { throw new RuntimeException(error); }
                });
                close.setDaemon(true);
                close.start();
            }
            Arrays.stream(args).filter(value -> value.startsWith("--drop-after-ms=")).findFirst().ifPresent(value -> {
                long delay = Long.parseLong(value.substring("--drop-after-ms=".length()));
                Thread drop = new Thread(() -> dropFirstAuthenticatedConnection(endpoint, delay), "P5-C test disconnect");
                drop.setDaemon(true);
                drop.start();
            });
            if (args.length > 0 && "--drop-on-stdin".equals(args[0])) {
                if (System.in.read() < 0) throw new IOException("P5-D test disconnect signal was not supplied");
                endpoint.close();
            }
            if (verifyAutoRenewAfterAuth) {
                long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15);
                while (!playback.stopped.get()) {
                    if (System.nanoTime() >= deadline) throw new IOException("persistent receive test did not receive STOP");
                    Thread.sleep(5);
                }
                if (endpoint.await(0, TimeUnit.MILLISECONDS))
                    throw new AssertionError("PC STOP terminated the persistent receive listener");
            } else if (!endpoint.await(15, TimeUnit.SECONDS)) throw new IOException("P5-C test endpoint timed out");
            String discoveryRecovery = verifyAutoRenewAfterAuth
                    ? verifyAutomaticSearchAfterAuthenticatedConnection(endpoint, initialDiscovery) + "|" : "";
            endpoint.close();
            if (playback.backgroundTrustManagement) {
                P5cReceiverCoordinator.BackgroundTrustSnapshot remainingTrust =
                        playback.queryBackgroundTrust(fixtureBindingCommitment);
                System.out.println("TRUST_REVOKED=" + playback.backgroundTrustRevoked.get()
                        + ",TRUST_CLEARED=" + (remainingTrust.kind == 0 && remainingTrust.persistence == 0
                                && remainingTrust.remainingMs == 0)
                        + ",TRUST_QUERIES=" + playback.backgroundTrustQueries.get()
                        + ",TRUST_CANCELS=" + playback.backgroundAuthCancels.get());
            }
            if (verifyModeSwitch) playback.verifyModeSwitch();
            if (Arrays.asList(args).contains("--verify-background-auth") && playback.backgroundAuthRequests.get() > 0)
                playback.verifyBackgroundAuthorization();
            System.out.println("RESULT=" + discoveryRecovery + endpoint.failure() + "," + playback.starts.get() + ","
                    + playback.pauses.get() + "," + playback.resumes.get() + "," + playback.stops.get() + ","
                    + playback.pcmBytes.get() + "," + playback.udpClosed.get() + "," + playback.shutdowns.get());
            System.out.flush();
        }
    }

    private static void runAudioV2UpgradeFixture(String[] args) throws Exception {
        PairingProtocol.BindingMaterial binding = new PairingProtocol.BindingMaterial(
                repeated((byte) 0x11, 16), repeated((byte) 0x22, 16), repeated((byte) 0x33, 32));
        String authMode = Arrays.stream(args).filter(value -> value.startsWith("--background-auth-mode="))
                .map(value -> value.substring("--background-auth-mode=".length())).findFirst().orElse("");
        FakePlayback playback = new FakePlayback(false, false, false, false, false, false, authMode);
        playback.audioV2Support = true;
        int formalSessions = Arrays.stream(args).filter(value -> value.startsWith("--audio-v2-upgrade-sessions="))
                .map(value -> Integer.parseInt(value.substring("--audio-v2-upgrade-sessions=".length())))
                .findFirst().orElse(1);
        playback.expectedAudioV2Sessions = formalSessions;
        playback.keepReceiveIntent = formalSessions > 1;
        int profile = Arrays.stream(args).filter(value -> value.startsWith("--audio-v2-profile="))
                .map(value -> Integer.parseInt(value.substring("--audio-v2-profile=".length())))
                .findFirst().orElse(2);
        playback.expectedAudioV2Profile = profile;
        P5cReceiverCoordinator endpoint = P5cReceiverCoordinator.openLoopbackAudioV2Test(binding, playback);
        java.lang.reflect.Method beginIntent = P5cReceiverCoordinator.class
                .getDeclaredMethod("beginConnectionIntent", byte[].class);
        beginIntent.setAccessible(true);
        beginIntent.invoke(endpoint, repeated((byte) 0x5a, 16));
        try (endpoint) {
            endpoint.start();
            Thread closeOnInput = new Thread(() -> {
                try {
                    if (System.in.read() >= 0) endpoint.close();
                } catch (IOException error) { throw new RuntimeException(error); }
            }, "P5-C v2 fixture close signal");
            closeOnInput.setDaemon(true);
            closeOnInput.start();
            System.out.println("PORT=" + endpoint.port());
            System.out.flush();
            if (!endpoint.await(25, TimeUnit.SECONDS))
                throw new IOException("audio-v2 upgrade fixture timed out");
            System.out.println("RESULT=" + endpoint.failure() + "," + playback.starts.get() + ","
                    + playback.pauses.get() + "," + playback.resumes.get() + "," + playback.stops.get()
                    + "," + playback.udpClosed.get() + "," + playback.allPcm24SessionsDecoded());
            System.out.flush();
        }
    }

    private static String verifyAutomaticSearchAfterAuthenticatedConnection(P5cReceiverCoordinator endpoint,
            P5cDiscovery initialDiscovery) throws Exception {
        java.lang.reflect.Field authenticatedField = P5cReceiverCoordinator.class.getDeclaredField("authenticatedSocket");
        authenticatedField.setAccessible(true);
        java.lang.reflect.Field discoveryField = P5cReceiverCoordinator.class.getDeclaredField("discovery");
        discoveryField.setAccessible(true);
        java.lang.reflect.Field establishedField = P5cReceiverCoordinator.class.getDeclaredField("connectionIntentEstablished");
        establishedField.setAccessible(true);
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
        P5cDiscovery nextDiscovery = null;
        while (System.nanoTime() < deadline) {
            nextDiscovery = (P5cDiscovery) discoveryField.get(endpoint);
            if (authenticatedField.get(endpoint) == null && nextDiscovery != null && nextDiscovery != initialDiscovery)
                break;
            Thread.sleep(5);
        }
        if (nextDiscovery == null || nextDiscovery == initialDiscovery || authenticatedField.get(endpoint) != null)
            throw new AssertionError("authenticated session did not install a fresh idle discovery service after STOP");
        if (!((Boolean) establishedField.get(endpoint)))
            throw new AssertionError("fixture did not establish the authenticated connection intent before STOP");
        P5cDiscovery.SearchSnapshot renewed = endpoint.maintainAutomaticSearch();
        if (!renewed.active || !"searching".equals(renewed.lastRequestStatus))
            throw new AssertionError("receive-intent maintenance did not restart the fresh post-auth idle search");
        P5cDiscovery.SearchSnapshot stopped = endpoint.stopSearch();
        P5cDiscovery.SearchSnapshot maintained = endpoint.maintainAutomaticSearch();
        if (!"stopped".equals(stopped.lastRequestStatus) || maintained.active
                || !"stopped".equals(maintained.lastRequestStatus))
            throw new AssertionError("post-auth search maintenance ignored explicit STOP");
        return "AUTO_RENEW_AFTER_AUTH=searching;AUTO_RENEW_AFTER_STOP=stopped";
    }

    private static void dropFirstAuthenticatedConnection(P5cReceiverCoordinator endpoint, long delayMs) {
        try {
            java.lang.reflect.Field owner = P5cReceiverCoordinator.class.getDeclaredField("authenticatedSocket");
            owner.setAccessible(true);
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10);
            Socket first = null;
            while (System.nanoTime() < deadline && first == null) {
                first = (Socket) owner.get(endpoint);
                if (first == null) Thread.sleep(5);
            }
            if (first == null) throw new AssertionError("test endpoint did not authenticate a session");
            Thread.sleep(delayMs);
            first.close();
        } catch (Exception error) { throw new RuntimeException(error); }
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
        byte[] legacyProgress = new byte[70];
        legacyProgress[69] = 0x5a;
        byte[] progressV2 = P5cControlProtocol.playbackProgressPayloadV2(legacyProgress, 2, 2, 48_000);
        if (progressV2.length != 78 || progressV2[69] != 0x5a || progressV2[70] != 1
                || progressV2[71] != 2 || progressV2[72] != 2 || progressV2[73] != 0
                || !Arrays.equals(Arrays.copyOfRange(progressV2, 74, 78), new byte[] {0, 0, (byte) 0xbb, (byte) 0x80}))
            throw new AssertionError("v2 playback progress extension changed");
        byte[] unknownProgressV2 = P5cControlProtocol.playbackProgressPayloadV2(legacyProgress, 0, 0, 0);
        if (unknownProgressV2.length != 78 || unknownProgressV2[70] != 1 || unknownProgressV2[71] != 0
                || unknownProgressV2[72] != 0 || unknownProgressV2[73] != 0
                || !Arrays.equals(Arrays.copyOfRange(unknownProgressV2, 74, 78), new byte[4]))
            throw new AssertionError("unknown v2 playback output must remain zero-filled");
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
        FakePlayback playback = new FakePlayback(false, false, false, false, false, false);
        try (P5cReceiverCoordinator endpoint = P5cReceiverCoordinator.openOnInterface(
                binding, playback, loopback, 32)) {
            endpoint.start();
            endpoint.startDiscovery(socket -> socketBinds.incrementAndGet());
            awaitDiscoveryBinds(endpoint, socketBinds, 2);
            try (Socket first = new Socket(loopback, endpoint.port())) {
                awaitActiveSocket(endpoint);
                endpoint.startDiscovery(socket -> socketBinds.incrementAndGet());
                if (socketBinds.get() != 2)
                    throw new AssertionError("unauthenticated connection replaced active discovery sockets");
            }
            Thread.sleep(100);
            if (socketBinds.get() != 2) throw new AssertionError("unauthenticated connection restarted discovery");

            try (Socket second = new Socket(loopback, endpoint.port())) {
                awaitActiveSocket(endpoint);
            }
            Thread.sleep(100);
            if (socketBinds.get() != 2) throw new AssertionError("rejected connection damaged live discovery");
            if (endpoint.failure().startsWith("listener_failed"))
                throw new AssertionError("discovery listener did not recover: " + endpoint.failure());
        }

        try (P5cReceiverCoordinator endpoint = P5cReceiverCoordinator.openOnInterface(
                binding, playback, loopback, 32)) {
            endpoint.start();
            endpoint.startDiscovery();
            P5cDiscovery.SearchSnapshot initial = endpoint.maintainAutomaticSearch();
            if (!initial.active || !"searching".equals(initial.lastRequestStatus))
                throw new AssertionError("idle automatic search did not start its first bounded discovery round");
            java.lang.reflect.Field discoveryField = P5cReceiverCoordinator.class.getDeclaredField("discovery");
            discoveryField.setAccessible(true);
            P5cDiscovery discovery = (P5cDiscovery) discoveryField.get(endpoint);
            java.lang.reflect.Field deadlineField = P5cDiscovery.class.getDeclaredField("searchDeadline");
            deadlineField.setAccessible(true);
            deadlineField.setLong(discovery, System.nanoTime() / 1_000_000L - 1);
            P5cDiscovery.SearchSnapshot renewed = endpoint.maintainAutomaticSearch();
            if (!renewed.active || !"searching".equals(renewed.lastRequestStatus))
                throw new AssertionError("expired automatic search did not renew its bounded discovery round");
            P5cDiscovery.SearchSnapshot stopped = endpoint.stopSearch();
            if (!"stopped".equals(stopped.lastRequestStatus))
                throw new AssertionError("explicit search cancellation was not retained");
            P5cDiscovery.SearchSnapshot stillStopped = endpoint.maintainAutomaticSearch();
            if (stillStopped.active || !"stopped".equals(stillStopped.lastRequestStatus))
                throw new AssertionError("automatic maintenance restarted an explicitly stopped search");
            java.lang.reflect.Field statusField = P5cDiscovery.class.getDeclaredField("lastRequestStatus");
            statusField.setAccessible(true);
            statusField.set(discovery, "interface-lost");
            P5cDiscovery.SearchSnapshot interfaceLost = endpoint.maintainAutomaticSearch();
            if (interfaceLost.active || !"interface-lost".equals(interfaceLost.lastRequestStatus))
                throw new AssertionError("automatic maintenance restarted search after interface loss");
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
        long now = 1_000_000_000L;
        AtomicLong clock = new AtomicLong(now);
        UdpPlaybackActivity activity = new UdpPlaybackActivity(clock::get);
        activity.resetPlaybackHead(0);
        activity.noteSuccessfulWrite(clock.get());
        clock.set(now + 10);
        if (activity.isActive(clock.get(), 0, true, now, false))
            throw new AssertionError("writes without AudioTrack consumption were reported as streaming");
        clock.set(now + 20);
        if (!activity.isActive(clock.get(), 240, true, now, false))
            throw new AssertionError("recent packet, successful write, and head advance were not reported active");
        if (activity.isActive(clock.get(), 480, true, now, true))
            throw new AssertionError("paused playback was reported streaming");
        clock.set(now + 9_000_000_000L);
        if (activity.isActive(clock.get(), 480, true, now, false))
            throw new AssertionError("stale audio activity remained streaming after the waiting window");

        clock.set(now);
        UdpPlaybackActivity wrapped = new UdpPlaybackActivity(clock::get);
        wrapped.resetPlaybackHead(-2);
        wrapped.noteSuccessfulWrite(clock.get());
        clock.set(now + 20);
        if (!wrapped.isActive(clock.get(), 2, true, now, false))
            throw new AssertionError("unsigned AudioTrack head wrap was treated as a reset");
        clock.set(now);
        UdpPlaybackActivity freshSession = new UdpPlaybackActivity(clock::get);
        freshSession.resetPlaybackHead(0);
        clock.set(now + 20);
        if (freshSession.isActive(clock.get(), 0, true, now, false))
            throw new AssertionError("playback activity leaked across audio sessions");
    }

    private static void verifyQueueAgeBurstAndLayeredBudget() throws Exception {
        PairingProtocol.AudioParameters parameters = new PairingProtocol.AudioParameters(48_000, 2, 0);
        UdpPcmQueue budget = new UdpPcmQueue(parameters);
        budget.configureAudioTrack(3_840);
        if (budget.targetFrames() != 1_920)
            throw new AssertionError("80 ms AudioTrack pre-roll consumed the 40 ms network jitter target");
        budget.reportTrackUnderruns(0);
        budget.reportTrackUnderruns(100);
        if (budget.targetFrames() != 7_200)
            throw new AssertionError("balanced network jitter target did not clamp underrun growth at 150 ms");
        budget.stop();

        byte[] key = repeated((byte) 0x6a, 32);
        long sessionId = 0x1020304050607080L;
        AtomicLong queueClock = new AtomicLong(System.nanoTime());
        UdpPcmQueue queue = new UdpPcmQueue(parameters, "balanced", queueClock::get);
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
            queue.expireStalePackets(receivedAt + 161_000_003L);
            if (queue.size() != 0 || queue.droppedFrames() != 600 || queue.expiredFrames() != 600)
                throw new AssertionError("stalled playback did not expire variable-size old audio by exact frame count");

            byte[] fresh = encodeUdp(key, sessionId, 4, 600, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(240, (short) 0x1234));
            queue.accept(decoder.open(fresh, fresh.length), decoder);
            queueClock.addAndGet(151_000_000L);
            InputStream pcm = queue.asPcmInputStream();
            DataInputStream pcmInput = new DataInputStream(pcm);
            byte[] header = new byte[8];
            pcmInput.readFully(header);
            byte[] first = new byte[960];
            pcmInput.readFully(first);

            byte[] afterGap = encodeUdp(key, sessionId, 5, 1_080, UdpPacketDecoder.AUDIO,
                    pcmPayloadValue(240, (short) 0x1234));
            queue.acceptAt(decoder.open(afterGap, afterGap.length), decoder,
                    queueClock.get() - 151_000_000L);
            byte[] plc = new byte[240 * 4];
            pcmInput.readFully(plc);
            if (littleShort(plc, 0) <= 0 || littleShort(plc, 0) >= 0x1234
                    || littleShort(plc, (240 - 1) * 4) != 0
                    || queue.plcEvents() != 1 || queue.plcRepeatedFrames() != 240
                    || queue.plcSilentFrames() != 0 || queue.plcOversizedGapSkips() != 0)
                throw new AssertionError("finite PLC gap did not repeat and fade the last real packet exactly once");
            byte[] recovered = new byte[240 * 4];
            pcmInput.readFully(recovered);
            if (littleShort(recovered, 0) <= 0 || littleShort(recovered, 0) >= 0x1234
                    || littleShort(recovered, (144 - 1) * 4) != 0x1234)
                throw new AssertionError("fresh real audio did not fade in after finite PLC recovery");
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
        final AtomicBoolean pcm24Decoded = new AtomicBoolean();
        final AtomicInteger pcm24DecodedSessions = new AtomicInteger();
        final AtomicBoolean stopped = new AtomicBoolean();
        final AtomicBoolean failResume;
        final AtomicInteger modeSequenceErrors = new AtomicInteger();
        final AtomicReference<String> queueBudgetFailure = new AtomicReference<>();
        final boolean verifyModeSwitch;
        boolean keepReceiveIntent;
        final boolean verifyAudioWindow;
        final AtomicInteger playbackHead = new AtomicInteger();
        final AtomicBoolean paused = new AtomicBoolean();
        final AtomicBoolean audioWindowAged = new AtomicBoolean();
        volatile UdpPlaybackActivity playbackActivity = new UdpPlaybackActivity();
        volatile long activityClockOffsetNanos;
        volatile UdpPcmQueue queue;
        volatile FloatPcmFrameQueue floatQueue;
        volatile UdpReceiver receiver;
        volatile UdpSessionContext context;
        volatile Thread receiverThread;
        volatile Thread sinkThread;
        Runnable beforeStart = () -> {};
        final AtomicBoolean failStartOnce;
        final AtomicBoolean throwStartOnce;
        final AtomicBoolean throwStopOnce;
        final String backgroundAuthMode;
        volatile boolean audioV2Support;
        volatile int expectedAudioV2Profile = 2;
        volatile int expectedAudioV2Sessions = 1;
        final AtomicInteger backgroundAuthRequests = new AtomicInteger();
        final AtomicInteger backgroundAuthQueries = new AtomicInteger();
        final AtomicInteger backgroundAuthEnds = new AtomicInteger();
        volatile byte[] backgroundAuthRequestId;
        volatile byte[] backgroundAuthBinding;
        volatile long backgroundAuthSession;
        volatile InetAddress backgroundAuthPeer;
        volatile int backgroundAuthStatus = P5cControlProtocol.AUTH_PENDING;
        volatile long backgroundAuthRequestedAt;
        volatile boolean backgroundAuthKeptForRecovery;
        volatile boolean backgroundTrustManagement;
        volatile boolean delayBackgroundTrustCapability;
        final AtomicInteger backgroundTrustQueries = new AtomicInteger();
        final AtomicInteger backgroundAuthCancels = new AtomicInteger();
        final AtomicBoolean backgroundTrustRevoked = new AtomicBoolean();

        FakePlayback(boolean failResume, boolean verifyModeSwitch, boolean verifyAudioWindow, boolean failStartOnce,
                boolean throwStartOnce, boolean throwStopOnce) {
            this(failResume, verifyModeSwitch, verifyAudioWindow, failStartOnce, throwStartOnce, throwStopOnce, "");
        }

        FakePlayback(boolean failResume, boolean verifyModeSwitch, boolean verifyAudioWindow, boolean failStartOnce,
                boolean throwStartOnce, boolean throwStopOnce, String backgroundAuthMode) {
            this.failResume = new AtomicBoolean(failResume);
            this.verifyModeSwitch = verifyModeSwitch;
            this.verifyAudioWindow = verifyAudioWindow;
            this.failStartOnce = new AtomicBoolean(failStartOnce);
            this.throwStartOnce = new AtomicBoolean(throwStartOnce);
            this.throwStopOnce = new AtomicBoolean(throwStopOnce);
            this.backgroundAuthMode = backgroundAuthMode;
        }

        @Override public boolean supportsPlaybackProgress() { return true; }
        @Override public boolean supportsAudioV2() { return audioV2Support; }
        @Override public boolean supportsAudioFormatV2(UdpPacketDecoder.AudioFormatV2 format) {
            return audioV2Support && format != null && expectedAudioV2Profile == 2
                    && format.profile == 2 && format.sampleFormat == 2 && format.sampleRate == 48_000
                    && format.channels == 2 && format.validBits == 24 && format.bytesPerSample == 3
                    && format.frameBytes == 6;
        }
        @Override public boolean supportsBackgroundTrustManagement() {
            if (delayBackgroundTrustCapability) {
                delayBackgroundTrustCapability = false;
                System.out.println("CAPABILITIES_RECEIVED");
                System.out.flush();
                try { Thread.sleep(2_000); }
                catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new RuntimeException(interrupted);
                }
            }
            return backgroundTrustManagement;
        }
        @Override public P5cReceiverCoordinator.BackgroundTrustSnapshot queryBackgroundTrust(byte[] bindingCommitment) {
            backgroundTrustQueries.incrementAndGet();
            if (backgroundTrustRevoked.get())
                return new P5cReceiverCoordinator.BackgroundTrustSnapshot(0, 0, 0);
            return new P5cReceiverCoordinator.BackgroundTrustSnapshot(2, 1, TimeUnit.HOURS.toMillis(3));
        }
        @Override public boolean revokeBackgroundTrust(byte[] bindingCommitment) {
            backgroundTrustRevoked.set(true);
            return true;
        }
        @Override public void cancelBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) {
            backgroundAuthCancels.incrementAndGet();
            P5cReceiverCoordinator.PlaybackEndpoint.super.cancelBackgroundStartAuthorization(
                    requestId, sessionId, peer, bindingCommitment);
        }
        @Override public boolean requiresBackgroundStartAuthorization() { return !backgroundAuthMode.isEmpty(); }
        @Override public int requestBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) {
            if (backgroundAuthRequestId != null && !Arrays.equals(backgroundAuthRequestId, requestId)
                    && !"persistent-stop".equals(backgroundAuthMode)) return P5cControlProtocol.AUTH_CANCELLED;
            if (backgroundAuthRequestId != null && Arrays.equals(backgroundAuthRequestId, requestId))
                return backgroundAuthStatus;
            backgroundAuthRequests.incrementAndGet();
            backgroundAuthRequestId = requestId.clone();
            backgroundAuthRequestedAt = System.nanoTime();
            backgroundAuthSession = sessionId;
            backgroundAuthPeer = peer;
            backgroundAuthBinding = bindingCommitment.clone();
            if ("denied".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_DENIED;
            else if ("expired".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_EXPIRED;
            else if ("persistent-stop".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_UNTIL_REVOKED;
            else if ("once".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_ONCE;
            else if ("eight-hours".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_EIGHT_HOURS;
            else if ("permanent".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_UNTIL_REVOKED;
            else if ("no-grant".equals(backgroundAuthMode)) backgroundAuthStatus = P5cControlProtocol.AUTH_CANCELLED;
            else backgroundAuthStatus = P5cControlProtocol.AUTH_PENDING;
            return backgroundAuthStatus;
        }
        @Override public int backgroundStartAuthorizationStatus(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment) {
            backgroundAuthQueries.incrementAndGet();
            if (!Arrays.equals(backgroundAuthRequestId, requestId) || backgroundAuthSession != sessionId
                    || !backgroundAuthPeer.equals(peer) || !Arrays.equals(backgroundAuthBinding, bindingCommitment))
                return P5cControlProtocol.AUTH_EXPIRED;
            if ("pending-slow-status".equals(backgroundAuthMode)) {
                try { Thread.sleep(5_000); }
                catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new RuntimeException(interrupted);
                }
            }
            if (P5cControlProtocol.AUTH_PENDING == backgroundAuthStatus) {
                if ("pending-allow".equals(backgroundAuthMode)
                        || "pending-allow-after-6s".equals(backgroundAuthMode)
                                && System.nanoTime() - backgroundAuthRequestedAt >= TimeUnit.SECONDS.toNanos(6))
                    backgroundAuthStatus = P5cControlProtocol.AUTH_ONCE;
            }
            return backgroundAuthStatus;
        }
        @Override public void endBackgroundStartAuthorization(byte[] requestId, long sessionId, InetAddress peer,
                byte[] bindingCommitment, boolean keepForRecovery) {
            if (Arrays.equals(backgroundAuthRequestId, requestId) && backgroundAuthSession == sessionId
                    && backgroundAuthPeer.equals(peer) && Arrays.equals(backgroundAuthBinding, bindingCommitment)) {
                backgroundAuthEnds.incrementAndGet();
                backgroundAuthKeptForRecovery = keepForRecovery;
                if (!keepForRecovery && backgroundAuthStatus == P5cControlProtocol.AUTH_PENDING)
                    backgroundAuthStatus = P5cControlProtocol.AUTH_CANCELLED;
            }
        }

        @Override public boolean keepStandbyAfterPlaybackStop() {
            return keepReceiveIntent || "persistent-stop".equals(backgroundAuthMode);
        }

        void verifyBackgroundAuthorization() throws IOException {
            if ("persistent-stop".equals(backgroundAuthMode)) {
                if (backgroundAuthRequests.get() != 2 || backgroundAuthEnds.get() != 2 || backgroundAuthKeptForRecovery)
                    throw new IOException("persistent authorization must permit two STOP-and-reconnect sessions");
                return;
            }
            if (backgroundAuthRequests.get() != 1 || backgroundAuthEnds.get() != 1 || backgroundAuthKeptForRecovery)
                throw new IOException("background request must be single-owner and end without a recovery grant");
        }

        @Override public boolean start(UdpSessionContext context, P5cReceiverCoordinator.StartPermission permission) throws IOException {
            if (backgroundTrustRevoked.get()) return false;
            beforeStart.run();
            if (!permission.allowed()) return false;
            this.context = context;
            if (throwStartOnce.compareAndSet(true, false)) throw new IOException("injected START throw");
            if (failStartOnce.compareAndSet(true, false)) return false;
            int startNumber = starts.incrementAndGet();
            if (context.formatV2 != null) {
                if (startNumber > expectedAudioV2Sessions)
                    throw new IOException("more V2 formal sessions than this fixture allows");
                if (!audioV2Support || context.parameters != null
                        || context.formatV2.profile != expectedAudioV2Profile || context.formatV2.profile != 2
                        || context.formatV2.sampleRate != 48_000 || context.formatV2.validBits != 24
                        || context.formatV2.channels != 2)
                    throw new IOException("unexpected authenticated v2 capture format");
                stopped.set(false);
                paused.set(false);
                pcm24Decoded.set(false);
                floatQueue = new FloatPcmFrameQueue(context.formatV2.sampleRate, true);
                receiver = new UdpReceiver(context, floatQueue);
                receiverThread = new Thread(receiver, "P5-C test PCM24 UDP receiver");
                receiverThread.start();
                return true;
            }
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
            awaitModeSwitchPayload();
            FloatPcmFrameQueue currentFloatQueue = floatQueue;
            if (currentFloatQueue != null && !pcm24Decoded.get()) verifyPcm24Decoded(currentFloatQueue);
            pauses.incrementAndGet(); paused.set(true);
            if (queue != null) queue.setPaused(true);
            if (currentFloatQueue != null) currentFloatQueue.setPaused(true);
        }
        @Override public void resume() throws IOException {
            resumes.incrementAndGet();
            if (failResume.compareAndSet(true, false)) throw new IOException("P5-D test RESUME failure");
            paused.set(false);
            if (queue != null) queue.setPaused(false);
            if (floatQueue != null) floatQueue.setPaused(false);
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
            if (throwStopOnce.compareAndSet(true, false)) throw new IllegalStateException("injected STOP throw");
            awaitModeSwitchPayload();
            stops.incrementAndGet();
            if (!stopped.compareAndSet(false, true)) return;
            UdpReceiver currentReceiver = receiver;
            if (currentReceiver != null) currentReceiver.close();
            UdpSessionContext currentContext = context;
            if (currentContext != null) currentContext.destroy();
            UdpPcmQueue currentQueue = queue;
            if (currentQueue != null) currentQueue.stop();
            FloatPcmFrameQueue currentFloatQueue = floatQueue;
            if (currentFloatQueue != null) currentFloatQueue.stop();
            join(receiverThread);
            join(sinkThread);
            udpClosed.set(currentReceiver == null || currentReceiver.isClosed());
        }

        private void awaitModeSwitchPayload() {
            if (!verifyModeSwitch || queue == null || !queue.hasAuthenticatedAudio() || stopped.get()) return;
            // The mode-switch fixture sends 960 frames per session. A streaming
            // observation only proves some consumption; wait before flushing it.
            int expectedBytes = starts.get() * 960 * 4;
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(1);
            while (pcmBytes.get() < expectedBytes) {
                if (System.nanoTime() >= deadline)
                    throw new AssertionError("mode-switch synthetic sink did not drain " + expectedBytes + " bytes");
                try { Thread.sleep(1); }
                catch (InterruptedException error) {
                    Thread.currentThread().interrupt();
                    throw new AssertionError("mode-switch sink wait interrupted", error);
                }
            }
        }

        private void verifyPcm24Decoded(FloatPcmFrameQueue currentQueue) {
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
            while (currentQueue.queuedFrames() < 2 && System.nanoTime() < deadline) {
                try { Thread.sleep(1); }
                catch (InterruptedException error) {
                    Thread.currentThread().interrupt();
                    return;
                }
            }
            float[] decoded = new float[4];
            if (currentQueue.poll(decoded, 0, 2) != 2) return;
            boolean matches = Float.floatToRawIntBits(decoded[0]) == Float.floatToRawIntBits(1.0f / 8_388_608.0f)
                    && Float.floatToRawIntBits(decoded[1]) == Float.floatToRawIntBits(-1.0f / 8_388_608.0f)
                    && Float.floatToRawIntBits(decoded[2]) == Float.floatToRawIntBits(32_769.0f / 8_388_608.0f)
                    && Float.floatToRawIntBits(decoded[3]) == Float.floatToRawIntBits(-32_769.0f / 8_388_608.0f);
            if (matches && pcm24Decoded.compareAndSet(false, true)) pcm24DecodedSessions.incrementAndGet();
        }

        boolean allPcm24SessionsDecoded() {
            return pcm24DecodedSessions.get() == expectedAudioV2Sessions;
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
