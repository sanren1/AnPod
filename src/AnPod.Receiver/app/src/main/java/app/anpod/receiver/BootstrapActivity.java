package app.anpod.receiver;

import android.app.Activity;
import android.content.Intent;
import android.os.Build;
import android.os.Bundle;

/**
 * Invisible trampoline for the adb-driven Receiver actions.
 *
 * <p>{@link ReceiverService} stays unexported, so the shell cannot start it directly; this activity
 * is the only DUMP-protected entry point. It owns no window ({@code Theme.NoDisplay}) and finishes
 * before resuming, so connecting, switching playback mode and stopping no longer flash a management
 * screen on the phone while the service keeps running.
 */
public final class BootstrapActivity extends Activity {
    public static final String ACTION_START = "app.anpod.receiver.action.START";
    public static final String ACTION_START_AUDIO = "app.anpod.receiver.action.START_AUDIO";
    public static final String ACTION_START_NETWORK_AUDIO = "app.anpod.receiver.action.START_NETWORK_AUDIO";
    public static final String ACTION_STOP = "app.anpod.receiver.action.STOP";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        try {
            Intent source = getIntent();
            Intent service = new Intent(this, ReceiverService.class);
            service.setAction(source.getAction());
            Bundle extras = source.getExtras();
            if (extras != null) service.putExtras(extras);
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(service);
            else startService(service);
        } catch (RuntimeException error) {
            ReceiverService.recordStartFailure(error.getClass().getSimpleName());
        } finally {
            // Theme.NoDisplay requires finishing before onResume.
            finish();
        }
    }
}
