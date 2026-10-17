package app.anpod.receiver;

import android.os.Binder;

import java.io.IOException;
import java.util.List;

    public final class ReceiverUiBinder extends Binder {
    private final ReceiverService service;

    ReceiverUiBinder(ReceiverService service) {
        this.service = service;
    }

        public UiSnapshot snapshot() { return service.uiState.snapshot(); }
        public AodTelemetrySnapshot aodSnapshot() { return service.uiState.aodSnapshot(); }
        public List<UiInterface> interfaces() { return service.uiState.interfaces(); }
        public String start(String address, int prefixLength, String requestId) { return service.startFromUi(address, prefixLength, requestId); }
        public String startReceiving(String address, int prefixLength, String requestId) {
            return service.startFromUi(address, prefixLength, requestId, false, true);
        }
        public String startBackgroundStandby(String address, int prefixLength, String requestId) {
            return service.startFromUi(address, prefixLength, requestId, true, true);
        }
        public boolean stop(String ownerId) { return service.stopFromUi(ownerId); }
        public boolean stop(String ownerId, long generation) { return service.stopFromUi(ownerId, generation); }
        public boolean trustBindingUntilRevoked(String ownerId, long generation) {
            return service.trustBindingUntilRevokedFromUi(ownerId, generation);
        }
        public boolean revokeBackgroundTrust(String expectedOwnerId, long expectedGeneration,
                String expectedBindingFingerprint) {
            return service.revokeBackgroundTrustFromUi(expectedOwnerId, expectedGeneration,
                    expectedBindingFingerprint);
        }
        public P5cDiscovery.SearchSnapshot startSearch(String expectedOwnerId) throws IOException {
            return service.uiState.coordinator(expectedOwnerId).startSearch();
        }
        public P5cDiscovery.SearchSnapshot startAutomaticSearch(String expectedOwnerId) throws IOException {
            return service.uiState.coordinator(expectedOwnerId).startAutomaticSearch();
        }
        public P5cDiscovery.SearchSnapshot stopSearch(String expectedOwnerId) throws IOException {
            return service.uiState.coordinator(expectedOwnerId).stopSearch();
        }
        public String requestConnection(String candidateId, String expectedOwnerId) throws IOException {
            return service.uiState.coordinator(expectedOwnerId).requestConnection(candidateId);
        }
    }
