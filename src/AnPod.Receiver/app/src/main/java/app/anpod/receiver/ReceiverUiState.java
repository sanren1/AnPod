package app.anpod.receiver;

import java.io.IOException;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.List;

/** Read-only views of ReceiverService state used by the UI binder. */
final class ReceiverUiState {
    private final ReceiverService service;
    private final AodMetricsSampler aodSampler = new AodMetricsSampler();
    private long aodSessionId = -1;

    ReceiverUiState(ReceiverService service) {
        this.service = service;
    }

    AodTelemetrySnapshot aodSnapshot() {
        long now = System.nanoTime();
        ReceiverService.AudioSession session = ReceiverService.currentAudio;
        if (session == null) {
            aodSessionId = -1;
            aodSampler.reset(now);
            return new AodTelemetrySnapshot(-1, AodTelemetrySnapshot.DisplayStatus.DISCONNECTED,
                    AodTelemetrySnapshot.Transport.UNKNOWN, null, null, null, false, now);
        }
        if (session.id != aodSessionId) {
            aodSessionId = session.id;
            aodSampler.reset(now);
        }
        boolean udp = session.udpQueue != null;
        long totalBytes = udp ? session.udpQueue.audioPayloadBytes() : session.inputPcmBytes;
        aodSampler.recordCumulativeBytes(totalBytes, now);
        PlaybackMetrics metrics = ReceiverService.playbackMetricsSnapshot();
        Long bitsPerSecond = aodSampler.bitsPerSecond(now);
        Float bufferMs = AodMetricsSampler.bufferMs(udp ? metrics.queuedFrames : 0,
                metrics.pendingFrames, metrics.sampleRate);
        Float lossPercent = null;
        if (udp) {
            aodSampler.updateFrameTotals(session.udpQueue.acceptedAudioFrames(),
                    session.udpQueue.concealedFrames());
            lossPercent = aodSampler.lossPercent();
        }
        boolean receivedRecently = aodSampler.receivedAudioRecently(now, 2_000_000_000L);
        AodTelemetrySnapshot.DisplayStatus status = receivedRecently
                ? AodTelemetrySnapshot.DisplayStatus.RECEIVING
                : AodTelemetrySnapshot.DisplayStatus.CONNECTED;
        if (udp && "reconnecting".equals(ReceiverService.networkAudioState))
            status = AodTelemetrySnapshot.DisplayStatus.RECONNECTING;
        AodTelemetrySnapshot.Transport transport = udp
                ? AodTelemetrySnapshot.Transport.WIFI_UDP
                : AodTelemetrySnapshot.Transport.USB_ADB;
        return new AodTelemetrySnapshot(session.id, status, transport, bitsPerSecond, bufferMs,
                lossPercent, receivedRecently, now);
    }

    UiSnapshot snapshot() {
        String binding = "empty";
        String fingerprint = "";
        String listenerState = "disabled", grantKind = "none", pendingRequestId = "", authorizationError = "";
        String grantPersistence = "none";
        long grantRemainingMs = 0, pendingDeadlineElapsedMs = 0;
        try {
            BindingStore.Status status = new BindingStore(service.getFilesDir()).status();
            binding = status.state;
            fingerprint = status.commitment;
            if (!fingerprint.isEmpty()) {
                synchronized (BindingStore.storageLock()) {
                    byte[] commitment = ReceiverService.unhex(fingerprint);
                    try { service.backgroundAuthorization.inspectBinding(commitment); }
                    finally { java.util.Arrays.fill(commitment, (byte) 0); }
                    listenerState = service.backgroundAuthorization.listenerState();
                    if (service.backgroundStandbyEnabled && service.networkCoordinator == null) listenerState = "starting";
                    try {
                        BackgroundStartAuthorization.GrantSnapshot grant = service.backgroundAuthorization.grantSnapshot();
                        grantKind = grant.kind;
                        grantPersistence = grant.persistenceCode == 2 ? "binding"
                                : grant.persistenceCode == 1 ? "process" : "none";
                        grantRemainingMs = grant.remainingMs;
                    } catch (IOException grantError) {
                        grantKind = "unavailable";
                        grantPersistence = "unavailable";
                        authorizationError = service.describeFailure(grantError);
                    }
                    pendingRequestId = service.backgroundAuthorization.pendingRequestId();
                    pendingDeadlineElapsedMs = service.backgroundAuthorization.pendingDeadlineElapsedMs();
                    if (authorizationError.isEmpty()) authorizationError = service.backgroundAuthorization.error();
                }
            }
        } catch (Exception error) {
            binding = "unavailable";
            grantKind = "unavailable";
            grantPersistence = "unavailable";
            authorizationError = service.describeFailure(error);
        }
        String method = service.networkCoordinator != null || !ReceiverService.networkAudioAddress.isEmpty() ? "wireless-udp"
                : ReceiverService.currentAudio != null ? "usb-or-local" : "unknown";
        String state = ReceiverUiOperation.displayState(ReceiverService.networkAudioState, ReceiverService.currentAudio != null, ReceiverService.audioActive);
        String mode = ReceiverService.currentAudio == null ? "" : ReceiverService.actualMode;
        Boolean playbackAdvancing = ReceiverService.playbackMetricsSnapshot().advancing;
        String operationError = service.uiOperation.error();
        if (operationError.isEmpty()) operationError = ReceiverService.networkAudioFailure;
        boolean canRevokeBackgroundTrust = service.canRevokeBackgroundTrust(binding, fingerprint, grantKind);
        return new UiSnapshot(state, binding, fingerprint, method, mode, BuildConfig.VERSION_NAME,
                BuildConfig.VERSION_CODE, service.uiOperation.ownerId(), service.uiOperation.phase(), operationError,
                service.currentOperationGeneration, ReceiverService.networkAudioAddress, ReceiverService.networkAudioPort, service.backgroundStandbyEnabled,
                canRevokeBackgroundTrust, playbackAdvancing,
                listenerState, grantKind, grantPersistence, grantRemainingMs, pendingRequestId,
                pendingDeadlineElapsedMs, authorizationError,
                service.networkCoordinator == null ? new P5cDiscovery.SearchSnapshot(false, Collections.emptyList(), "idle")
                        : service.networkCoordinator.searchSnapshot());
    }

    List<UiInterface> interfaces() {
        ArrayList<UiInterface> result = new ArrayList<>();
        try {
            Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
            while (interfaces != null && interfaces.hasMoreElements()) {
                NetworkInterface network = interfaces.nextElement();
                if (!network.isUp() || network.isLoopback()) continue;
                for (java.net.InterfaceAddress item : network.getInterfaceAddresses()) {
                    InetAddress address = item.getAddress();
                    if (address instanceof Inet4Address && address.isSiteLocalAddress())
                        result.add(new UiInterface(network.getDisplayName(), address.getHostAddress(), item.getNetworkPrefixLength()));
                }
            }
        } catch (SocketException ignored) { }
        Collections.sort(result, new Comparator<UiInterface>() {
            @Override
            public int compare(UiInterface left, UiInterface right) {
                int byName = left.name.compareTo(right.name);
                return byName != 0 ? byName : left.address.compareTo(right.address);
            }
        });
        return result;
    }

    P5cReceiverCoordinator coordinator(String expectedOwnerId) throws IOException {
        P5cReceiverCoordinator coordinator = service.networkCoordinator;
        if (expectedOwnerId == null || expectedOwnerId.isEmpty()
                || !expectedOwnerId.equals(service.networkCoordinatorOwnerId)
                || !service.isCurrentOperation(expectedOwnerId, service.currentOperationGeneration))
            throw new IOException("接收会话已结束，请刷新状态后重试。");
        if (coordinator == null) throw new IOException("请先启动 Receiver 无线 UDP 接收。");
        return coordinator;
    }
}
