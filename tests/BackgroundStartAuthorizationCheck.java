package app.anpod.receiver;

import java.net.InetAddress;

public final class BackgroundStartAuthorizationCheck {
    public static void main(String[] args) throws Exception {
        onceRecoveryAndStop();
        pendingReconnectDoesNotExtendDeadline();
        cancelledRequestCannotBeReopenedByItsNotification();
        onceRecoveryCannotExtendItsOriginalGrace();
        transientTrustExpiresAndDoesNotSurviveRestart();
        eightHourExpirySurvivesSnapshotReads();
        signedSessionIdsRemainValidTokens();
        persistentTrustSurvivesRestartAndResetClearsIt();
        staleAndExpiredDecisionsAreIgnored();
    }

    private static void onceRecoveryAndStop() throws Exception {
        TestClock clock = new TestClock();
        MemoryStore store = new MemoryStore();
        BackgroundStartAuthorization auth = auth(clock, store);
        byte[] binding = bytes(1), request = bytes(2);
        auth.enable("owner", 7, binding);
        check(auth.request("owner", 7, request, 10, peer(), binding) == BackgroundStartAuthorization.PENDING, "initial request must pend");
        BackgroundStartAuthorization.Token token = auth.pendingToken();
        check(auth.decide(token, BackgroundStartAuthorization.ONCE), "current once decision must apply");
        check(auth.status("owner", 7, request, 10, peer(), binding) == BackgroundStartAuthorization.ONCE, "once must authorize the active session");
        auth.end("owner", 7, request, 10, peer(), binding, true, true);
        clock.advance(29_000);
        check(auth.request("owner", 7, request, 11, peer(), binding) == BackgroundStartAuthorization.ONCE, "unexpected disconnect may recover within the original 30 seconds");
        auth.end("owner", 7, request, 11, peer(), binding, true, false);
        check(auth.request("owner", 7, bytes(9), 12, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "normal STOP must clear once authorization for the next episode");
        check(!auth.decide(token, BackgroundStartAuthorization.EIGHT_HOURS), "old session notification must not affect a new request");
    }

    private static void transientTrustExpiresAndDoesNotSurviveRestart() throws Exception {
        TestClock clock = new TestClock();
        MemoryStore store = new MemoryStore();
        byte[] binding = bytes(3), request = bytes(4);
        BackgroundStartAuthorization first = auth(clock, store);
        first.enable("owner", 1, binding);
        check(first.request("owner", 1, request, 20, peer(), binding) == BackgroundStartAuthorization.PENDING, "initial request must pend");
        check(first.decide(first.pendingToken(), BackgroundStartAuthorization.EIGHT_HOURS), "eight-hour trust must apply");
        check(first.grantKind().equals("eightHours") && first.grantRemainingMs() == BackgroundStartAuthorization.EIGHT_HOURS_MS,
                "eight-hour trust must expose its elapsed-time remainder");
        clock.advance(BackgroundStartAuthorization.EIGHT_HOURS_MS);
        check(first.request("owner", 1, bytes(18), 23, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "eight-hour trust must expire on the monotonic clock");

        BackgroundStartAuthorization restarted = auth(clock, store);
        restarted.enable("owner", 2, binding);
        check(restarted.request("owner", 2, request, 21, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "eight-hour trust must be lost on process restart");
        clock.advance(BackgroundStartAuthorization.EIGHT_HOURS_MS + 1);
        check(restarted.request("owner", 2, bytes(11), 22, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "expired transient trust must not authorize");
    }

    private static void eightHourExpirySurvivesSnapshotReads() throws Exception {
        TestClock clock = new TestClock();
        MemoryStore store = new MemoryStore();
        BackgroundStartAuthorization auth = auth(clock, store);
        byte[] binding = bytes(19), request = bytes(20);
        auth.enable("owner", 11, binding);
        check(auth.request("owner", 11, request, 80, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "request must pend before eight-hour trust");
        check(auth.decide(auth.pendingToken(), BackgroundStartAuthorization.EIGHT_HOURS),
                "eight-hour trust must apply");
        long deadline = auth.eightHourDeadlineElapsedMs("owner", 11);
        clock.advance(BackgroundStartAuthorization.EIGHT_HOURS_MS);
        check(auth.grantKind().equals("none") && auth.grantRemainingMs() == 0,
                "expired eight-hour grant must disappear from snapshots");
        check(auth.status("owner", 11, request, 80, peer(), binding) == BackgroundStartAuthorization.EXPIRED,
                "expired eight-hour grant must no longer authorize the session");
        check(auth.eightHourDeadlineElapsedMs("owner", 11) == deadline,
                "snapshot and status reads must preserve the waiter's expiry signal");
        check(auth.expireEightHourGrant("owner", 11, deadline),
                "waiter must consume the expiry and stop standby when no permanent grant exists");
        check(!auth.expireEightHourGrant("owner", 11, deadline), "expiry signal must be consumed only once");

        auth.enable("owner", 12, binding);
        check(auth.request("owner", 12, request, 81, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "request must pend before permanent trust upgrade");
        check(auth.decide(auth.pendingToken(), BackgroundStartAuthorization.EIGHT_HOURS),
                "second eight-hour trust must apply");
        long upgradedDeadline = auth.eightHourDeadlineElapsedMs("owner", 12);
        clock.advance(BackgroundStartAuthorization.EIGHT_HOURS_MS);
        auth.trustUntilRevoked("owner", 12);
        check(!auth.expireEightHourGrant("owner", 12, upgradedDeadline),
                "permanent trust upgrade must suppress the expiry stop signal");
        check(auth.status("owner", 12, request, 81, peer(), binding) == BackgroundStartAuthorization.UNTIL_REVOKED,
                "permanent trust upgrade must authorize the current session");
    }

    private static void signedSessionIdsRemainValidTokens() throws Exception {
        TestClock clock = new TestClock();
        BackgroundStartAuthorization auth = auth(clock, new MemoryStore());
        byte[] binding = bytes(21), request = bytes(22);
        long sessionId = Long.MIN_VALUE;
        auth.enable("owner", 13, binding);
        check(auth.request("owner", 13, request, sessionId, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "all 64-bit session IDs, including signed-negative values, are valid");
        BackgroundStartAuthorization.Token token = auth.pendingToken();
        check(auth.decide(token, BackgroundStartAuthorization.ONCE), "valid signed session token must be actionable");
        check(auth.status("owner", 13, request, sessionId, peer(), binding) == BackgroundStartAuthorization.ONCE,
                "exact signed session token must authorize");
        check(auth.status("owner", 13, request, sessionId + 1, peer(), binding) == BackgroundStartAuthorization.EXPIRED,
                "a changed session ID must not match the token");
    }

    private static void pendingReconnectDoesNotExtendDeadline() throws Exception {
        TestClock clock = new TestClock();
        BackgroundStartAuthorization auth = auth(clock, new MemoryStore());
        byte[] binding = bytes(12), request = bytes(13);
        auth.enable("owner", 2, binding);
        check(auth.request("owner", 2, request, 25, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "initial request must pend");
        BackgroundStartAuthorization.Token stale = auth.pendingToken();
        auth.end("owner", 2, request, 25, peer(), binding, false, false);
        clock.advance(10_000);
        check(auth.request("owner", 2, request, 26, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "same episode may rebind a pending token after reconnect");
        BackgroundStartAuthorization.Token current = auth.pendingToken();
        check(current.deadlineElapsedMs == stale.deadlineElapsedMs, "reconnect must not extend the 30-second request window");
        check(!auth.decide(stale, BackgroundStartAuthorization.ONCE), "old session notification must be stale");
        check(auth.decide(current, BackgroundStartAuthorization.DENIED), "current denial must apply");
        check(auth.request("owner", 2, request, 27, peer(), binding) == BackgroundStartAuthorization.DENIED,
                "repeated request ID must preserve its denied result");
    }

    private static void cancelledRequestCannotBeReopenedByItsNotification() throws Exception {
        TestClock clock = new TestClock();
        BackgroundStartAuthorization auth = auth(clock, new MemoryStore());
        byte[] binding = bytes(14), request = bytes(15);
        auth.enable("owner", 6, binding);
        check(auth.request("owner", 6, request, 60, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "request must pend before cancellation");
        BackgroundStartAuthorization.Token token = auth.pendingToken();
        check(auth.cancel(token), "current request must cancel");
        check(auth.status("owner", 6, request, 60, peer(), binding) == BackgroundStartAuthorization.CANCELLED,
                "cancelled status must remain visible to the active session");
        check(!auth.decide(token, BackgroundStartAuthorization.ONCE), "cancelled notification action must be ignored");
        check(auth.request("owner", 6, request, 61, peer(), binding) == BackgroundStartAuthorization.CANCELLED,
                "same request ID must not reopen a cancelled request");
    }

    private static void onceRecoveryCannotExtendItsOriginalGrace() throws Exception {
        TestClock clock = new TestClock();
        BackgroundStartAuthorization auth = auth(clock, new MemoryStore());
        byte[] binding = bytes(16), request = bytes(17);
        auth.enable("owner", 10, binding);
        check(auth.request("owner", 10, request, 70, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "request must pend before once grant");
        check(auth.decide(auth.pendingToken(), BackgroundStartAuthorization.ONCE), "once grant must apply");
        auth.end("owner", 10, request, 70, peer(), binding, true, true);
        long deadline = auth.onceRecoveryDeadlineElapsedMs("owner", 10);
        clock.advance(20_000);
        check(auth.request("owner", 10, request, 71, peer(), binding) == BackgroundStartAuthorization.ONCE,
                "once grant may recover before its original deadline");
        auth.end("owner", 10, request, 71, peer(), binding, true, true);
        check(auth.onceRecoveryDeadlineElapsedMs("owner", 10) == deadline,
                "a second disconnect must not extend the once recovery lease");
        clock.advance(10_001);
        check(auth.request("owner", 10, request, 72, peer(), binding) == BackgroundStartAuthorization.EXPIRED,
                "once recovery must expire at the original fixed deadline");
    }

    private static void persistentTrustSurvivesRestartAndResetClearsIt() throws Exception {
        TestClock clock = new TestClock();
        MemoryStore store = new MemoryStore();
        byte[] binding = bytes(5), request = bytes(6);
        BackgroundStartAuthorization first = auth(clock, store);
        first.enable("owner", 3, binding);
        first.trustUntilRevoked("owner", 3);

        BackgroundStartAuthorization restarted = auth(clock, store);
        restarted.enable("owner", 4, binding);
        check(restarted.request("owner", 4, request, 30, peer(), binding) == BackgroundStartAuthorization.UNTIL_REVOKED,
                "until-revoked trust must survive process restart");
        restarted.resetBinding(BackgroundStartAuthorization.hex(binding));
        check(store.value == null && !restarted.enabled(), "binding reset must clear durable and active trust");
        restarted.enable("owner", 5, binding);
        check(restarted.request("owner", 5, request, 31, peer(), binding) == BackgroundStartAuthorization.PENDING,
                "reset binding must require a fresh grant");
        restarted.trustUntilRevoked("owner", 5);
        restarted.revoke("owner", 5);
        check(store.value == null && !restarted.enabled(), "explicit revocation must clear the durable grant and stop standby");
    }

    private static void staleAndExpiredDecisionsAreIgnored() throws Exception {
        TestClock clock = new TestClock();
        BackgroundStartAuthorization auth = auth(clock, new MemoryStore());
        byte[] binding = bytes(7), request = bytes(8);
        auth.enable("owner", 8, binding);
        check(auth.request("owner", 8, request, 40, peer(), binding) == BackgroundStartAuthorization.PENDING, "initial request must pend");
        BackgroundStartAuthorization.Token expired = auth.pendingToken();
        clock.advance(BackgroundStartAuthorization.REQUEST_WINDOW_MS);
        check(!auth.decide(expired, BackgroundStartAuthorization.ONCE), "expired notification must be ignored");
        check(auth.status("owner", 8, request, 40, peer(), binding) == BackgroundStartAuthorization.EXPIRED, "pending request must expire");
        check(auth.request("owner", 8, bytes(10), 41, peer(), binding) == BackgroundStartAuthorization.PENDING, "new request ID must create a new episode");
        check(!auth.decide(expired, BackgroundStartAuthorization.ONCE), "old notification must not change the new request");
        auth.enable("owner", 9, binding);
        check(!auth.decide(auth.pendingToken(), BackgroundStartAuthorization.ONCE), "old operation generation must be rejected");
    }

    private static BackgroundStartAuthorization auth(TestClock clock, MemoryStore store) {
        return new BackgroundStartAuthorization(clock, store, null);
    }

    private static InetAddress peer() throws Exception { return InetAddress.getByName("192.168.1.20"); }
    private static byte[] bytes(int value) { byte[] result = new byte[32]; java.util.Arrays.fill(result, (byte) value); return result; }
    private static void check(boolean condition, String message) { if (!condition) throw new AssertionError(message); }

    private static final class TestClock implements BackgroundStartAuthorization.Clock {
        private long now;
        @Override public long elapsedRealtime() { return now; }
        void advance(long millis) { now += millis; }
    }

    private static final class MemoryStore implements BackgroundStartAuthorization.GrantStore {
        String value;
        @Override public boolean hasUntilRevoked(String commitment) { return commitment.equals(value); }
        @Override public void saveUntilRevoked(String commitment) { value = commitment; }
        @Override public void clearUntilRevoked(String commitment) { if (commitment.equals(value)) value = null; }
    }
}
