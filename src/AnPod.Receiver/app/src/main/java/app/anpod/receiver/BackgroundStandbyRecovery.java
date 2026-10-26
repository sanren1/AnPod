package app.anpod.receiver;

import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.SharedPreferences;
import android.content.Intent;
import android.util.Log;

/** Stores the user's separate choice to offer one standby restore after boot. */
final class BackgroundStandbyRecovery {
    static final int NOTIFICATION_ID = 3;
    private static final String PREFERENCES = "anpod_receiver";
    private static final String KEY_BINDING = "boot_standby_binding_commitment";
    private static final String KEY_OPERATION = "boot_standby_operation_id";

    private BackgroundStandbyRecovery() { }

    static String bindingCommitment(Context context) {
        return context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
                .getString(KEY_BINDING, "");
    }

    static String operationId(Context context) {
        return context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
                .getString(KEY_OPERATION, "");
    }

    static boolean save(Context context, String bindingCommitment, String operationId) {
        if (!isCommitment(bindingCommitment) || operationId == null || operationId.isEmpty()) return false;
        return preferences(context).edit().putString(KEY_BINDING, bindingCommitment)
                .putString(KEY_OPERATION, operationId).commit();
    }

    static boolean matches(Context context, String expectedCommitment, String expectedOperationId,
            String currentBindingCommitment) {
        String stored = bindingCommitment(context);
        if (stored.isEmpty() || !stored.equals(expectedCommitment)
                || !operationId(context).equals(expectedOperationId)) return false;
        if (stored.equals(currentBindingCommitment)) return true;
        clear(context);
        return false;
    }

    static void clear(Context context) {
        String commitment = bindingCommitment(context);
        String operationId = operationId(context);
        if (!commitment.isEmpty() && !operationId.isEmpty()) cancelPendingIntent(context, commitment, operationId);
        if (!preferences(context).edit().remove(KEY_BINDING).remove(KEY_OPERATION).commit())
            Log.e("AnPodReceiver", "Could not clear boot standby recovery intent");
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager != null) manager.cancel(NOTIFICATION_ID);
    }

    static Intent notificationIntent(Context context, String bindingCommitment, String operationId) {
        return new Intent(context, RecoveryActivity.class)
                .setData(android.net.Uri.parse("anpod://receiver/standby-recovery/" + bindingCommitment + "/" + operationId))
                .putExtra(RecoveryActivity.EXTRA_BINDING_COMMITMENT, bindingCommitment)
                .putExtra(RecoveryActivity.EXTRA_OPERATION_ID, operationId)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
    }

    private static void cancelPendingIntent(Context context, String commitment, String operationId) {
        PendingIntent pending = PendingIntent.getActivity(context, NOTIFICATION_ID,
                notificationIntent(context, commitment, operationId), PendingIntent.FLAG_NO_CREATE
                        | PendingIntent.FLAG_ONE_SHOT | PendingIntent.FLAG_IMMUTABLE);
        if (pending != null) pending.cancel();
    }

    private static SharedPreferences preferences(Context context) {
        return context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE);
    }

    private static boolean isCommitment(String value) {
        if (value == null || value.length() != 64) return false;
        for (int index = 0; index < value.length(); index++) {
            char item = value.charAt(index);
            if (!(item >= '0' && item <= '9') && !(item >= 'a' && item <= 'f')) return false;
        }
        return true;
    }
}
