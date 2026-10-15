package app.anpod.receiver;

import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.widget.TextView;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;

public final class BootstrapActivity extends Activity {
    public static final String ACTION_START = "app.anpod.receiver.action.START";
    public static final String ACTION_START_AUDIO = "app.anpod.receiver.action.START_AUDIO";
    public static final String ACTION_START_NETWORK_AUDIO = "app.anpod.receiver.action.START_NETWORK_AUDIO";
    public static final String ACTION_STOP = "app.anpod.receiver.action.STOP";
    private static final long NETWORK_START_WINDOW_MS = 25_000L;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private TextView status;
    private long networkDeadline;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        status = new TextView(this);
        status.setText("AnPod Receiver 正在响应电脑上的管理请求…");
        status.setPadding(32, 32, 32, 32);
        setContentView(status);

        String action = getIntent().getAction();
        if (ACTION_STOP.equals(action)) {
            try {
                startService(new Intent(this, ReceiverService.class).setAction(ACTION_STOP));
                status.setText("AnPod Receiver 正在停止并清理当前音频会话…");
            } catch (RuntimeException error) {
                status.setText("AnPod Receiver 停止请求失败，请重试。");
            }
            status.postDelayed(this::finish, 250);
        } else if (ACTION_START.equals(action) || ACTION_START_AUDIO.equals(action)
                || ACTION_START_NETWORK_AUDIO.equals(action)) {
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
            } else if (ACTION_START_NETWORK_AUDIO.equals(action)) {
                service.putExtra("networkAddress", getIntent().getStringExtra("networkAddress"));
                service.putExtra("networkPrefixLength", getIntent().getIntExtra("networkPrefixLength", 0));
            }
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(service);
            else startService(service);
            if (ACTION_START_NETWORK_AUDIO.equals(action)) {
                status.setText("已通过 USB 启动网络接收。保持此页面打开，等待电脑完成认证和开始播放。");
                networkDeadline = SystemClock.elapsedRealtime() + NETWORK_START_WINDOW_MS;
                status.postDelayed(this::checkNetworkStart, 100);
            } else {
                status.setText("AnPod Receiver 管理服务已启动，正在等待电脑检查状态。");
                status.postDelayed(this::finish, 1_000);
            }
        } catch (RuntimeException error) {
            ReceiverService.recordStartFailure(error.getClass().getSimpleName());
            status.setText("AnPod Receiver 启动失败，请查看系统限制后重试。");
            status.postDelayed(this::finish, 1_500);
        }
    }

    private void checkNetworkStart() {
        String state = ReceiverService.networkAudioState;
        if ("receiverRunning".equals(state) || "streaming".equals(state) || "paused".equals(state)) {
            status.setText("电脑已通过认证；Receiver 音频会话已启动。");
            status.postDelayed(this::finish, 500);
        } else if ("failed".equals(state) || !ReceiverService.startFailure.isEmpty()) {
            status.setText("网络接收启动失败：" + ReceiverService.startFailure + "。请检查 USB 引导和 Receiver 状态。");
            status.postDelayed(this::finish, 1_500);
        } else if (SystemClock.elapsedRealtime() >= networkDeadline) {
            status.setText("认证等待已超时；临时接收已关闭。请通过 USB/bootstrap 重试。");
            status.postDelayed(this::finish, 1_500);
        } else {
            status.postDelayed(this::checkNetworkStart, 150);
        }
    }

    @Override
    protected void onDestroy() {
        handler.removeCallbacksAndMessages(null);
        super.onDestroy();
    }
}
