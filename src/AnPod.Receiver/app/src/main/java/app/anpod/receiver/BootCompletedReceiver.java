package app.anpod.receiver;

import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

import java.io.IOException;

/** Posts a restore prompt only; it never starts the Receiver service. */
public final class BootCompletedReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        if (intent == null || !Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction())) return;
        String expectedCommitment = BackgroundStandbyRecovery.bindingCommitment(context);
        String operationId = BackgroundStandbyRecovery.operationId(context);
        if (expectedCommitment.isEmpty() || operationId.isEmpty()) return;

        BindingStore.Status binding;
        try { binding = new BindingStore(context.getFilesDir()).status(); }
        catch (IOException | PairingProtocol.Failure error) {
            Log.e("AnPodReceiver", "Could not check binding before boot standby notification", error);
            return;
        }
        if (!"saved".equals(binding.state)
                || !BackgroundStandbyRecovery.matches(context, expectedCommitment, operationId, binding.commitment)) {
            BackgroundStandbyRecovery.clear(context);
            return;
        }
        if (!ReceiverNotifications.canPost(context, ReceiverService.CHANNEL_ID)) return;

        Intent open = BackgroundStandbyRecovery.notificationIntent(context, expectedCommitment, operationId);
        PendingIntent content = PendingIntent.getActivity(context, BackgroundStandbyRecovery.NOTIFICATION_ID,
                open, PendingIntent.FLAG_CANCEL_CURRENT | PendingIntent.FLAG_ONE_SHOT | PendingIntent.FLAG_IMMUTABLE);
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager != null) manager.notify(BackgroundStandbyRecovery.NOTIFICATION_ID,
                ReceiverNotifications.standbyRecovery(context, ReceiverService.CHANNEL_ID, content));
    }
}
