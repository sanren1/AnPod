package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import android.app.Notification;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;

import androidx.test.core.app.ApplicationProvider;

import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.annotation.Config;

@RunWith(RobolectricTestRunner.class)
@Config(sdk = 34)
public final class ReceiverNotificationsTest {
    private Context context;

    @Before
    public void setUp() {
        context = ApplicationProvider.getApplicationContext();
    }

    @Test
    public void authorizationExposesThreeDecisionsAndADeleteIntent() {
        Notification notification = ReceiverNotifications.authorization(context, "test-channel",
                pending(1), pending(2), pending(3), pending(4));

        assertEquals(3, notification.actions.length);
        assertNotNull(notification.deleteIntent);
        assertTrue((notification.flags & Notification.FLAG_AUTO_CANCEL) != 0);
    }

    @Test
    public void playbackOnlyAddsStopActionWhenProvided() {
        Notification withStop = ReceiverNotifications.playback(context, "test-channel", "text",
                pending(1), pending(2), null);
        Notification withoutStop = ReceiverNotifications.playback(context, "test-channel", "text",
                pending(1), null, null);

        assertEquals(1, withStop.actions.length);
        assertNull(withoutStop.actions);
        assertTrue((withoutStop.flags & Notification.FLAG_ONGOING_EVENT) != 0);
    }

    private PendingIntent pending(int code) {
        return PendingIntent.getActivity(context, code, new Intent(),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }
}
