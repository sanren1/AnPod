package app.anpod.receiver;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;

public final class ReceiverService extends Service {
    private static final String CHANNEL_ID = "receiver-management";
    private static final int NOTIFICATION_ID = 1;
    private static final long MAX_RUN_MS = 120_000L;

    static volatile boolean foregroundReady;
    static volatile long startedAtElapsedMs;
    static volatile String startFailure = "";

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable timeout;

    @Override
    public void onCreate() {
        super.onCreate();
        foregroundReady = false;
        startedAtElapsedMs = 0;
        startFailure = "";
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        try {
            Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                    ? new Notification.Builder(this, CHANNEL_ID)
                    : new Notification.Builder(this);
            Notification notification = builder
                    .setSmallIcon(android.R.drawable.stat_sys_headset)
                    .setContentTitle("AnPod Receiver")
                    .setContentText("管理检查进行中；当前不播放音频。")
                    .setOngoing(true)
                    .build();
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(NOTIFICATION_ID, notification,
                        android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE);
            } else {
                startForeground(NOTIFICATION_ID, notification);
            }
            startedAtElapsedMs = android.os.SystemClock.elapsedRealtime();
            foregroundReady = true;
        } catch (RuntimeException error) {
            recordStartFailure(error.getClass().getSimpleName());
            stopSelf(startId);
            return START_NOT_STICKY;
        }

        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = () -> stopSelf(startId);
        handler.postDelayed(timeout, MAX_RUN_MS);
        return START_NOT_STICKY;
    }

    @Override
    public void onTimeout(int startId) {
        foregroundReady = false;
        stopSelf(startId);
    }

    @Override
    public void onDestroy() {
        foregroundReady = false;
        if (timeout != null) handler.removeCallbacks(timeout);
        if (Build.VERSION.SDK_INT >= 24) stopForeground(STOP_FOREGROUND_REMOVE);
        else stopForeground(true);
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    static void recordStartFailure(String reason) {
        startFailure = reason;
        foregroundReady = false;
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "Receiver 管理", NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("显示由电脑发起的短时 Receiver 管理状态。");
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
    }
}
