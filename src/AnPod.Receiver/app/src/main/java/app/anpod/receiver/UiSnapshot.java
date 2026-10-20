package app.anpod.receiver;

import java.util.List;

    public final class UiSnapshot {
        public final String serviceState, bindingState, connectionMethod, playbackMode, versionName, networkAddress;
        public final String operationId, operationState, error;
        public final String bindingFingerprint, backgroundListenerState, backgroundGrantKind, backgroundGrantPersistence;
        public final String pendingAuthorizationRequestId, backgroundAuthorizationError;
        public final String requestedOutputEngine, actualOutputEngine, outputFallbackReason;
        public final int oboeXrunCount;
        public final int versionCode, networkPort;
        public final long operationGeneration, backgroundGrantRemainingMs, pendingAuthorizationDeadlineElapsedMs;
        public final Boolean playbackAdvancing;
        public final boolean backgroundStandbyEnabled, canRevokeBackgroundTrust;
        public final boolean searchActive;
        public final List<P5cDiscovery.SearchCandidate> searchCandidates;
        public final String connectionRequestStatus;
        UiSnapshot(String serviceState, String bindingState, String bindingFingerprint, String connectionMethod, String playbackMode,
                String versionName, int versionCode, String operationId, String operationState, String error,
                long operationGeneration, String networkAddress, int networkPort, boolean backgroundStandbyEnabled,
                boolean canRevokeBackgroundTrust, Boolean playbackAdvancing,
                String backgroundListenerState, String backgroundGrantKind, String backgroundGrantPersistence,
                long backgroundGrantRemainingMs,
                String pendingAuthorizationRequestId, long pendingAuthorizationDeadlineElapsedMs,
                String backgroundAuthorizationError, P5cDiscovery.SearchSnapshot search,
                String requestedOutputEngine, String actualOutputEngine, String outputFallbackReason,
                int oboeXrunCount) {
            this.serviceState = serviceState; this.bindingState = bindingState;
            this.bindingFingerprint = bindingFingerprint;
            this.connectionMethod = connectionMethod; this.playbackMode = playbackMode;
            this.versionName = versionName; this.versionCode = versionCode;
            this.operationId = operationId; this.operationState = operationState; this.error = error;
            this.operationGeneration = operationGeneration;
            this.networkAddress = networkAddress; this.networkPort = networkPort;
            this.playbackAdvancing = playbackAdvancing;
            this.backgroundStandbyEnabled = backgroundStandbyEnabled;
            this.canRevokeBackgroundTrust = canRevokeBackgroundTrust;
            this.backgroundListenerState = backgroundListenerState; this.backgroundGrantKind = backgroundGrantKind;
            this.backgroundGrantPersistence = backgroundGrantPersistence;
            this.backgroundGrantRemainingMs = backgroundGrantRemainingMs;
            this.pendingAuthorizationRequestId = pendingAuthorizationRequestId;
            this.pendingAuthorizationDeadlineElapsedMs = pendingAuthorizationDeadlineElapsedMs;
            this.backgroundAuthorizationError = backgroundAuthorizationError;
            this.requestedOutputEngine = requestedOutputEngine;
            this.actualOutputEngine = actualOutputEngine;
            this.outputFallbackReason = outputFallbackReason;
            this.oboeXrunCount = oboeXrunCount;
            this.searchActive = search.active; this.searchCandidates = search.candidates;
            this.connectionRequestStatus = search.lastRequestStatus;
        }
    }
