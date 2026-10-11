package app.anpod.receiver;

import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.widget.TextView;

public final class BootstrapActivity extends Activity {
    public static final String ACTION_START = "app.anpod.receiver.action.START";
    public static final String ACTION_START_AUDIO = "app.anpod.receiver.action.START_AUDIO";
    public static final String ACTION_STOP = "app.anpod.receiver.action.STOP";
    private TextView status;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        status = new TextView(this);
        status.setText("AnPod Receiver 正在响应电脑上的管理请求…");
        status.setPadding(32, 32, 32, 32);
        setContentView(status);

        String action = getIntent().getAction();
        if (ACTION_STOP.equals(action)) {
            stopService(new Intent(this, ReceiverService.class));
            status.setText("AnPod Receiver 已停止。");
            status.postDelayed(this::finish, 250);
        } else if (ACTION_START.equals(action) || ACTION_START_AUDIO.equals(action)) {
            requestNotificationPermissionOrStart();
        } else {
            finish();
        }
    }

    private void requestNotificationPermissionOrStart() {
        if (Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {android.Manifest.permission.POST_NOTIFICATIONS}, 1);
            return;
        }
        startReceiverService();
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode != 1) return;
        if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            startReceiverService();
        } else {
            status.setText("需要通知权限才能显示 Receiver 的系统状态。");
            status.postDelayed(this::finish, 1_500);
        }
    }

    private void startReceiverService() {
        try {
            Intent service = new Intent(this, ReceiverService.class);
            String action = getIntent().getAction();
            service.setAction(action);
            if (ACTION_START_AUDIO.equals(action)) {
                service.putExtras(getIntent().getExtras());
            }
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(service);
            else startService(service);
            status.setText("AnPod Receiver 管理服务已启动，正在等待电脑检查状态。");
            status.postDelayed(this::finish, 1_000);
        } catch (RuntimeException error) {
            ReceiverService.recordStartFailure(error.getClass().getSimpleName());
            status.setText("AnPod Receiver 启动失败，请查看系统限制后重试。");
            status.postDelayed(this::finish, 1_500);
        }
    }
}
