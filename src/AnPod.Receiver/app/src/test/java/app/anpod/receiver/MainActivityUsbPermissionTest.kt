package app.anpod.receiver

import android.hardware.usb.UsbAccessory
import java.util.concurrent.atomic.AtomicLong
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.util.ReflectionHelpers

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MainActivityUsbPermissionTest {
    @Test
    fun permissionResultFromSupersededReceiverAttemptIsRejected() {
        val controller = Robolectric.buildActivity(MainActivity::class.java).create()
        val activity = controller.get()
        try {
            val generation = 18L
            val accessory = ReflectionHelpers.callConstructor(
                UsbAccessory::class.java,
                ReflectionHelpers.ClassParameter.from(String::class.java, "AnPod"),
                ReflectionHelpers.ClassParameter.from(String::class.java, "AnPod USB Direct"),
                ReflectionHelpers.ClassParameter.from(String::class.java, "USB Direct"),
                ReflectionHelpers.ClassParameter.from(String::class.java, "1"),
                ReflectionHelpers.ClassParameter.from(String::class.java, ""),
                ReflectionHelpers.ClassParameter.from(String::class.java, "test-serial")
            )
            ReflectionHelpers.setField(activity, "pendingUsbPermission",
                MainActivity.PendingUsbPermission(generation, accessory, "permission-action"))
            ReflectionHelpers.setField(activity, "activityStarted", true)
            ReflectionHelpers.setField(activity, "bound", true)
            val discovery = ReflectionHelpers.getField<AtomicLong>(activity, "discoveryGeneration")
            discovery.set(generation)

            discovery.incrementAndGet() // A different Receiver entry invalidates the pending USB callback.
            val message = ReflectionHelpers.getField<androidx.compose.runtime.MutableState<String>>(
                activity, "localMessage\$delegate")
            message.value = "stale-callback-sentinel"

            val complete = MainActivity::class.java.getDeclaredMethod(
                "completeUsbPermission",
                MainActivity.PendingUsbPermission::class.java,
                Boolean::class.javaPrimitiveType,
                UsbAccessory::class.java
            )
            complete.isAccessible = true
            complete.invoke(activity,
                ReflectionHelpers.getField<MainActivity.PendingUsbPermission>(activity, "pendingUsbPermission"),
                false,
                accessory)

            assertEquals("stale-callback-sentinel", message.value)
            assertEquals(null, ReflectionHelpers.getField<MainActivity.PendingUsbPermission?>(activity, "pendingUsbPermission"))
        } finally {
            controller.destroy()
        }
    }
}
