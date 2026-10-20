package app.anpod.receiver;

import static org.junit.Assert.*;

import android.hardware.usb.UsbAccessory;
import android.hardware.usb.UsbManager;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.AsynchronousCloseException;
import java.nio.channels.ReadableByteChannel;
import java.nio.channels.WritableByteChannel;
import java.util.concurrent.TimeUnit;

import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.RuntimeEnvironment;
import org.robolectric.Shadows;
import org.robolectric.annotation.Config;
import org.robolectric.util.ReflectionHelpers;

@RunWith(RobolectricTestRunner.class)
@Config(sdk = 35)
public final class AoaAccessoryPermissionTest {
    @Test public void productIdentityIsExactAndStaleOwnerCannotOpenAccessory() throws Exception {
        UsbManager manager = RuntimeEnvironment.getApplication().getSystemService(UsbManager.class);
        UsbAccessory product = accessory("AnPod", "AnPod USB Direct");
        UsbAccessory unrelated = accessory("AnPod", "AnPod Probe Lab");
        assertTrue(AoaReceiverCoordinator.isProductAccessory(product));
        assertFalse(AoaReceiverCoordinator.isProductAccessory(unrelated));

        Shadows.shadowOf(manager).setAttachedUsbAccessory(product);
        Shadows.shadowOf(manager).grantPermission(product);
        PairingProtocol.BindingMaterial binding = binding();
        try {
            IOException error = assertThrows(IOException.class, () -> AoaReceiverCoordinator.openAccessory(
                    manager, product, binding, new Endpoint(false), "stale-owner", 7));
            assertEquals("AoaOwnerGenerationExpired", error.getMessage());
            assertBindingClosed(binding);
        } finally { binding.close(); }
    }

    @Test public void deniedUsbPermissionRejectsBeforeOpenAndConsumesBinding() throws Exception {
        UsbManager manager = RuntimeEnvironment.getApplication().getSystemService(UsbManager.class);
        UsbAccessory product = accessory("AnPod", "AnPod USB Direct");
        Shadows.shadowOf(manager).setAttachedUsbAccessory(product);
        PairingProtocol.BindingMaterial binding = binding();
        try {
            IOException error = assertThrows(IOException.class, () -> AoaReceiverCoordinator.openAccessory(
                    manager, product, binding, new Endpoint(true), "denied-owner", 10));
            assertEquals("UsbAccessoryPermissionOrAttachmentLost", error.getMessage());
            assertBindingClosed(binding);
        } finally { binding.close(); }
    }

    @Test public void activeOwnerRejectsSecondOpenAndConsumesItsBinding() throws Exception {
        PairingProtocol.BindingMaterial activeBinding = binding();
        AoaReceiverCoordinator active = AoaReceiverCoordinator.start(new AoaAccessoryIo(
                new BlockingInput(), new Sink(), () -> { }), activeBinding,
                new Endpoint(true), "active-owner", 8, 5000);
        PairingProtocol.BindingMaterial secondBinding = binding();
        UsbManager manager = RuntimeEnvironment.getApplication().getSystemService(UsbManager.class);
        try {
            IOException error = assertThrows(IOException.class, () -> AoaReceiverCoordinator.openAccessory(
                    manager, accessory("AnPod", "AnPod USB Direct"), secondBinding,
                    new Endpoint(true), "second-owner", 9));
            assertEquals("AoaSessionCleanupPending", error.getMessage());
            assertBindingClosed(secondBinding);
        } finally {
            active.cancel("test-finished");
            assertTrue(active.awaitTermination(2, TimeUnit.SECONDS));
            secondBinding.close();
            activeBinding.close();
        }
    }

    private static void assertBindingClosed(PairingProtocol.BindingMaterial binding) {
        assertThrows(IllegalStateException.class, binding::encodedRecord);
    }

    private static PairingProtocol.BindingMaterial binding() {
        byte[] host = new byte[16], receiver = new byte[16], secret = new byte[32];
        for (int i = 0; i < secret.length; i++) secret[i] = (byte) i;
        return new PairingProtocol.BindingMaterial(host, receiver, secret);
    }

    private static UsbAccessory accessory(String manufacturer, String model) {
        return ReflectionHelpers.callConstructor(UsbAccessory.class,
                ReflectionHelpers.ClassParameter.from(String.class, manufacturer),
                ReflectionHelpers.ClassParameter.from(String.class, model),
                ReflectionHelpers.ClassParameter.from(String.class, "USB Direct"),
                ReflectionHelpers.ClassParameter.from(String.class, "1"),
                ReflectionHelpers.ClassParameter.from(String.class, ""),
                ReflectionHelpers.ClassParameter.from(String.class, "test-serial"));
    }

    private static final class Endpoint implements AoaReceiverCoordinator.PlaybackEndpoint {
        private final boolean current;
        Endpoint(boolean current) { this.current = current; }
        @Override public boolean isCurrent(String ownerId, long generation) { return current; }
        @Override public boolean supportsAudio(UdpPacketDecoder.AudioFormatV2 format) { return true; }
        @Override public void start(AoaSessionContext context) { }
        @Override public void pause() { }
        @Override public void resume() { }
        @Override public void stop() { }
        @Override public boolean streaming() { return false; }
        @Override public byte[] playbackProgressPayloadV2() { return new byte[78]; }
    }

    private static final class BlockingInput implements ReadableByteChannel {
        private boolean open = true;
        @Override public synchronized int read(ByteBuffer destination) throws IOException {
            while (open) try { wait(); }
            catch (InterruptedException error) {
                Thread.currentThread().interrupt();
                throw new IOException("cancelled", error);
            }
            throw new AsynchronousCloseException();
        }
        @Override public synchronized boolean isOpen() { return open; }
        @Override public synchronized void close() { open = false; notifyAll(); }
    }

    private static final class Sink implements WritableByteChannel {
        private boolean open = true;
        @Override public int write(ByteBuffer source) throws IOException {
            if (!open) throw new AsynchronousCloseException();
            int count = source.remaining();
            source.position(source.limit());
            return count;
        }
        @Override public synchronized boolean isOpen() { return open; }
        @Override public synchronized void close() { open = false; }
    }
}
