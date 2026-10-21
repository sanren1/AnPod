package app.anpod.receiver;

import java.io.IOException;
import java.net.InetAddress;

/** In-memory authorization state; only until-revoked grants are delegated to persistent storage. */
final class BackgroundStartAuthorization {
    interface Clock { long elapsedRealtime(); }
    interface GrantStore {
        boolean hasUntilRevoked(String bindingCommitment) throws IOException;
        void saveUntilRevoked(String bindingCommitment) throws IOException;
        void clearUntilRevoked(String bindingCommitment) throws IOException;
    }
    interface Listener { void changed(Token pending); }

    static final int PENDING = P5cControlProtocol.AUTH_PENDING;
    static final int ONCE = P5cControlProtocol.AUTH_ONCE;
    static final int EIGHT_HOURS = P5cControlProtocol.AUTH_EIGHT_HOURS;
    static final int UNTIL_REVOKED = P5cControlProtocol.AUTH_UNTIL_REVOKED;
    static final int DENIED = P5cControlProtocol.AUTH_DENIED;
    static final int EXPIRED = P5cControlProtocol.AUTH_EXPIRED;
    static final int CANCELLED = P5cControlProtocol.AUTH_CANCELLED;
    static final long REQUEST_WINDOW_MS = 30_000L;
    static final long EIGHT_HOURS_MS = TimeUnitHours.EIGHT_HOURS_MS;
    static final long RECOVERY_WINDOW_MS = 30_000L;

    private final Clock clock;
    private final GrantStore store;
    private final Listener listener;
    private String ownerId = "";
    private long ownerGeneration;
    private String bindingCommitment = "";
    private boolean standbyEnabled;
    private Request current;
    private Episode episode;
    private OnceGrant once;
    private String eightHourBinding = "";
    private long eightHourExpiresAt;
    private String error = "";

    BackgroundStartAuthorization(Clock clock, GrantStore store, Listener listener) {
        this.clock = clock;
        this.store = store;
        this.listener = listener;
    }

    synchronized void enable(String ownerId, long generation, byte[] bindingCommitment) {
        this.ownerId = ownerId;
        ownerGeneration = generation;
        this.bindingCommitment = hex(bindingCommitment);
        standbyEnabled = true;
        current = null;
        episode = null;
        once = null;
        error = "";
        changed();
    }

    synchronized void inspectBinding(byte[] bindingCommitment) {
        this.bindingCommitment = hex(bindingCommitment);
    }

    synchronized void disable(String ownerId, long generation) {
        if (!owns(ownerId, generation)) return;
        standbyEnabled = false;
        current = null;
        episode = null;
        once = null;
        error = "";
        changed();
    }

    synchronized int request(String ownerId, long generation, byte[] requestId, long sessionId,
            InetAddress peer, byte[] commitment) throws IOException {
        if (!matchesOwner(ownerId, generation, commitment)) return EXPIRED;
        long now = clock.elapsedRealtime();
        expireOnceGrant(now);
        Token token = new Token(ownerId, generation, sessionId, hex(requestId),
                hex(peer.getAddress()), hex(commitment), now + REQUEST_WINDOW_MS);
        if (current != null && current.token.sameSession(token)) return current.status;

        if (episode != null && episode.matches(token)) {
            token = new Token(ownerId, generation, sessionId, token.requestId, token.peer,
                    token.bindingCommitment, episode.deadlineElapsedMs);
            if (episode.status == PENDING && now >= episode.deadlineElapsedMs) episode.status = EXPIRED;
            if (episode.status == ONCE && !matchesOnce(requestId, peer, commitment, ownerId, generation, now))
                episode.status = EXPIRED;
            try {
                if (episode.status == EIGHT_HOURS && trustedStatus(commitment, now) != EIGHT_HOURS)
                    episode.status = EXPIRED;
                if (episode.status == UNTIL_REVOKED && !store.hasUntilRevoked(hex(commitment)))
                    episode.status = EXPIRED;
            } catch (IOException failure) {
                error = failure.getClass().getSimpleName();
                changed();
                throw failure;
            }
            current = new Request(token, episode.status);
            changed();
            return episode.status;
        }
        once = null;

        int trusted;
        try { trusted = trustedStatus(commitment, now); }
        catch (IOException failure) {
            error = failure.getClass().getSimpleName();
            changed();
            throw failure;
        }
        if (trusted != EXPIRED) return install(token, trusted);
        if (matchesOnce(requestId, peer, commitment, ownerId, generation, now))
            return install(token, ONCE);

        current = new Request(token, PENDING);
        episode = new Episode(token, PENDING);
        changed();
        return PENDING;
    }

    synchronized int status(String ownerId, long generation, byte[] requestId, long sessionId,
            InetAddress peer, byte[] commitment) throws IOException {
        if (!matchesOwner(ownerId, generation, commitment) || current == null
                || !current.token.matches(ownerId, generation, requestId, sessionId, peer, commitment)) return EXPIRED;
        long now = clock.elapsedRealtime();
        expireEightHourGrantStatus(now);
        if (current.status == EIGHT_HOURS && !hex(commitment).equals(eightHourBinding)) {
            current.status = EXPIRED;
            if (episode != null && episode.matches(current.token)) episode.status = EXPIRED;
            error = "Background-start trust expired.";
            changed();
        }
        if (current.status == UNTIL_REVOKED) {
            try {
                if (!store.hasUntilRevoked(hex(commitment))) {
                    current.status = EXPIRED;
                    error = "Background-start trust was revoked.";
                    changed();
                }
            } catch (IOException failure) {
                error = failure.getClass().getSimpleName();
                changed();
                throw failure;
            }
        }
        if (current.status == ONCE && once != null && once.matches(current.token)
                && once.recoveryDeadlineElapsedMs != 0 && now >= once.recoveryDeadlineElapsedMs) {
            current.status = EXPIRED;
            if (episode != null && episode.matches(current.token)) episode.status = EXPIRED;
            once = null;
            error = "Background-start recovery expired.";
            changed();
        }
        if (current.status == PENDING && now >= current.token.deadlineElapsedMs) {
            current.status = EXPIRED;
            if (episode != null && episode.matches(current.token)) episode.status = EXPIRED;
            error = "Background-start authorization expired.";
            changed();
        }
        return current.status;
    }

    synchronized boolean decide(Token token, int decision) throws IOException {
        if (current == null || !current.token.equals(token) || current.status != PENDING
                || clock.elapsedRealtime() >= token.deadlineElapsedMs || !owns(token.ownerId, token.ownerGeneration))
            return false;
        switch (decision) {
            case ONCE:
                once = new OnceGrant(token);
                current.status = ONCE;
                break;
            case EIGHT_HOURS:
                eightHourBinding = token.bindingCommitment;
                eightHourExpiresAt = clock.elapsedRealtime() + EIGHT_HOURS_MS;
                current.status = EIGHT_HOURS;
                break;
            case DENIED:
                current.status = DENIED;
                break;
            default:
                throw new IllegalArgumentException("Unsupported notification decision");
        }
        if (episode != null && episode.matches(token)) episode.status = current.status;
        error = "";
        changed();
        return true;
    }

    synchronized boolean cancel(Token token) {
        if (current == null || !current.token.equals(token) || current.status != PENDING) return false;
        current.status = CANCELLED;
        if (episode != null && episode.matches(token)) episode.status = CANCELLED;
        once = null;
        changed();
        return true;
    }

    synchronized void end(String ownerId, long generation, byte[] requestId, long sessionId,
            InetAddress peer, byte[] commitment, boolean playbackHadStarted, boolean keepForRecovery) {
        if (current == null || !current.token.matches(ownerId, generation, requestId, sessionId, peer, commitment)) return;
        if (keepForRecovery && once != null && once.matches(current.token)) {
            once.playbackActive = false;
            if (once.recoveryDeadlineElapsedMs == 0)
                once.recoveryDeadlineElapsedMs = clock.elapsedRealtime() + RECOVERY_WINDOW_MS;
        } else if (playbackHadStarted && once != null && once.matches(current.token)) {
            once = null;
            if (episode != null && episode.matches(current.token)) episode.status = CANCELLED;
        } else if (once != null && once.matches(current.token)) {
            once.playbackActive = false;
        }
        current = null;
        changed();
    }

    synchronized void playbackStarted(String ownerId, long generation, byte[] requestId, long sessionId,
            InetAddress peer, byte[] commitment) {
        if (current != null && current.token.matches(ownerId, generation, requestId, sessionId, peer, commitment)) {
            if (current.status == ONCE && once != null && once.matches(current.token)) {
                once.playbackActive = true;
                once.recoveryDeadlineElapsedMs = 0;
            }
            changed();
        }
    }

    synchronized long onceRecoveryDeadlineElapsedMs(String ownerId, long generation) {
        return owns(ownerId, generation) && once != null ? once.recoveryDeadlineElapsedMs : 0;
    }

    synchronized void revoke(String ownerId, long generation) throws IOException {
        if (!owns(ownerId, generation)) return;
        store.clearUntilRevoked(bindingCommitment);
        if (store.hasUntilRevoked(bindingCommitment)) throw new IOException("Background trust remains after revoke");
        if (bindingCommitment.equals(eightHourBinding)) {
            eightHourBinding = "";
            eightHourExpiresAt = 0;
        }
        current = null;
        episode = null;
        once = null;
        standbyEnabled = false;
        error = "";
        changed();
    }

    synchronized void revokeBindingGrant(String expectedBindingCommitment) throws IOException {
        if (standbyEnabled || expectedBindingCommitment == null
                || !bindingCommitment.equals(expectedBindingCommitment))
            throw new IOException("Receiver binding changed");
        store.clearUntilRevoked(expectedBindingCommitment);
        if (store.hasUntilRevoked(expectedBindingCommitment))
            throw new IOException("Background trust remains after revoke");
        if (expectedBindingCommitment.equals(eightHourBinding)) {
            eightHourBinding = "";
            eightHourExpiresAt = 0;
        }
        current = null;
        episode = null;
        once = null;
        error = "";
        changed();
    }

    synchronized void trustUntilRevoked(String ownerId, long generation) throws IOException {
        if (!owns(ownerId, generation) || bindingCommitment.isEmpty()) throw new IOException("Receiver owner changed");
        store.saveUntilRevoked(bindingCommitment);
        if (current != null) {
            current.status = UNTIL_REVOKED;
            if (episode != null) episode.status = UNTIL_REVOKED;
        }
        error = "";
        changed();
    }

    synchronized void resetBinding(String commitment) throws IOException {
        if (commitment == null) { store.clearUntilRevoked(null); return; }
        store.clearUntilRevoked(commitment);
        if (commitment.equals(bindingCommitment)) {
            standbyEnabled = false;
            current = null;
            episode = null;
            once = null;
            bindingCommitment = "";
            error = "";
        }
        if (commitment.equals(eightHourBinding)) {
            eightHourBinding = "";
            eightHourExpiresAt = 0;
        }
        changed();
    }

    synchronized boolean enabled() { return standbyEnabled; }
    synchronized String bindingCommitment() { return standbyEnabled ? bindingCommitment : ""; }
    synchronized Token pendingToken() { return current != null && current.status == PENDING ? current.token : null; }
    synchronized String pendingRequestId() { Token token = pendingToken(); return token == null ? "" : token.requestId; }
    synchronized long pendingDeadlineElapsedMs() { Token token = pendingToken(); return token == null ? 0 : token.deadlineElapsedMs; }
    synchronized String error() { return error; }

    synchronized void reportError(String ownerId, long generation, String message) {
        if (!owns(ownerId, generation)) return;
        error = message == null ? "Background authorization failed." : message;
        changed();
    }

    synchronized void reportErrorForBinding(String expectedBindingCommitment, String message) {
        if (expectedBindingCommitment == null || !bindingCommitment.equals(expectedBindingCommitment)) return;
        error = message == null ? "Background authorization failed." : message;
        changed();
    }

    synchronized long eightHourDeadlineElapsedMs(String ownerId, long generation) {
        return owns(ownerId, generation) && eightHourBinding.equals(bindingCommitment) ? eightHourExpiresAt : 0;
    }

    synchronized boolean expireEightHourGrant(String ownerId, long generation, long deadline) throws IOException {
        if (!owns(ownerId, generation) || deadline == 0 || deadline != eightHourExpiresAt
                || clock.elapsedRealtime() < deadline) return false;
        String expiredBinding = eightHourBinding;
        error = "Background-start trust expired.";
        boolean stillTrusted = store.hasUntilRevoked(expiredBinding);
        eightHourBinding = "";
        eightHourExpiresAt = 0;
        if (current != null && current.status == EIGHT_HOURS) {
            current.status = stillTrusted ? UNTIL_REVOKED : EXPIRED;
            if (episode != null && episode.matches(current.token)) episode.status = current.status;
        }
        changed();
        return !stillTrusted;
    }

    synchronized GrantSnapshot grantSnapshot() throws IOException {
        long now = clock.elapsedRealtime();
        expireOnceGrant(now);
        try {
            expireEightHourGrantStatus(now);
            if (!bindingCommitment.isEmpty() && store.hasUntilRevoked(bindingCommitment))
                return new GrantSnapshot("untilRevoked", 3, 2, -1);
        } catch (IOException failure) {
            error = failure.getMessage() == null ? failure.getClass().getSimpleName() : failure.getMessage();
            changed();
            throw failure;
        }
        if (!eightHourBinding.isEmpty() && eightHourBinding.equals(bindingCommitment)
                && eightHourExpiresAt > now)
            return new GrantSnapshot("eightHours", 2, 1, Math.max(0, eightHourExpiresAt - now));
        if (once != null && once.bindingCommitment.equals(bindingCommitment)) {
            long remaining = once.recoveryDeadlineElapsedMs == 0 ? -1
                    : Math.max(0, once.recoveryDeadlineElapsedMs - now);
            return new GrantSnapshot("once", 1, 1, remaining);
        }
        return new GrantSnapshot("none", 0, 0, 0);
    }

    synchronized String grantKind() throws IOException { return grantSnapshot().kind; }

    synchronized long grantRemainingMs() throws IOException { return grantSnapshot().remainingMs; }

    synchronized String listenerState() {
        if (!standbyEnabled) return "disabled";
        return current != null && current.status == PENDING ? "pending" : "listening";
    }

    private int trustedStatus(byte[] commitment, long now) throws IOException {
        String value = hex(commitment);
        if (value.equals(eightHourBinding) && eightHourExpiresAt > now) return EIGHT_HOURS;
        if (store.hasUntilRevoked(value)) return UNTIL_REVOKED;
        return EXPIRED;
    }

    private int install(Token token, int status) {
        current = new Request(token, status);
        episode = new Episode(token, status);
        changed();
        return status;
    }

    private boolean matchesOnce(byte[] requestId, InetAddress peer, byte[] commitment,
            String ownerId, long generation, long now) {
        return once != null && once.requestId.equals(hex(requestId)) && once.peer.equals(hex(peer.getAddress()))
                && once.bindingCommitment.equals(hex(commitment)) && once.ownerId.equals(ownerId)
                && once.ownerGeneration == generation && (once.playbackActive
                        || (once.recoveryDeadlineElapsedMs == 0
                                ? once.requestDeadlineElapsedMs > now : once.recoveryDeadlineElapsedMs > now));
    }

    private boolean matchesOwner(String ownerId, long generation, byte[] commitment) {
        return standbyEnabled && owns(ownerId, generation) && bindingCommitment.equals(hex(commitment));
    }

    private boolean owns(String ownerId, long generation) {
        return standbyEnabled && ownerId != null && ownerId.equals(this.ownerId) && generation == ownerGeneration;
    }

    private void expireEightHourGrantStatus(long now) throws IOException {
        if (!eightHourBinding.isEmpty() && now >= eightHourExpiresAt
                && current != null && current.status == EIGHT_HOURS) {
            boolean stillTrusted = store.hasUntilRevoked(eightHourBinding);
            current.status = stillTrusted ? UNTIL_REVOKED : EXPIRED;
            if (episode != null && episode.matches(current.token)) episode.status = current.status;
            error = "Background-start trust expired.";
            changed();
        }
    }

    private void expireOnceGrant(long now) {
        if (once != null && once.recoveryDeadlineElapsedMs != 0 && now >= once.recoveryDeadlineElapsedMs) {
            if (current != null && current.status == ONCE && once.matches(current.token)) {
                current.status = EXPIRED;
                if (episode != null && episode.matches(current.token)) episode.status = EXPIRED;
            }
            once = null;
            error = "Background-start recovery expired.";
            changed();
        }
    }

    private void changed() { if (listener != null) listener.changed(pendingToken()); }

    static String hex(byte[] bytes) {
        char[] digits = "0123456789abcdef".toCharArray();
        char[] result = new char[bytes.length * 2];
        for (int i = 0; i < bytes.length; i++) {
            result[i * 2] = digits[(bytes[i] >>> 4) & 15];
            result[i * 2 + 1] = digits[bytes[i] & 15];
        }
        return new String(result);
    }

    static final class GrantSnapshot {
        final String kind;
        final int kindCode, persistenceCode;
        final long remainingMs;
        GrantSnapshot(String kind, int kindCode, int persistenceCode, long remainingMs) {
            this.kind = kind; this.kindCode = kindCode; this.persistenceCode = persistenceCode;
            this.remainingMs = remainingMs;
        }
    }

    static final class Token {
        final String ownerId, requestId, peer, bindingCommitment;
        final long ownerGeneration, sessionId, deadlineElapsedMs;
        Token(String ownerId, long ownerGeneration, long sessionId, String requestId, String peer,
                String bindingCommitment, long deadlineElapsedMs) {
            this.ownerId = ownerId; this.ownerGeneration = ownerGeneration; this.sessionId = sessionId;
            this.requestId = requestId; this.peer = peer; this.bindingCommitment = bindingCommitment;
            this.deadlineElapsedMs = deadlineElapsedMs;
        }
        boolean sameSession(Token other) { return sameIdentity(other) && sessionId == other.sessionId; }
        boolean equals(Token other) {
            return other != null && sameSession(other) && deadlineElapsedMs == other.deadlineElapsedMs;
        }
        boolean matches(String ownerId, long generation, byte[] requestId, long sessionId,
                InetAddress peer, byte[] commitment) {
            return this.ownerId.equals(ownerId) && ownerGeneration == generation && this.sessionId == sessionId
                    && this.requestId.equals(hex(requestId)) && this.peer.equals(hex(peer.getAddress()))
                    && bindingCommitment.equals(hex(commitment));
        }
        private boolean sameIdentity(Token other) {
            return other != null && ownerId.equals(other.ownerId) && ownerGeneration == other.ownerGeneration
                    && requestId.equals(other.requestId) && peer.equals(other.peer)
                    && bindingCommitment.equals(other.bindingCommitment);
        }
    }

    private static final class Request {
        final Token token;
        int status;
        Request(Token token, int status) { this.token = token; this.status = status; }
    }

    private static final class Episode {
        final String ownerId, requestId, peer, bindingCommitment;
        final long ownerGeneration, deadlineElapsedMs;
        int status;
        Episode(Token token, int status) {
            ownerId = token.ownerId; ownerGeneration = token.ownerGeneration; requestId = token.requestId;
            peer = token.peer; bindingCommitment = token.bindingCommitment;
            deadlineElapsedMs = token.deadlineElapsedMs; this.status = status;
        }
        boolean matches(Token token) { return ownerId.equals(token.ownerId) && ownerGeneration == token.ownerGeneration
                && requestId.equals(token.requestId) && peer.equals(token.peer)
                && bindingCommitment.equals(token.bindingCommitment); }
    }

    private static final class OnceGrant {
        final String ownerId, requestId, peer, bindingCommitment;
        final long ownerGeneration;
        long recoveryDeadlineElapsedMs;
        final long requestDeadlineElapsedMs;
        boolean playbackActive;
        OnceGrant(Token token) {
            ownerId = token.ownerId; ownerGeneration = token.ownerGeneration; requestId = token.requestId;
            peer = token.peer; bindingCommitment = token.bindingCommitment;
            requestDeadlineElapsedMs = token.deadlineElapsedMs;
        }
        boolean matches(Token token) { return ownerId.equals(token.ownerId) && ownerGeneration == token.ownerGeneration
                && requestId.equals(token.requestId) && peer.equals(token.peer)
                && bindingCommitment.equals(token.bindingCommitment); }
    }

    private static final class TimeUnitHours { static final long EIGHT_HOURS_MS = 8L * 60 * 60 * 1000; }
}
