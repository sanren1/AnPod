package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.pm.PackageManager;
import android.os.Build;

/** Builds the Receiver notifications so the service only wires intents and state. */
final class ReceiverNotifications {
    private static final String TITLE = "AnPod Receiver";

    private ReceiverNotifications() { }

    static boolean canPost(Context context, String channelId) {
        if (Build.VERSION.SDK_INT >= 33
                && context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED)
            return false;
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null || (Build.VERSION.SDK_INT >= 24 && !manager.areNotificationsEnabled())) return false;
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel channel = manager.getNotificationChannel(channelId);
            if (channel == null || channel.getImportance() == NotificationManager.IMPORTANCE_NONE) return false;
        }
        return true;
    }

    static Notification authorization(Context context, String channelId, PendingIntent content,
            PendingIntent once, PendingIntent eightHours, PendingIntent deny) {
        Notification.Builder builder = builder(context, channelId);
        builder.setSmallIcon(android.R.drawable.stat_sys_headset)
                .setContentTitle(TITLE)
                .setContentText("已配对电脑请求后台播放，请选择是否允许。")
                .setContentIntent(content)
                .setAutoCancel(true)
                .setOnlyAlertOnce(true)
                .addAction(android.R.drawable.ic_media_play, "允许一次", once)
                .addAction(android.R.drawable.ic_lock_lock, "信任 8 小时", eightHours)
                .addAction(android.R.drawable.ic_delete, "拒绝", deny);
        builder.setDeleteIntent(deny);
        return builder.build();
    }

    static Notification playback(Context context, String channelId, String text,
            PendingIntent content, PendingIntent stopAction, PendingIntent aodAction) {
        Notification.Builder builder = builder(context, channelId);
        builder.setSmallIcon(android.R.drawable.stat_sys_headset)
                .setContentTitle(TITLE)
                .setContentText(text)
                .setContentIntent(content)
                .setOngoing(true);
        if (aodAction != null) builder.addAction(android.R.drawable.ic_menu_view, "打开常显", aodAction);
        if (stopAction != null) builder.addAction(android.R.drawable.ic_media_pause, "停止接收", stopAction);
        return builder.build();
    }

    static Notification standbyRecovery(Context context, String channelId, PendingIntent content) {
        Notification.Builder builder = builder(context, channelId);
        builder.setSmallIcon(android.R.drawable.stat_sys_headset)
                .setContentTitle(context.getString(R.string.background_standby_recovery_title))
                .setContentText(context.getString(R.string.background_standby_recovery_text))
                .setContentIntent(content)
                .setAutoCancel(true)
                .setOnlyAlertOnce(true);
        return builder.build();
    }

    private static Notification.Builder builder(Context context, String channelId) {
        return Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(context, channelId) : new Notification.Builder(context);
    }
}
