package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.assertFalse;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.ContextWrapper;
import android.content.Intent;
import androidx.test.core.app.ApplicationProvider;

import java.io.File;
import java.util.Arrays;

import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.RuntimeEnvironment;
import org.robolectric.Shadows;
import org.robolectric.annotation.Config;

@RunWith(RobolectricTestRunner.class)
@Config(sdk = 34)
public final class ReceiverNotificationsTest {
    private Context context;

    @Before
    public void setUp() {
        context = ApplicationProvider.getApplicationContext();
        BackgroundStandbyRecovery.clear(context);
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

    @Test
    public void standbyRecoveryNotificationIsDismissibleAndNotOngoing() {
        Notification notification = ReceiverNotifications.standbyRecovery(context, "test-channel", pending(5));

        assertTrue((notification.flags & Notification.FLAG_AUTO_CANCEL) != 0);
        assertFalse((notification.flags & Notification.FLAG_ONGOING_EVENT) != 0);
        assertNotNull(notification.contentIntent);
    }

    @Test
    public void bootPostsOnlyForMatchingSavedBindingAndDoesNotStartService() throws Exception {
        File isolatedFiles = isolatedFiles();
        try {
            Context isolated = withFilesDirectory(context, isolatedFiles);
            BindingStore.Status binding = createBinding(isolated);
            assertTrue(BackgroundStandbyRecovery.save(isolated, binding.commitment, "successful-standby-owner"));
            Shadows.shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(
                    android.Manifest.permission.POST_NOTIFICATIONS);
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            manager.createNotificationChannel(new NotificationChannel("receiver-management", "Receiver",
                    NotificationManager.IMPORTANCE_LOW));

            new BootCompletedReceiver().onReceive(isolated, new Intent(Intent.ACTION_BOOT_COMPLETED));

            assertEquals(1, manager.getActiveNotifications().length);
            java.lang.reflect.Field activeService = ReceiverService.class.getDeclaredField("activeService");
            activeService.setAccessible(true);
            assertNull("boot notification must not create the Receiver service", activeService.get(null));
        } finally {
            deleteTree(isolatedFiles);
        }
    }

    @Test
    public void bootSuppressesAndClearsRecoveryWhenBindingChanges() throws Exception {
        File isolatedFiles = isolatedFiles();
        try {
            Context isolated = withFilesDirectory(context, isolatedFiles);
            BindingStore.Status binding = createBinding(isolated);
            String oldCommitment = binding.commitment;
            assertTrue(BackgroundStandbyRecovery.save(isolated, oldCommitment, "old-owner"));
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            manager.createNotificationChannel(new NotificationChannel("receiver-management", "Receiver",
                    NotificationManager.IMPORTANCE_LOW));
            byte[] hostId = new byte[16], receiverId = new byte[16], secret = new byte[32];
            Arrays.fill(hostId, (byte) 4);
            Arrays.fill(receiverId, (byte) 5);
            Arrays.fill(secret, (byte) 6);
            new BindingStore(isolated.getFilesDir()).removeIfMatches(oldCommitment);
            try (PairingProtocol.BindingMaterial replacement = new PairingProtocol.BindingMaterial(hostId, receiverId, secret)) {
                new BindingStore(isolated.getFilesDir()).createOrConfirm(replacement);
            }

            new BootCompletedReceiver().onReceive(isolated, new Intent(Intent.ACTION_BOOT_COMPLETED));

            assertEquals(0, manager.getActiveNotifications().length);
            assertEquals("", BackgroundStandbyRecovery.bindingCommitment(isolated));
            assertEquals("", BackgroundStandbyRecovery.operationId(isolated));
        } finally {
            deleteTree(isolatedFiles);
        }
    }

    @Test
    public void clearCancelsOldImmutableNotificationPendingIntent() throws Exception {
        String commitment = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        String operationId = "old-standby-owner";
        assertTrue(BackgroundStandbyRecovery.save(context, commitment, operationId));
        PendingIntent old = PendingIntent.getActivity(context, BackgroundStandbyRecovery.NOTIFICATION_ID,
                BackgroundStandbyRecovery.notificationIntent(context, commitment, operationId),
                PendingIntent.FLAG_CANCEL_CURRENT | PendingIntent.FLAG_ONE_SHOT | PendingIntent.FLAG_IMMUTABLE);

        BackgroundStandbyRecovery.clear(context);

        assertFalse(BackgroundStandbyRecovery.matches(context, commitment, operationId, commitment));
        try {
            old.send();
            throw new AssertionError("cleared recovery PendingIntent should be cancelled");
        } catch (PendingIntent.CanceledException expected) { }
    }

    @Test
    public void unavailableNotificationChannelSuppressesBootPrompt() throws Exception {
        File isolatedFiles = isolatedFiles();
        try {
            Context isolated = withFilesDirectory(context, isolatedFiles);
            BindingStore.Status binding = createBinding(isolated);
            assertTrue(BackgroundStandbyRecovery.save(isolated, binding.commitment, "standby-owner"));
            context.getSystemService(NotificationManager.class).deleteNotificationChannel("receiver-management");

            new BootCompletedReceiver().onReceive(isolated, new Intent(Intent.ACTION_BOOT_COMPLETED));

            assertEquals(0, context.getSystemService(NotificationManager.class).getActiveNotifications().length);
        } finally {
            deleteTree(isolatedFiles);
        }
    }

    private PendingIntent pending(int code) {
        return PendingIntent.getActivity(context, code, new Intent(),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    private BindingStore.Status createBinding(Context target) throws Exception {
        byte[] host = new byte[16], receiver = new byte[16], secret = new byte[32];
        Arrays.fill(host, (byte) 1);
        Arrays.fill(receiver, (byte) 2);
        Arrays.fill(secret, (byte) 3);
        try (PairingProtocol.BindingMaterial material = new PairingProtocol.BindingMaterial(host, receiver, secret)) {
            BindingStore store = new BindingStore(target.getFilesDir());
            store.createOrConfirm(material);
            return store.status();
        }
    }

    private File isolatedFiles() {
        File directory = new File(context.getCacheDir(), "boot-recovery-test-" + java.util.UUID.randomUUID());
        assertTrue(directory.mkdirs());
        return directory;
    }

    private Context withFilesDirectory(Context base, File directory) {
        return new ContextWrapper(base) {
            @Override public File getFilesDir() { return directory; }
        };
    }

    private void deleteTree(File directory) {
        File[] children = directory.listFiles();
        if (children != null) for (File child : children) {
            if (child.isDirectory()) deleteTree(child);
            else assertTrue(child.delete());
        }
        assertTrue(directory.delete());
    }
}
