package app.anpod.receiver

import androidx.compose.runtime.MutableState
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.util.ReflectionHelpers

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MainActivityStatusDecisionTest {
    @Test
    fun primaryActionDispatchesByChoiceAndActualOperation() {
        assertEquals(PrimaryAction.CONNECT_USB_DIRECT, primaryActionFor(ConnectionChoice.USB_DIRECT))
        assertEquals(PrimaryAction.SHOW_ADB_GUIDE, primaryActionFor(ConnectionChoice.USB_ADB))
        assertEquals(PrimaryAction.CONNECT_WIFI, primaryActionFor(ConnectionChoice.WIFI))
        assertEquals(PrimaryAction.CANCEL_USB_PERMISSION,
            primaryActionFor(ConnectionChoice.USB_DIRECT, usbPermissionPending = true))
        assertEquals(PrimaryAction.CANCEL_PENDING_CONNECTION,
            primaryActionFor(ConnectionChoice.WIFI, connectionPending = true))
        assertEquals(PrimaryAction.CLOSE_ADB_GUIDE,
            primaryActionFor(ConnectionChoice.USB_ADB, adbGuideVisible = true))
        assertEquals(PrimaryAction.STOP_RECEIVER,
            primaryActionFor(ConnectionChoice.USB_ADB, receiverActive = true))
        assertEquals(PrimaryAction.STOP_BACKGROUND_STANDBY,
            primaryActionFor(ConnectionChoice.WIFI, backgroundStandbyEnabled = true))
        assertEquals(PrimaryAction.DISABLED,
            primaryActionFor(ConnectionChoice.WIFI, operationStopping = true))
    }

    @Test
    fun actualTransportUsesOnlyExplicitSnapshotValues() {
        assertEquals(R.string.connection_usb_direct, actualConnectionLabel("usb-aoa"))
        assertEquals(R.string.connection_wifi, actualConnectionLabel("wireless-udp"))
        assertEquals(R.string.connection_usb_adb_short, actualConnectionLabel("usb-adb"))
        assertEquals(R.string.connection_usb_adb_short, actualConnectionLabel("usb-or-local"))
        assertEquals(R.string.actual_connection_unknown, actualConnectionLabel(null))
    }

    @Test
    fun normalUsbPermissionMessagesAreNotTreatedAsErrors() {
        assertEquals(true, isInformationalUsbPermissionMessage("pending", "pending", "cancelled"))
        assertEquals(true, isInformationalUsbPermissionMessage("cancelled", "pending", "cancelled"))
        assertEquals(false, isInformationalUsbPermissionMessage("denied", "pending", "cancelled"))
    }

    @Test
    fun adbGuideStatusStepNeverOverridesRealStatus() {
        fun showGuide(
            choice: ConnectionChoice = ConnectionChoice.USB_ADB,
            snapshotAvailable: Boolean = true,
            needsAttention: Boolean = false,
            receiverActive: Boolean = false,
            backgroundStandbyEnabled: Boolean = false,
            connectionPending: Boolean = false
        ) = shouldShowAdbGuideAsStatusStep(
            choice = choice,
            adbGuideVisible = true,
            snapshotAvailable = snapshotAvailable,
            needsAttention = needsAttention,
            receiverActive = receiverActive,
            backgroundStandbyEnabled = backgroundStandbyEnabled,
            connectionPending = connectionPending
        )

        assertEquals(true, showGuide())
        assertEquals(false, showGuide(needsAttention = true))
        assertEquals(false, showGuide(receiverActive = true))
        assertEquals(false, showGuide(backgroundStandbyEnabled = true))
        assertEquals(false, showGuide(connectionPending = true))
        assertEquals(false, showGuide(snapshotAvailable = false))
        assertEquals(false, showGuide(choice = ConnectionChoice.WIFI))
    }

    @Test
    fun connectionChoiceDefaultsToDirectAndPersistsAcrossActivityRestart() {
        val preferences = RuntimeEnvironment.getApplication()
            .getSharedPreferences("anpod_receiver", android.content.Context.MODE_PRIVATE)
        preferences.edit().clear().commit()

        val firstController = Robolectric.buildActivity(MainActivity::class.java).create()
        try {
            val firstActivity = firstController.get()
            assertEquals(ConnectionChoice.USB_DIRECT, connectionChoice(firstActivity))
            firstActivity.saveConnectionChoicePreference(ConnectionChoice.WIFI)
        } finally {
            firstController.destroy()
        }

        val restartedController = Robolectric.buildActivity(MainActivity::class.java).create()
        try {
            assertEquals(ConnectionChoice.WIFI, connectionChoice(restartedController.get()))
        } finally {
            restartedController.destroy()
            preferences.edit().clear().commit()
        }
    }

    @Test
    fun mainStatusDistinguishesConnectedPausedAndPreparedStates() {
        fun label(
            state: String,
            standby: Boolean = false,
            attention: Boolean = false,
            adbGuide: Boolean = false,
            operation: String = "idle"
        ) = chooseMainStateLabel(
            state, operation, snapshotAvailable = true, backgroundStandbyEnabled = standby,
            connectionPending = false, needsAttention = attention, adbGuideVisible = adbGuide
        )

        assertEquals(R.string.aod_connected, label("authenticated"))
        assertEquals(R.string.aod_connected, label("receiverRunning"))
        assertEquals(R.string.state_receiving, label("streaming"))
        assertEquals(R.string.paused, label("paused"))
        assertEquals(R.string.state_waiting_for_computer, label("prepared"))
        assertEquals(R.string.state_needs_attention, label("streaming", attention = true))
        assertEquals(R.string.state_waiting_for_pc_action, label("stopped", adbGuide = true))
        assertEquals(R.string.state_receiving, label("streaming", adbGuide = true))
        assertEquals(R.string.state_needs_attention, label("stopped", attention = true, adbGuide = true))
        assertEquals(R.string.stopping, label("streaming", adbGuide = true, operation = "stopping"))
        assertEquals(R.string.usb_direct_permission_pending,
            chooseMainStateLabel("stopped", "idle", true, false, false, false, usbPermissionPending = true))
    }

    private fun connectionChoice(activity: MainActivity): ConnectionChoice =
        ReflectionHelpers.getField<MutableState<ConnectionChoice>>(activity, "connectionChoice\$delegate").value

    @Test
    fun activeSessionGuidanceWinsWhenWirelessSetupIsUnavailable() {
        fun next(serviceState: String, playbackAdvancing: Boolean? = null) = chooseMainNextStep(
            serviceState, playbackAdvancing, "listening", true, "untilRevoked",
            bound = false, noNetwork = true, networkChoiceNeeded = true
        )

        assertEquals(R.string.next_step_audio_received, next("streaming", true))
        assertEquals(R.string.next_step_check_computer_playback, next("receiverRunning", false))
        assertEquals(R.string.next_step_play_on_computer, next("streaming"))
        assertEquals(R.string.next_step_resume_playback, next("paused"))
        assertEquals(R.string.next_step_play_on_computer, next("authenticated"))
    }
}
