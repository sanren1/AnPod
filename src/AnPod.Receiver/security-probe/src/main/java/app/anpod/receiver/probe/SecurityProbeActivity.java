package app.anpod.receiver.probe;

import android.app.Activity;
import android.content.ComponentName;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;

public final class SecurityProbeActivity extends Activity {
    private static final String TAG = "AnPodReceiverProbe";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        String provider = "error";
        try (Cursor cursor = getContentResolver().query(
                Uri.parse("content://app.anpod.receiver.state/health"), null, null, null, null)) {
            provider = cursor == null ? "no-result" : "allowed";
        } catch (SecurityException denied) {
            provider = "denied";
        } catch (RuntimeException error) {
            provider = "error";
        }

        String bootstrap = "error";
        try {
            Intent intent = new Intent("app.anpod.receiver.action.PROBE")
                    .setComponent(new ComponentName("app.anpod.receiver", "app.anpod.receiver.BootstrapActivity"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivity(intent);
            bootstrap = "allowed";
        } catch (SecurityException denied) {
            bootstrap = "denied";
        } catch (RuntimeException error) {
            bootstrap = "error";
        }
        Log.i(TAG, "RESULT provider=" + provider + " bootstrap=" + bootstrap);
        finish();
    }
}
