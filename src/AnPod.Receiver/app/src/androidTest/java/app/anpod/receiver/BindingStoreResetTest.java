package app.anpod.receiver;

import android.content.Context;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.test.InstrumentationTestCase;
import android.app.UiAutomation;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/** Device-side checks for conditional binding removal; no test clears the installed binding. */
public final class BindingStoreResetTest extends InstrumentationTestCase {
    private static final Uri BINDING_URI = Uri.parse("content://app.anpod.receiver.state/binding");
    private Context context;

    @Override
    protected void setUp() throws Exception {
        super.setUp();
        context = getInstrumentation().getTargetContext();
    }

    public void testBindingStoreCompareAndDeleteIsIsolatedAndIdempotent() throws Exception {
        File directory = new File(context.getCacheDir(), "binding-reset-" + UUID.randomUUID());
        assertTrue(directory.mkdir());
        BindingStore store = new BindingStore(directory);
        PairingProtocol.BindingMaterial original = PairingProtocol.BindingMaterial.generate();
        PairingProtocol.BindingMaterial wrong = PairingProtocol.BindingMaterial.generate();
        try {
            store.createOrConfirm(original);
            String expected = store.status().commitment;
            store.saveUntilRevoked(expected);
            try {
                store.removeIfMatches(hex(wrong.commitment()));
                fail("a different expected commitment must not delete the binding");
            } catch (BindingStore.ConflictException expectedConflict) { }
            assertEquals(expected, store.status().commitment);
            assertTrue("a failed compare must retain the matching grant", store.hasUntilRevoked(expected));
            assertEquals(BindingStore.ResetResult.REMOVED, store.removeIfMatches(expected));
            assertEquals("empty", store.status().state);
            assertFalse("binding reset must clear its durable authorization grant", store.hasUntilRevoked(expected));
            assertEquals(BindingStore.ResetResult.ALREADY_EMPTY, store.removeIfMatches(null));
            try {
                store.removeIfMatches(expected);
                fail("a saved expectation must not match an empty store");
            } catch (BindingStore.ConflictException expectedConflict) { }
        } finally {
            original.close();
            wrong.close();
            File[] files = directory.listFiles();
            if (files != null) for (File file : files) file.delete();
            directory.delete();
        }
    }

    public void testResetCapabilityAndExpectedEmptyAreNonDestructive() throws Exception {
        String before = shell("content query --uri " + BINDING_URI
                + " --projection bindingState:bindingCommitment:bindingFailure");
        String capability = shell("content query --uri " + BINDING_URI
                + " --projection bindingResetVersion");
        assertTrue("conditional reset capability must be explicit", capability.contains("bindingResetVersion=1"));

        // expectedEmpty can only assert an empty store. If one is present, the provider must retain it.
        shell("content delete --uri " + BINDING_URI + " --where expectedEmpty");
        String after = shell("content query --uri " + BINDING_URI
                + " --projection bindingState:bindingCommitment:bindingFailure");
        assertEquals("the non-destructive expectedEmpty assertion must preserve current state", before, after);
    }

    public void testApplicationUidCannotDeleteThroughTheResetProvider() throws Exception {
        String before = shell("content query --uri " + BINDING_URI
                + " --projection bindingState:bindingCommitment:bindingFailure");
        try {
            context.getContentResolver().delete(BINDING_URI, "expectedEmpty", null);
            fail("the ordinary app UID must not invoke the shell-only conditional delete");
        } catch (SecurityException expected) { }
        String after = shell("content query --uri " + BINDING_URI
                + " --projection bindingState:bindingCommitment:bindingFailure");
        assertEquals("rejected app-UID delete must preserve binding state", before, after);
    }

    public void testBusyReceiverRefusesEvenAnExpectedEmptyReset() throws Exception {
        String before = shell("content query --uri " + BINDING_URI
                + " --projection bindingState:bindingCommitment:bindingFailure");
        boolean oldAudioActive = ReceiverService.audioActive;
        String oldNetworkState = ReceiverService.networkAudioState;
        try {
            ReceiverService.audioActive = true;
            String result = shell("content delete --uri " + BINDING_URI + " --where expectedEmpty");
            assertTrue("active playback must be reported as busy", result.contains("receiver_busy"));
        } finally {
            ReceiverService.audioActive = oldAudioActive;
            ReceiverService.networkAudioState = oldNetworkState;
        }
        String after = shell("content query --uri " + BINDING_URI
                + " --projection bindingState:bindingCommitment:bindingFailure");
        assertEquals(before, after);
    }

    private String shell(String command) throws IOException {
        UiAutomation automation = getInstrumentation().getUiAutomation();
        try (ParcelFileDescriptor output = automation.executeShellCommand(command);
                FileInputStream input = new FileInputStream(output.getFileDescriptor());
                ByteArrayOutputStream bytes = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[1024];
            int count;
            while ((count = input.read(buffer)) >= 0) bytes.write(buffer, 0, count);
            return new String(bytes.toByteArray(), StandardCharsets.UTF_8);
        }
    }

    private static String hex(byte[] value) {
        char[] digits = "0123456789abcdef".toCharArray();
        char[] output = new char[value.length * 2];
        for (int i = 0; i < value.length; i++) {
            output[i * 2] = digits[(value[i] >>> 4) & 0x0f];
            output[i * 2 + 1] = digits[value[i] & 0x0f];
        }
        return new String(output);
    }
}
