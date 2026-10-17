package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;

import java.io.IOException;

/** Background authorization: notification, user decision handling, and recovery deadlines. */
final class ReceiverBackgroundAuthorization {
    private final ReceiverService service;

    ReceiverBackgroundAuthorization(ReceiverService service) {
        this.service = service;
    }

    void update(BackgroundStartAuthorization.Token supplied) {
        if (service.destroyed || !service.backgroundStandbyEnabled || !service.notificationPermissionAvailable()) {
            cancel();
            if (service.backgroundStandbyEnabled)
                service.requestTermination(service.uiOperation.ownerId(), service.currentOperationGeneration, true, "BackgroundNotificationsUnavailable");
            return;
        }
        BackgroundStartAuthorization.Token token;
        synchronized (BindingStore.storageLock()) { token = service.backgroundAuthorization.pendingToken(); }
        if (token == null || !token.equals(supplied) || !service.isCurrentOperation(token.ownerId, token.ownerGeneration)) {
            cancel();
            return;
        }
        int immutable = Build.VERSION.SDK_INT >= 23 ? PendingIntent.FLAG_IMMUTABLE : 0;
        PendingIntent once = decisionIntent(token, BackgroundStartAuthorization.ONCE, immutable);
        PendingIntent eightHours = decisionIntent(token,
                BackgroundStartAuthorization.EIGHT_HOURS, immutable);
        PendingIntent deny = decisionIntent(token, BackgroundStartAuthorization.DENIED, immutable);
        Notification notification = ReceiverNotifications.authorization(service, ReceiverService.CHANNEL_ID,
                service.mainActivityPendingIntent(), once, eightHours, deny);
        NotificationManager manager = service.getSystemService(NotificationManager.class);
        if (manager != null) manager.notify(ReceiverService.AUTH_NOTIFICATION_ID, notification);
    }

    private PendingIntent decisionIntent(BackgroundStartAuthorization.Token token,
            int decision, int immutable) {
        String key = token.ownerId + "/" + token.ownerGeneration + "/" + token.sessionId + "/"
                + token.requestId + "/" + token.peer + "/" + token.bindingCommitment + "/"
                + token.deadlineElapsedMs + "/" + decision;
        Intent action = new Intent(service, ReceiverService.class).setAction(ReceiverService.ACTION_BACKGROUND_AUTH_DECISION)
                .setData(Uri.parse("anpod://receiver/background-auth/" + Uri.encode(key)))
                .putExtra(ReceiverService.EXTRA_AUTH_OWNER, token.ownerId).putExtra(ReceiverService.EXTRA_AUTH_GENERATION, token.ownerGeneration)
                .putExtra(ReceiverService.EXTRA_AUTH_SESSION, token.sessionId).putExtra(ReceiverService.EXTRA_AUTH_REQUEST, token.requestId)
                .putExtra(ReceiverService.EXTRA_AUTH_PEER, token.peer).putExtra(ReceiverService.EXTRA_AUTH_BINDING, token.bindingCommitment)
                .putExtra(ReceiverService.EXTRA_AUTH_DEADLINE, token.deadlineElapsedMs).putExtra(ReceiverService.EXTRA_AUTH_DECISION, decision);
        return PendingIntent.getService(service, key.hashCode(), action,
                PendingIntent.FLAG_UPDATE_CURRENT | immutable);
    }

    void handleDecision(Intent intent, int startId) {
        String ownerId = intent.getStringExtra(ReceiverService.EXTRA_AUTH_OWNER);
        long generation = intent.getLongExtra(ReceiverService.EXTRA_AUTH_GENERATION, -1L);
        long sessionId = intent.getLongExtra(ReceiverService.EXTRA_AUTH_SESSION, -1L);
        String requestId = intent.getStringExtra(ReceiverService.EXTRA_AUTH_REQUEST);
        String peer = intent.getStringExtra(ReceiverService.EXTRA_AUTH_PEER);
        String commitment = intent.getStringExtra(ReceiverService.EXTRA_AUTH_BINDING);
        long deadline = intent.getLongExtra(ReceiverService.EXTRA_AUTH_DEADLINE, 0L);
        int decision = intent.getIntExtra(ReceiverService.EXTRA_AUTH_DECISION, BackgroundStartAuthorization.DENIED);
        if (!intent.hasExtra(ReceiverService.EXTRA_AUTH_OWNER) || !intent.hasExtra(ReceiverService.EXTRA_AUTH_GENERATION)
                || !intent.hasExtra(ReceiverService.EXTRA_AUTH_SESSION) || !intent.hasExtra(ReceiverService.EXTRA_AUTH_REQUEST)
                || !intent.hasExtra(ReceiverService.EXTRA_AUTH_PEER) || !intent.hasExtra(ReceiverService.EXTRA_AUTH_BINDING)
                || !intent.hasExtra(ReceiverService.EXTRA_AUTH_DEADLINE) || !intent.hasExtra(ReceiverService.EXTRA_AUTH_DECISION)
                || ownerId == null || requestId == null || peer == null || commitment == null
                || requestId.length() != 32 || peer.length() != 8 || commitment.length() != 64
                || deadline <= 0) {
            stopStaleAuthorizationAction(startId);
            return;
        }
        BackgroundStartAuthorization.Token token = new BackgroundStartAuthorization.Token(ownerId,
                generation, sessionId, requestId, peer, commitment, deadline);
        try {
            synchronized (BindingStore.storageLock()) {
                service.backgroundAuthorization.decide(token, decision);
            }
        } catch (IOException error) {
            setError(ownerId, generation, error.getClass().getSimpleName());
        }
        stopStaleAuthorizationAction(startId);
    }

    private void stopStaleAuthorizationAction(int startId) {
        if (service.uiOperation.canBegin() && ReceiverService.currentAudio == null && service.networkCoordinator == null) service.stopSelf(startId);
    }

    void cancel() {
        NotificationManager manager = service.getSystemService(NotificationManager.class);
        if (manager != null) manager.cancel(ReceiverService.AUTH_NOTIFICATION_ID);
    }

    void setError(String ownerId, long generation, String message) {
        synchronized (BindingStore.storageLock()) { service.backgroundAuthorization.reportError(ownerId, generation, message); }
    }

    long onceDeadline(String ownerId, long generation) {
        synchronized (BindingStore.storageLock()) {
            return service.backgroundAuthorization.onceRecoveryDeadlineElapsedMs(ownerId, generation);
        }
    }

    long newDeadline(String ownerId, long generation) {
        long fixed = onceDeadline(ownerId, generation);
        return fixed == 0 ? android.os.SystemClock.elapsedRealtime() + 30_000L : fixed;
    }
}
