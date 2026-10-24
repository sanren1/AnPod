package app.anpod.receiver

import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import androidx.compose.runtime.MutableState
import java.lang.reflect.Proxy
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows
import org.robolectric.annotation.Config
import org.robolectric.util.ReflectionHelpers

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MainActivityUsbSerialPermissionTest {
    @Test
    fun accessoryAttachOpensManagementEntryWithoutRequestingPermissionOrPlayback() {
        val app = RuntimeEnvironment.getApplication()
        val attach = Intent(UsbManager.ACTION_USB_ACCESSORY_ATTACHED)
        val matches = app.packageManager.queryIntentActivities(attach, 0)
        assertTrue(matches.any { it.activityInfo.name == MainActivity::class.java.name })
        val info = app.packageManager.getActivityInfo(
            ComponentName(app, MainActivity::class.java), PackageManager.GET_META_DATA)
        assertTrue(info.metaData.containsKey(UsbManager.ACTION_USB_ACCESSORY_ATTACHED))

        val controller = Robolectric.buildActivity(MainActivity::class.java, attach).create()
        val activity = controller.get()
        try {
            assertNull(ReflectionHelpers.getField<MainActivity.PendingUsbPermission?>(
                activity, "pendingUsbPermission"))
            assertFalse(ReflectionHelpers.getField<MutableState<Boolean>>(
                activity, "usbPermissionPending\$delegate").value)
            assertNull(ReceiverService.currentAudio)
        } finally {
            controller.destroy()
        }
    }

    @Test
    fun repeatedAccessoryRefreshDoesNotReadProtectedSerial() {
        val app = RuntimeEnvironment.getApplication()
        val manager = app.getSystemService(UsbManager::class.java)
        Shadows.shadowOf(app.packageManager)
            .setSystemFeature(PackageManager.FEATURE_USB_ACCESSORY, true)
        val first = unauthorizedAccessory()
        val second = unauthorizedAccessory()
        val controller = Robolectric.buildActivity(MainActivity::class.java).create()
        val activity = controller.get()
        try {
            val refresh = MainActivity::class.java.getDeclaredMethod("refreshUsbAccessory")
            refresh.isAccessible = true

            Shadows.shadowOf(manager).setAttachedUsbAccessory(first.accessory)
            refresh.invoke(activity)
            Shadows.shadowOf(manager).setAttachedUsbAccessory(second.accessory)
            refresh.invoke(activity)

            val available = ReflectionHelpers.getField<MutableState<Boolean>>(
                activity, "usbDirectAvailable\$delegate")
            assertTrue(available.value)
            assertEquals(0, first.serialReads.get())
            assertEquals(0, second.serialReads.get())
        } finally {
            controller.destroy()
        }
    }

    @Test
    fun permissionResultWithoutCurrentPermissionDoesNotCompareAccessories() {
        val app = RuntimeEnvironment.getApplication()
        val accessory = unauthorizedAccessory()
        Shadows.shadowOf(app.getSystemService(UsbManager::class.java))
            .setAttachedUsbAccessory(accessory.accessory)
        val controller = Robolectric.buildActivity(MainActivity::class.java).create()
        val activity = controller.get()
        try {
            val generation = 23L
            val attempt = MainActivity.PendingUsbPermission(generation, accessory.accessory, "permission-action")
            ReflectionHelpers.setField(activity, "pendingUsbPermission", attempt)
            ReflectionHelpers.setField(activity, "activityStarted", true)
            ReflectionHelpers.setField(activity, "bound", true)
            ReflectionHelpers.getField<AtomicLong>(activity, "discoveryGeneration").set(generation)

            val complete = MainActivity::class.java.getDeclaredMethod(
                "completeUsbPermission",
                MainActivity.PendingUsbPermission::class.java,
                Boolean::class.javaPrimitiveType,
                UsbAccessory::class.java
            )
            complete.isAccessible = true
            complete.invoke(activity, attempt, true, accessory.accessory)

            val message = ReflectionHelpers.getField<MutableState<String>>(
                activity, "localMessage\$delegate")
            assertEquals(activity.getString(R.string.usb_direct_detached_or_permission_lost), message.value)
            assertEquals(0, accessory.serialReads.get())
            assertFalse(ReflectionHelpers.getField<MutableState<Boolean>>(
                activity, "usbPermissionPending\$delegate").value)
        } finally {
            controller.destroy()
        }
    }

    private fun unauthorizedAccessory(): ProtectedAccessory {
        val serialReads = AtomicInteger()
        val readerInterface = Class.forName("android.hardware.usb.IUsbSerialReader")
        val reader = Proxy.newProxyInstance(
            readerInterface.classLoader,
            arrayOf(readerInterface)
        ) { proxy, method, args ->
            when (method.name) {
                "getSerial" -> {
                    serialReads.incrementAndGet()
                    throw SecurityException("USB accessory permission required")
                }
                "asBinder" -> null
                "hashCode" -> System.identityHashCode(proxy)
                "equals" -> proxy === args?.firstOrNull()
                "toString" -> "ProtectedUsbSerialReader"
                else -> null
            }
        }
        val constructor = UsbAccessory::class.java.declaredConstructors.first {
            it.parameterTypes.lastOrNull() == readerInterface
        }
        constructor.isAccessible = true
        val accessory = constructor.newInstance(
            "AnPod", "AnPod USB Direct", "USB Direct", "1", "", reader
        ) as UsbAccessory
        return ProtectedAccessory(accessory, serialReads)
    }

    private data class ProtectedAccessory(val accessory: UsbAccessory, val serialReads: AtomicInteger)
}
