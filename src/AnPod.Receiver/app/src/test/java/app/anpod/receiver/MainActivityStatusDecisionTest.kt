package app.anpod.receiver

import androidx.compose.runtime.MutableState
import android.content.Intent
import android.os.Bundle
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
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
class MainActivityStatusDecisionTest {
    @Test
    fun recoveryClickIsConsumedOnceWithoutReplacingAnActiveOwner() {
        val app = RuntimeEnvironment.getApplication()
        val preferences = app.getSharedPreferences("anpod_receiver", android.content.Context.MODE_PRIVATE)
        preferences.edit().clear().commit()
        val binding = PairingProtocol.BindingMaterial.generate()
        val store = BindingStore(app.filesDir)
        val service = Robolectric.buildService(ReceiverService::class.java).create().get()
        var controller: org.robolectric.android.controller.ActivityController<RecoveryActivity>? = null
        try {
            store.createOrConfirm(binding)
            val commitment = store.status().commitment
            assertTrue(BackgroundStandbyRecovery.save(app, commitment, "standby-owner"))
            Shadows.shadowOf(app).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
            assertTrue(service.uiOperation.begin("active-owner"))
            ReflectionHelpers.setField(service, "networkAudioState", "starting")
            val intent = Intent(app, RecoveryActivity::class.java)
                .putExtra(RecoveryActivity.EXTRA_BINDING_COMMITMENT, commitment)
                .putExtra(RecoveryActivity.EXTRA_OPERATION_ID, "standby-owner")
            controller = Robolectric.buildActivity(RecoveryActivity::class.java, intent).create()
            val activity = controller.get()
            val binder = ReceiverUiBinder(service)
            ReflectionHelpers.getField<MutableState<ReceiverUiBinder?>>(activity,
                "receiver${'$'}delegate").value = binder
            ReflectionHelpers.getField<MutableState<UiSnapshot?>>(activity,
                "snapshot${'$'}delegate").value = binder.snapshot()
            ReflectionHelpers.setField(activity, "bound", true)
            ReflectionHelpers.setField(activity, "activityStarted", true)
            assertNotNull(ReflectionHelpers.getField<MutableState<ReceiverUiBinder?>>(activity,
                "receiver${'$'}delegate").value)
            ReflectionHelpers.callInstanceMethod<Unit>(activity, "maybeStartBootRecovery")

            assertTrue(ReflectionHelpers.getField<Boolean>(activity, "recoveryClickConsumed"))
            assertEquals("active-owner", service.uiOperation.ownerId())
            assertEquals(ReceiverUiOperation.STARTING, service.uiOperation.phase())
            ReflectionHelpers.callInstanceMethod<Unit>(activity, "maybeStartBootRecovery")
            assertEquals("active-owner", service.uiOperation.ownerId())
        } finally {
            if (controller != null) {
                ReflectionHelpers.setField(controller.get(), "bound", false)
                ReflectionHelpers.setField(controller.get(), "activityStarted", false)
            }
            controller?.destroy()
            BackgroundStandbyRecovery.clear(app)
            try { store.removeIfMatches(store.status().commitment) } catch (_: Exception) { }
            binding.close()
            service.onDestroy()
            preferences.edit().clear().commit()
        }
    }

    @Test
    fun publicMainActivityIgnoresForgedRecoveryIntentAndPrivateActivityKeepsConsumedClickAcrossRotation() {
        val forged = Intent(RuntimeEnvironment.getApplication(), MainActivity::class.java)
            .setAction("app.anpod.receiver.action.RECOVER_STANDBY")
            .putExtra(RecoveryActivity.EXTRA_BINDING_COMMITMENT, "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .putExtra(RecoveryActivity.EXTRA_OPERATION_ID, "forged-owner")
        val publicController = Robolectric.buildActivity(MainActivity::class.java, forged).create()
        try {
            assertFalse(ReflectionHelpers.getField(publicController.get(), "recoveryClickPending"))
        } finally {
            publicController.destroy()
        }

        val trustedIntent = Intent(RuntimeEnvironment.getApplication(), RecoveryActivity::class.java)
            .putExtra(RecoveryActivity.EXTRA_BINDING_COMMITMENT, "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .putExtra(RecoveryActivity.EXTRA_OPERATION_ID, "trusted-owner")
        val firstController = Robolectric.buildActivity(RecoveryActivity::class.java, trustedIntent).create()
        val state = Bundle()
        try {
            ReflectionHelpers.setField(firstController.get(), "recoveryClickConsumed", true)
            firstController.saveInstanceState(state)
        } finally {
            firstController.destroy()
        }
        val recreated = Robolectric.buildActivity(RecoveryActivity::class.java, trustedIntent).create(state)
        try {
            assertTrue(ReflectionHelpers.getField(recreated.get(), "recoveryClickConsumed"))
            assertFalse(ReflectionHelpers.getField(recreated.get(), "recoveryClickPending"))
        } finally {
            recreated.destroy()
        }
    }

    @Test
    fun primaryActionDispatchesByChoiceAndActualOperation() {
        assertEquals(PrimaryAction.CONNECT_USB_DIRECT, primaryActionFor(ConnectionChoice.USB_DIRECT))
        assertEquals(PrimaryAction.WAIT_ADB, primaryActionFor(ConnectionChoice.USB_ADB))
        assertEquals(PrimaryAction.CONNECT_WIFI, primaryActionFor(ConnectionChoice.WIFI))
        assertEquals(PrimaryAction.CANCEL_USB_PERMISSION,
            primaryActionFor(ConnectionChoice.USB_DIRECT, usbPermissionPending = true))
        assertEquals(PrimaryAction.CANCEL_PENDING_CONNECTION,
            primaryActionFor(ConnectionChoice.WIFI, connectionPending = true))
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
        assertEquals(R.string.connection_wifi_adb, actualConnectionLabel("wifi-adb"))
        assertEquals(R.string.usb_or_local_connection, actualConnectionLabel("usb-or-local"))
        assertEquals(R.string.actual_connection_unknown, actualConnectionLabel(null))
        assertEquals(ConnectionChoice.USB_DIRECT, connectionChoiceForActualMethod("usb-aoa"))
        assertEquals(ConnectionChoice.USB_ADB, connectionChoiceForActualMethod("usb-adb"))
        assertEquals(ConnectionChoice.WIFI, connectionChoiceForActualMethod("wifi-adb"))
        assertEquals(ConnectionChoice.WIFI, connectionChoiceForActualMethod("wireless-udp"))
        assertEquals(null, connectionChoiceForActualMethod("usb-or-local"))
        assertEquals(null, connectionChoiceForActualMethod("unknown"))
        assertEquals(ConnectionChoice.USB_DIRECT,
            displayedConnectionChoice(ConnectionChoice.WIFI, "usb-aoa", selectionLocked = true))
        assertEquals(ConnectionChoice.WIFI,
            displayedConnectionChoice(ConnectionChoice.WIFI, "usb-aoa", selectionLocked = false))
        assertEquals(null,
            displayedConnectionChoice(ConnectionChoice.WIFI, "unknown", selectionLocked = true))
        val activeWifiChoice = displayedConnectionChoice(ConnectionChoice.USB_DIRECT, "wireless-udp", selectionLocked = true)
        assertEquals(ConnectionChoice.WIFI, activeWifiChoice)
        assertEquals(false, shouldShowUsbDirectGuidance(activeWifiChoice, selectionLocked = true))
        val activeAdbChoice = displayedConnectionChoice(ConnectionChoice.USB_DIRECT, "usb-adb", selectionLocked = true)
        assertEquals(ConnectionChoice.USB_ADB, activeAdbChoice)
        assertEquals(false, shouldShowUsbDirectGuidance(activeAdbChoice, selectionLocked = true))
        assertEquals(true, shouldShowUsbDirectGuidance(ConnectionChoice.USB_DIRECT, selectionLocked = false))
    }

    @Test
    fun normalUsbPermissionMessagesAreNotTreatedAsErrors() {
        assertEquals(true, isInformationalUsbPermissionMessage("pending", "pending", "cancelled"))
        assertEquals(true, isInformationalUsbPermissionMessage("cancelled", "pending", "cancelled"))
        assertEquals(false, isInformationalUsbPermissionMessage("denied", "pending", "cancelled"))
    }

    @Test
    fun adbWaitActionKeepsHigherPriorityCancelAndStopActions() {
        assertEquals(PrimaryAction.WAIT_ADB, primaryActionFor(ConnectionChoice.USB_ADB))
        assertEquals(PrimaryAction.CANCEL_PENDING_CONNECTION,
            primaryActionFor(ConnectionChoice.USB_ADB, connectionPending = true))
        assertEquals(PrimaryAction.STOP_RECEIVER,
            primaryActionFor(ConnectionChoice.USB_ADB, receiverActive = true))
        assertEquals(PrimaryAction.STOP_BACKGROUND_STANDBY,
            primaryActionFor(ConnectionChoice.USB_ADB, backgroundStandbyEnabled = true))
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
            advancing: Boolean? = null,
            operation: String = "idle"
        ) = chooseMainStateLabel(
            state, operation, snapshotAvailable = true, backgroundStandbyEnabled = standby,
            connectionPending = false, needsAttention = attention, playbackAdvancing = advancing
        )

        assertEquals(R.string.state_connected_waiting_for_audio, label("authenticated"))
        assertEquals(R.string.aod_connected, label("receiverRunning"))
        assertEquals(R.string.aod_connected, label("receiverRunning", advancing = false))
        listOf("usb-aoa" to ConnectionChoice.USB_DIRECT, "usb-adb" to ConnectionChoice.USB_ADB)
            .forEach { (method, expectedChoice) ->
                assertEquals(expectedChoice, connectionChoiceForActualMethod(method))
                assertEquals(R.string.state_playing_computer_audio,
                    chooseMainStateLabel("receiverRunning", "idle", true, false, false, false,
                        playbackAdvancing = true))
            }
        assertEquals(R.string.state_receiving, label("streaming"))
        assertEquals(R.string.state_receiving, label("streaming", advancing = false))
        assertEquals(R.string.state_playing_computer_audio, label("streaming", advancing = true))
        assertEquals(R.string.paused, label("paused"))
        assertEquals(R.string.state_waiting_for_computer, label("prepared"))
        assertEquals(R.string.state_needs_attention, label("streaming", attention = true))
        assertEquals(R.string.state_not_connected, label("stopped"))
        assertEquals(R.string.stopping, label("streaming", operation = "stopping"))
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
