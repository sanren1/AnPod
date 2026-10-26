package app.anpod.receiver

import android.Manifest
import android.app.PendingIntent
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.provider.Settings
import android.net.Uri
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.lifecycle.Lifecycle
import java.util.concurrent.atomic.AtomicLong
import java.util.UUID

private data class BackgroundTrustTarget(
    val operationId: String,
    val operationGeneration: Long,
    val bindingFingerprint: String
)

internal enum class ConnectionChoice(val preferenceValue: String) {
    USB_DIRECT("usb-direct"), USB_ADB("usb-adb"), WIFI("wifi");

    companion object {
        fun fromPreference(value: String?) = values().firstOrNull { it.preferenceValue == value } ?: USB_DIRECT
    }
}

internal enum class PrimaryAction {
    CONNECT_USB_DIRECT, WAIT_ADB, CONNECT_WIFI, CANCEL_USB_PERMISSION,
    CANCEL_PENDING_CONNECTION, STOP_RECEIVER, STOP_BACKGROUND_STANDBY, DISABLED
}

internal fun primaryActionFor(
    choice: ConnectionChoice,
    receiverActive: Boolean = false,
    operationStopping: Boolean = false,
    usbPermissionPending: Boolean = false,
    connectionPending: Boolean = false,
    backgroundStandbyEnabled: Boolean = false
): PrimaryAction = when {
    operationStopping -> PrimaryAction.DISABLED
    receiverActive -> PrimaryAction.STOP_RECEIVER
    backgroundStandbyEnabled -> PrimaryAction.STOP_BACKGROUND_STANDBY
    usbPermissionPending -> PrimaryAction.CANCEL_USB_PERMISSION
    connectionPending -> PrimaryAction.CANCEL_PENDING_CONNECTION
    choice == ConnectionChoice.USB_DIRECT -> PrimaryAction.CONNECT_USB_DIRECT
    choice == ConnectionChoice.USB_ADB -> PrimaryAction.WAIT_ADB
    else -> PrimaryAction.CONNECT_WIFI
}

class MainActivity : ComponentActivity() {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var receiver by mutableStateOf<ReceiverUiBinder?>(null)
    private var snapshot by mutableStateOf<UiSnapshot?>(null)
    private val interfaces = mutableStateListOf<UiInterface>()
    private var selectedAddress by mutableStateOf("")
    private var selectedPrefix by mutableStateOf(0)
    private var localMessage by mutableStateOf("")
    private var bound = false
    @Volatile private var activityStarted = false
    private val discoveryGeneration = AtomicLong()
    private data class PendingConnection(
        val generation: Long,
        val previousOperationId: String,
        val backgroundStandby: Boolean,
        val operationId: String? = null
    )
    private var pendingConnection by mutableStateOf<PendingConnection?>(null)
    private var aodEnabled by mutableStateOf(false)
    private var aodDisplaySettings by mutableStateOf(AodDisplaySettings())
    private var aodShownForSession: String? = null
    private var outputEngine by mutableStateOf("oboeExperimental")
    private var connectionChoice by mutableStateOf(ConnectionChoice.USB_DIRECT)
    private var usbDirectAvailable by mutableStateOf(false)
    private var usbPermissionPending by mutableStateOf(false)
    internal data class PendingUsbPermission(val generation: Long, val accessory: UsbAccessory, val action: String)
    private var pendingUsbPermission: PendingUsbPermission? = null
    private var usbPermissionReceiver: BroadcastReceiver? = null

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            discoveryGeneration.incrementAndGet()
            pendingConnection = null
            receiver = service as ReceiverUiBinder
            refresh()
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            discoveryGeneration.incrementAndGet()
            pendingConnection = null
            receiver = null
        }
    }

    private val refreshTask = object : Runnable {
        override fun run() {
            refresh()
            if (bound) mainHandler.postDelayed(this, 700)
        }
    }

    private fun refresh() {
        val service = receiver ?: return
        snapshot = service.snapshot()
        val currentInterfaces = service.interfaces()
        interfaces.clear()
        interfaces.addAll(currentInterfaces)
        val activeAddress = snapshot?.networkAddress.orEmpty()
        val activeInterface = interfaces.singleOrNull { it.address == activeAddress }
        val currentSelection = interfaces.singleOrNull { it.address == selectedAddress && it.prefixLength == selectedPrefix }
        val preferences = getSharedPreferences("anpod_receiver", MODE_PRIVATE)
        val savedAddress = preferences.getString("network_address", "").orEmpty()
        val savedPrefix = preferences.getInt("network_prefix", 0)
        val savedSelection = interfaces.singleOrNull { it.address == savedAddress && it.prefixLength == savedPrefix }
        when {
            snapshot?.operationState == "running" && activeInterface != null -> {
                selectNetwork(activeInterface)
            }
            currentSelection != null -> selectNetwork(currentSelection)
            savedSelection != null -> {
                selectNetwork(savedSelection)
            }
            interfaces.size == 1 -> {
                selectNetwork(interfaces.single())
            }
            else -> {
                selectedAddress = ""
                selectedPrefix = 0
            }
        }
        advancePendingConnection(snapshot)
        refreshUsbAccessory()
        maybeEnterAod()
    }

    private fun refreshUsbAccessory() {
        val available = Build.VERSION.SDK_INT >= 24
                && packageManager.hasSystemFeature(PackageManager.FEATURE_USB_ACCESSORY)
        usbDirectAvailable = if (available) {
            val manager = getSystemService(UsbManager::class.java)
            manager?.accessoryList?.any { AoaReceiverCoordinator.isProductAccessory(it) } == true
        } else false
    }

    private fun selectNetwork(item: UiInterface) {
        val changed = selectedAddress != item.address || selectedPrefix != item.prefixLength
        selectedAddress = item.address
        selectedPrefix = item.prefixLength
        if (changed) {
            getSharedPreferences("anpod_receiver", MODE_PRIVATE).edit()
                .putString("network_address", item.address)
                .putInt("network_prefix", item.prefixLength)
                .apply()
        }
        if (receiver?.clearNetworkSelectionFailure(item.address, item.prefixLength) == true)
            snapshot = receiver?.snapshot()
    }

    private fun copyDiagnostics(message: String) {
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText(getString(R.string.diagnostics_label), message))
        Toast.makeText(this, R.string.diagnostics_copied, Toast.LENGTH_SHORT).show()
    }

    private fun openNotificationSettings() {
        val intent = if (Build.VERSION.SDK_INT >= 26) {
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
        } else {
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))
        }
        startActivity(intent)
    }

    private fun maybeEnterAod() {
        if (!aodEnabled) return
        if (!lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return
        val current = snapshot ?: return
        if (current.serviceState !in setOf("authenticated", "receiverRunning", "streaming")) return
        val key = current.operationId + "/" + current.operationGeneration
        if (aodShownForSession == key) return
        aodShownForSession = key
        startActivity(Intent(this, AodActivity::class.java))
    }

    private fun updateAodEnabled(value: Boolean) {
        aodEnabled = value
        getSharedPreferences("anpod_receiver", MODE_PRIVATE).edit().putBoolean("aod_enabled", value).apply()
    }

    private fun openAod() {
        startActivity(Intent(this, AodActivity::class.java))
    }

    override fun onStart() {
        super.onStart()
        activityStarted = true
        bound = bindService(Intent(this, ReceiverService::class.java), connection, Context.BIND_AUTO_CREATE)
        mainHandler.post(refreshTask)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        refresh()
    }

    override fun onStop() {
        activityStarted = false
        discoveryGeneration.incrementAndGet()
        pendingConnection = null
        if (pendingUsbPermission != null) {
            pendingUsbPermission = null
            usbPermissionPending = false
            unregisterUsbPermissionReceiver()
        }
        mainHandler.removeCallbacks(refreshTask)
        if (bound) unbindService(connection)
        bound = false
        receiver = null
        super.onStop()
    }

    override fun onDestroy() {
        unregisterUsbPermissionReceiver()
        pendingUsbPermission = null
        usbPermissionPending = false
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val preferences = getSharedPreferences("anpod_receiver", MODE_PRIVATE)
        aodEnabled = preferences.getBoolean("aod_enabled", false)
        aodDisplaySettings = AodDisplaySettings.read(preferences)
        outputEngine = when (preferences.getString("output_engine", "oboeExperimental")) {
            "oboeExperimental" -> "oboeExperimental"
            else -> "audioTrack"
        }
        connectionChoice = ConnectionChoice.fromPreference(preferences.getString("connection_choice", null))
        WindowCompat.setDecorFitsSystemWindows(window, false)
        setContent {
            ReceiverTheme {
                ReceiverScreen(
                    snapshot = snapshot,
                    interfaces = interfaces,
                    selectedAddress = selectedAddress,
                    selectedPrefix = selectedPrefix,
                    localMessage = localMessage,
                    connectionPending = pendingConnection != null,
                    onSelect = { selectNetwork(it) },
                    connectionChoice = connectionChoice,
                    onSelectConnectionChoice = { choice ->
                        if (choice != connectionChoice) localMessage = ""
                        saveConnectionChoicePreference(choice)
                    },
                    onConnectComputer = { connectComputer() },
                    onConnectUsbDirect = { connectUsbDirect() },
                    usbDirectAvailable = usbDirectAvailable,
                    usbDirectSupported = Build.VERSION.SDK_INT >= 24,
                    usbAccessoryFeatureAvailable = packageManager.hasSystemFeature(PackageManager.FEATURE_USB_ACCESSORY),
                    usbPermissionPending = usbPermissionPending,
                    onCancelUsbPermission = { cancelUsbPermission() },
                    onCancelPendingConnection = { cancelPendingConnection() },
                    onStartBackgroundStandby = { startBackgroundStandby() },
                    onStopBackgroundStandby = { stopBackgroundStandby() },
                    serviceConnected = receiver != null,
                    onTrustBinding = { target -> trustBindingUntilRevoked(target) },
                    onRevokeBackgroundTrust = { target, grantKind -> revokeBackgroundTrust(target, grantKind) },
                    onStop = {
                        val current = snapshot
                        cancelPendingConnection()
                        if (current != null) receiver?.stop(current.operationId)
                    },
                    aodEnabled = aodEnabled,
                    aodDisplaySettings = aodDisplaySettings,
                    onAodDisplaySettingsChange = { value ->
                        aodDisplaySettings = value
                        value.save(getSharedPreferences("anpod_receiver", MODE_PRIVATE))
                    },
                    onToggleAod = { updateAodEnabled(it) },
                    outputEngine = outputEngine,
                    onOutputEngineChange = { value ->
                        outputEngine = value
                        getSharedPreferences("anpod_receiver", MODE_PRIVATE).edit()
                            .putString("output_engine", value)
                            .apply()
                    },
                    onOpenAod = { openAod() },
                    onCopyDiagnostics = { copyDiagnostics(it) },
                    onOpenNotificationSettings = { openNotificationSettings() }
                )
            }
        }
    }

    private fun connectComputer() = beginReceive(backgroundStandby = false)

    internal fun saveConnectionChoicePreference(choice: ConnectionChoice) {
        connectionChoice = choice
        getSharedPreferences("anpod_receiver", MODE_PRIVATE).edit()
            .putString("connection_choice", choice.preferenceValue)
            .apply()
    }

    private fun connectUsbDirect() {
        val service = receiver
        if (!activityStarted || !bound || service == null) {
            localMessage = getString(R.string.receiver_service_not_ready_retry)
            return
        }
        if (Build.VERSION.SDK_INT < 24) {
            localMessage = getString(R.string.usb_direct_requires_api24)
            return
        }
        if (!packageManager.hasSystemFeature(PackageManager.FEATURE_USB_ACCESSORY)) {
            localMessage = getString(R.string.usb_direct_not_supported)
            return
        }
        val current = snapshot
        if (current?.bindingState != "saved") {
            localMessage = getString(R.string.usb_direct_requires_pairing)
            return
        }
        if (current.operationState in setOf("starting", "running", "stopping")
            || current.serviceState in setOf("starting", "prepared", "authenticated", "receiverRunning", "streaming", "paused")) {
            localMessage = getString(R.string.usb_direct_busy)
            return
        }
        val manager = getSystemService(UsbManager::class.java)
        val accessory = manager?.accessoryList?.firstOrNull { AoaReceiverCoordinator.isProductAccessory(it) }
        if (accessory == null) {
            usbDirectAvailable = false
            localMessage = getString(R.string.usb_direct_not_attached)
            return
        }
        val attempt = PendingUsbPermission(discoveryGeneration.incrementAndGet(), accessory,
                "$packageName.USB_PERMISSION.${discoveryGeneration.get()}")
        pendingUsbPermission = attempt
        usbPermissionPending = true
        localMessage = getString(R.string.usb_direct_permission_pending)
        if (manager.hasPermission(accessory)) {
            completeUsbPermission(attempt, true, accessory)
            return
        }
        val permissionReceiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action != attempt.action) return
                val returned = if (Build.VERSION.SDK_INT >= 33)
                    intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY, UsbAccessory::class.java)
                else @Suppress("DEPRECATION") (intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY) as? UsbAccessory)
                val granted = intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)
                completeUsbPermission(attempt, granted, returned)
            }
        }
        usbPermissionReceiver = permissionReceiver
        val filter = IntentFilter(attempt.action)
        if (Build.VERSION.SDK_INT >= 33)
            registerReceiver(permissionReceiver, filter, RECEIVER_NOT_EXPORTED)
        else registerReceiver(permissionReceiver, filter)
        val permissionIntent = Intent(attempt.action).setPackage(packageName)
            .setData(Uri.parse("anpod://usb-permission/${attempt.generation}"))
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
        val callback = PendingIntent.getBroadcast(this, attempt.generation.toInt(), permissionIntent, flags)
        try { manager.requestPermission(accessory, callback) }
        catch (error: RuntimeException) {
            unregisterUsbPermissionReceiver()
            pendingUsbPermission = null
            usbPermissionPending = false
            localMessage = getString(R.string.usb_direct_permission_failed)
        }
    }

    private fun completeUsbPermission(attempt: PendingUsbPermission, granted: Boolean, returned: UsbAccessory?) {
        if (pendingUsbPermission !== attempt) return
        val currentAttempt = isUsbPermissionResultCurrent(attempt.generation)
        unregisterUsbPermissionReceiver()
        pendingUsbPermission = null
        usbPermissionPending = false
        if (!currentAttempt) return
        if (!granted || returned == null) {
            localMessage = getString(R.string.usb_direct_permission_denied)
            return
        }
        val manager = getSystemService(UsbManager::class.java) ?: run {
            localMessage = getString(R.string.usb_direct_detached_or_permission_lost)
            refreshUsbAccessory()
            return
        }
        val permissionCurrent = try {
            manager.hasPermission(returned) && manager.hasPermission(attempt.accessory)
        } catch (_: SecurityException) {
            false
        }
        if (!permissionCurrent) {
            localMessage = getString(R.string.usb_direct_detached_or_permission_lost)
            refreshUsbAccessory()
            return
        }
        val attached = try {
            if (returned != attempt.accessory) {
                localMessage = getString(R.string.usb_direct_permission_denied)
                return
            }
            manager.accessoryList?.firstOrNull {
                manager.hasPermission(it) && it == attempt.accessory
                        && AoaReceiverCoordinator.isProductAccessory(it)
            }
        } catch (_: SecurityException) {
            null
        }
        val attachedStillPermitted = try { attached != null && manager.hasPermission(attached) }
        catch (_: SecurityException) { false }
        if (!attachedStillPermitted) {
            localMessage = getString(R.string.usb_direct_detached_or_permission_lost)
            refreshUsbAccessory()
            return
        }
        val result = receiver?.startAoaAccessory(attached, "usb-${attempt.generation}")
        localMessage = if (result.isNullOrEmpty()) "" else result
        refresh()
    }

    internal fun isUsbPermissionResultCurrent(generation: Long): Boolean {
        val pending = pendingUsbPermission ?: return false
        return pending.generation == generation && discoveryGeneration.get() == generation
                && activityStarted && bound
    }

    private fun cancelUsbPermission() {
        discoveryGeneration.incrementAndGet()
        pendingUsbPermission = null
        usbPermissionPending = false
        unregisterUsbPermissionReceiver()
        localMessage = getString(R.string.usb_direct_cancelled)
    }

    private fun unregisterUsbPermissionReceiver() {
        val receiver = usbPermissionReceiver ?: return
        usbPermissionReceiver = null
        try { unregisterReceiver(receiver) } catch (_: IllegalArgumentException) { }
    }

    private fun startBackgroundStandby() = beginReceive(backgroundStandby = true)

    private fun beginReceive(backgroundStandby: Boolean) {
        val current = snapshot
        val service = receiver
        if (current == null || service == null) { localMessage = getString(R.string.receiver_service_not_ready_retry); return }
        if (current.serviceState in setOf("starting", "prepared", "authenticated", "receiverRunning", "streaming", "paused")
            || current.operationState in setOf("starting", "running", "stopping")) return
        if (!activityStarted || !bound) { localMessage = getString(R.string.receiver_service_not_ready_retry); return }
        if (selectedAddress.isEmpty() || interfaces.none { it.address == selectedAddress && it.prefixLength == selectedPrefix }) {
            localMessage = getString(R.string.select_available_private_ipv4_interface)
            return
        }
        val generation = discoveryGeneration.incrementAndGet()
        pendingConnection = PendingConnection(generation, current.operationId, backgroundStandby)
        localMessage = ""
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestNotificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            return
        }
        startReceiverForPendingConnection()
    }

    private val requestNotificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startReceiverForPendingConnection() else {
            val backgroundStandby = pendingConnection?.backgroundStandby == true
            pendingConnection = null
            localMessage = getString(if (backgroundStandby) R.string.background_notification_permission_required else R.string.notification_permission_required)
        }
    }

    private fun startReceiverForPendingConnection() {
        val pending = pendingConnection ?: return
        val service = receiver ?: run { pendingConnection = null; localMessage = getString(R.string.receiver_service_connecting_retry); return }
        if (!activityStarted || pending.generation != discoveryGeneration.get()) { pendingConnection = null; return }
        val current = snapshot
        if (current == null || current.operationId != pending.previousOperationId
            || current.operationState in setOf("starting", "running", "stopping")
            || current.serviceState in setOf("starting", "prepared", "authenticated", "receiverRunning", "streaming", "paused")) {
            pendingConnection = null
            localMessage = getString(R.string.receiver_operation_changed_retry)
            return
        }
        if (selectedAddress.isEmpty() || interfaces.none { it.address == selectedAddress && it.prefixLength == selectedPrefix }) {
            pendingConnection = null
            localMessage = getString(R.string.no_private_ipv4_interface_available)
            return
        }
        if (pendingUsbPermission != null) cancelUsbPermission()
        localMessage = ""
        localMessage = if (pending.backgroundStandby)
            service.startBackgroundStandby(selectedAddress, selectedPrefix, UUID.randomUUID().toString())
        else service.startReceiving(selectedAddress, selectedPrefix, UUID.randomUUID().toString())
        if (localMessage.isNotEmpty()) pendingConnection = null
        refresh()
    }

    private fun advancePendingConnection(current: UiSnapshot?) {
        var pending = pendingConnection ?: return
        if (!activityStarted || pending.generation != discoveryGeneration.get()) { pendingConnection = null; return }
        if (current == null) { pendingConnection = null; return }
        if (pending.operationId == null && current.operationId.isNotEmpty() && current.operationId != pending.previousOperationId) {
            pending = pending.copy(operationId = current.operationId)
            pendingConnection = pending
        }
        val targetOperationId = pending.operationId ?: return
        if (current.operationId != targetOperationId) { pendingConnection = null; return }
        if (current.backgroundStandbyEnabled != pending.backgroundStandby) { pendingConnection = null; return }
        if (current.serviceState == "prepared" && current.operationState == "running" && current.networkPort > 0) {
            pendingConnection = null
        } else if (current.serviceState in setOf("failed", "stopped", "receiverRunning", "streaming", "paused")
            || current.operationState in setOf("error", "stopped", "stopping")) {
            pendingConnection = null
        }
    }

    private fun cancelPendingConnection() {
        pendingConnection = null
        discoveryGeneration.incrementAndGet()
    }

    private fun stopBackgroundStandby() {
        val current = snapshot ?: return
        val service = receiver ?: run { localMessage = getString(R.string.receiver_service_not_ready_retry); return }
        if (!activityStarted || !bound || !current.backgroundStandbyEnabled || current.operationId.isEmpty()) return
        cancelPendingConnection()
        if (!service.stop(current.operationId, current.operationGeneration))
            localMessage = getString(R.string.background_standby_stop_failed)
        else localMessage = ""
        refresh()
    }

    private fun trustBindingUntilRevoked(target: BackgroundTrustTarget) {
        if (snapshot == null) return
        val service = receiver ?: run { localMessage = getString(R.string.receiver_service_not_ready_retry); return }
        val latest = service.snapshot()
        if (!activityStarted || !bound || !latest.backgroundStandbyEnabled || latest.operationId.isEmpty()
            || latest.operationId != target.operationId || latest.operationGeneration != target.operationGeneration
            || latest.bindingFingerprint != target.bindingFingerprint) { refresh(); return }
        localMessage = if (service.trustBindingUntilRevoked(target.operationId, target.operationGeneration)) ""
            else getString(R.string.background_trust_update_failed)
        refresh()
    }

    private fun revokeBackgroundTrust(target: BackgroundTrustTarget, expectedGrantKind: String) {
        snapshot ?: return
        val service = receiver ?: run { localMessage = getString(R.string.receiver_service_not_ready_retry); return }
        val current = service.snapshot()
        if (!activityStarted || !bound || !current.canRevokeBackgroundTrust
            || current.operationId != target.operationId || current.operationGeneration != target.operationGeneration
            || current.bindingFingerprint != target.bindingFingerprint || current.backgroundGrantKind != expectedGrantKind
            || expectedGrantKind !in setOf("once", "eightHours", "untilRevoked")) return
        localMessage = if (service.revokeBackgroundTrust(target.operationId, target.operationGeneration,
                target.bindingFingerprint)) ""
            else getString(R.string.background_trust_update_failed)
        refresh()
    }

}

@Composable
private fun ReceiverTheme(content: @Composable () -> Unit) {
    val context = LocalContext.current
    val dark = androidx.compose.foundation.isSystemInDarkTheme()
    val colors = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && dark -> dynamicDarkColorScheme(context)
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = colors, content = content)
}

@Composable
private fun ReceiverScreen(
    snapshot: UiSnapshot?,
    connectionChoice: ConnectionChoice,
    onSelectConnectionChoice: (ConnectionChoice) -> Unit,
    interfaces: List<UiInterface>,
    selectedAddress: String,
    selectedPrefix: Int,
    localMessage: String,
    connectionPending: Boolean,
    onSelect: (UiInterface) -> Unit,
    onConnectComputer: () -> Unit,
    onConnectUsbDirect: () -> Unit,
    onCancelPendingConnection: () -> Unit,
    usbDirectAvailable: Boolean,
    usbDirectSupported: Boolean,
    usbAccessoryFeatureAvailable: Boolean,
    usbPermissionPending: Boolean,
    onCancelUsbPermission: () -> Unit,
    onStartBackgroundStandby: () -> Unit,
    onStopBackgroundStandby: () -> Unit,
    serviceConnected: Boolean,
    onTrustBinding: (BackgroundTrustTarget) -> Unit,
    onRevokeBackgroundTrust: (BackgroundTrustTarget, String) -> Unit,
    onStop: () -> Unit,
    aodEnabled: Boolean,
    aodDisplaySettings: AodDisplaySettings,
    onAodDisplaySettingsChange: (AodDisplaySettings) -> Unit,
    onToggleAod: (Boolean) -> Unit,
    outputEngine: String,
    onOutputEngineChange: (String) -> Unit,
    onOpenAod: () -> Unit,
    onCopyDiagnostics: (String) -> Unit,
    onOpenNotificationSettings: () -> Unit
) {
    var trustConfirmationTarget by remember { mutableStateOf<BackgroundTrustTarget?>(null) }
    var otherSettingsExpanded by remember { mutableStateOf(false) }
    var aodSettingsOpen by remember { mutableStateOf(false) }
    val currentTrustTarget = snapshot?.takeIf {
        serviceConnected && (it.backgroundStandbyEnabled || it.canRevokeBackgroundTrust)
            && it.bindingFingerprint.isNotEmpty()
    }?.let { BackgroundTrustTarget(it.operationId, it.operationGeneration, it.bindingFingerprint) }
    LaunchedEffect(trustConfirmationTarget, currentTrustTarget) {
        if (trustConfirmationTarget != null && trustConfirmationTarget != currentTrustTarget) trustConfirmationTarget = null
    }
    val permissionPendingMessage = stringResource(R.string.usb_direct_permission_pending)
    val permissionCancelledMessage = stringResource(R.string.usb_direct_cancelled)
    val localStatusMessage = localMessage.takeIf {
        isInformationalUsbPermissionMessage(it, permissionPendingMessage, permissionCancelledMessage)
    }
    val diagnostic = localMessage.takeIf { it.isNotEmpty() && localStatusMessage == null }
        ?: snapshot?.error?.takeIf { it.isNotEmpty() }
        ?: snapshot?.backgroundAuthorizationError?.takeIf { it.isNotEmpty() }.orEmpty()
    val notificationPermissionDenied = localMessage == stringResource(R.string.notification_permission_required)
            || localMessage == stringResource(R.string.background_notification_permission_required)
    val bound = snapshot?.bindingState == "saved"
    val running = snapshot?.operationState in setOf("starting", "running", "stopping") ||
            snapshot?.serviceState in setOf("starting", "prepared", "authenticated", "receiverRunning", "streaming", "paused")
    val selectedNetworkAvailable = interfaces.any { it.address == selectedAddress && it.prefixLength == selectedPrefix }
    val networkChoiceNeeded = !running && interfaces.size > 1
            && (!selectedNetworkAvailable || diagnostic.contains("InvalidReceiverInterface"))
    val needsAttention = diagnostic.isNotEmpty() || snapshot?.backgroundListenerState == "pending"
            || snapshot?.serviceState == "failed" || snapshot?.operationState == "error"
            || snapshot?.backgroundGrantKind == "unavailable"
    val backgroundStandbyEnabled = snapshot?.backgroundStandbyEnabled == true
    val receiverActive = snapshot?.serviceState in setOf("authenticated", "receiverRunning", "streaming", "paused") ||
        (!backgroundStandbyEnabled && (snapshot?.operationState in setOf("starting", "running") ||
            snapshot?.serviceState in setOf("starting", "prepared")))
    val selectionLocked = running || connectionPending || usbPermissionPending
    val action = primaryActionFor(
        connectionChoice,
        receiverActive = receiverActive,
        operationStopping = snapshot?.operationState == "stopping",
        usbPermissionPending = usbPermissionPending,
        connectionPending = connectionPending,
        backgroundStandbyEnabled = backgroundStandbyEnabled
    )
    val displayedConnectionChoice = displayedConnectionChoice(
        connectionChoice, snapshot?.connectionMethod, selectionLocked
    )
    val canPerformAction = action != PrimaryAction.DISABLED && when (action) {
        PrimaryAction.CONNECT_USB_DIRECT -> usbDirectSupported && usbAccessoryFeatureAvailable && serviceConnected
        PrimaryAction.CONNECT_WIFI -> serviceConnected
        PrimaryAction.WAIT_ADB -> false
        PrimaryAction.CANCEL_USB_PERMISSION, PrimaryAction.CANCEL_PENDING_CONNECTION -> true
        PrimaryAction.STOP_RECEIVER, PrimaryAction.STOP_BACKGROUND_STANDBY -> serviceConnected
        PrimaryAction.DISABLED -> false
    }

    Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(
            modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(horizontal = 20.dp),
        ) {
            Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.padding(top = 28.dp, bottom = 14.dp))
            Column(
                modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState())
                    .padding(bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp)
            ) {

            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        stateLabel(snapshot, connectionPending, needsAttention, usbPermissionPending),
                        style = MaterialTheme.typography.headlineSmall,
                        color = if (needsAttention) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
                        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite }
                    )
                    if (snapshot?.serviceState in setOf("authenticated", "receiverRunning", "streaming", "paused")) snapshot?.let {
                        Text(stringResource(R.string.actual_connection,
                            stringResource(actualConnectionLabel(it.connectionMethod))),
                            style = MaterialTheme.typography.bodyMedium)
                    }
                    val statusHint = when {
                        connectionChoice == ConnectionChoice.USB_ADB && !needsAttention && !receiverActive &&
                                !backgroundStandbyEnabled && !connectionPending -> null
                        snapshot == null || needsAttention || receiverActive || backgroundStandbyEnabled || connectionPending ->
                            nextStep(snapshot, bound, connectionPending, diagnostic, interfaces.isEmpty(),
                                networkChoiceNeeded, notificationPermissionDenied)
                        else -> connectionChoice.descriptionResource()?.let { stringResource(it) }
                    }
                    statusHint?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    var errorDetailsExpanded by remember(diagnostic) { mutableStateOf(false) }
                    if (diagnostic.isNotEmpty()) {
                        TextButton(
                            onClick = { errorDetailsExpanded = !errorDetailsExpanded },
                            modifier = Modifier.heightIn(min = 48.dp)
                        ) {
                            Text(stringResource(R.string.show_error_details))
                        }
                        if (errorDetailsExpanded) {
                            Text(diagnostic, style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant)
                            TextButton(onClick = { onCopyDiagnostics(diagnostic) },
                                modifier = Modifier.heightIn(min = 48.dp)) {
                                Text(stringResource(R.string.copy_diagnostics))
                            }
                        }
                        if (notificationPermissionDenied) {
                            TextButton(onClick = onOpenNotificationSettings,
                                modifier = Modifier.heightIn(min = 48.dp)) {
                                Text(stringResource(R.string.open_notification_settings))
                            }
                        }
                    }
                }
            }

            val methodCardColor = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)
            Card(colors = methodCardColor) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(stringResource(R.string.connection_mode), style = MaterialTheme.typography.titleMedium)
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                        ConnectionChoice.values().forEachIndexed { index, choice ->
                            val accessibleLabel = stringResource(choice.labelResource())
                            SegmentedButton(
                                selected = displayedConnectionChoice == choice,
                                onClick = { onSelectConnectionChoice(choice) },
                                enabled = !selectionLocked,
                                shape = SegmentedButtonDefaults.itemShape(index, ConnectionChoice.values().size),
                                modifier = Modifier.heightIn(min = 48.dp).semantics {
                                    contentDescription = accessibleLabel
                                }
                            ) { Text(stringResource(choice.shortLabelResource()), softWrap = true) }
                        }
                    }
                    displayedConnectionChoice?.descriptionResource()?.let {
                        Text(stringResource(it), style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Button(
                        onClick = {
                            when (action) {
                                PrimaryAction.CONNECT_USB_DIRECT -> onConnectUsbDirect()
                                PrimaryAction.WAIT_ADB -> Unit
                                PrimaryAction.CONNECT_WIFI -> onConnectComputer()
                                PrimaryAction.CANCEL_USB_PERMISSION -> onCancelUsbPermission()
                                PrimaryAction.CANCEL_PENDING_CONNECTION -> onCancelPendingConnection()
                                PrimaryAction.STOP_RECEIVER -> onStop()
                                PrimaryAction.STOP_BACKGROUND_STANDBY -> onStopBackgroundStandby()
                                PrimaryAction.DISABLED -> Unit
                            }
                        },
                        enabled = canPerformAction,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)
                    ) { Text(stringResource(action.labelResource())) }
                    if (localStatusMessage != null) Text(localStatusMessage,
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (shouldShowUsbDirectGuidance(displayedConnectionChoice, selectionLocked)) {
                        val directNote = when {
                            !usbDirectSupported -> R.string.usb_direct_requires_api24
                            !usbAccessoryFeatureAvailable -> R.string.usb_direct_not_supported
                            !usbDirectAvailable -> R.string.usb_direct_connect_hint
                            else -> 0
                        }
                        if (directNote != 0) Text(stringResource(directNote),
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    if (connectionChoice == ConnectionChoice.WIFI && !running && (networkChoiceNeeded || interfaces.isEmpty())) {
                        Text(stringResource(R.string.network_recovery_title), style = MaterialTheme.typography.titleSmall)
                        if (interfaces.isEmpty()) {
                            Text(stringResource(R.string.no_private_ipv4_interfaces_found),
                                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        } else {
                            Text(stringResource(R.string.network_recovery_help),
                                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            InterfacePicker(interfaces, selectedAddress, selectedPrefix, false, onSelect)
                        }
                    }
                }
            }

            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        val aodToggleDescription = stringResource(R.string.aod_toggle)
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(stringResource(R.string.aod_title), style = MaterialTheme.typography.titleMedium)
                            Text(
                                stringResource(R.string.aod_toggle_description),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        Switch(
                            checked = aodEnabled,
                            onCheckedChange = onToggleAod,
                            modifier = Modifier.semantics {
                                contentDescription = aodToggleDescription
                            }
                        )
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton(onClick = { aodSettingsOpen = true },
                            modifier = Modifier.weight(1f).heightIn(min = 48.dp)) {
                            Text(stringResource(R.string.aod_settings))
                        }
                        OutlinedButton(onClick = onOpenAod, enabled = serviceConnected,
                            modifier = Modifier.weight(1f).heightIn(min = 48.dp)) {
                            Text(stringResource(R.string.aod_preview))
                        }
                    }
                }
            }

            Card(colors = methodCardColor) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { otherSettingsExpanded = !otherSettingsExpanded },
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text(stringResource(if (otherSettingsExpanded) R.string.hide_other_settings else R.string.other_settings))
                    }
                    AnimatedVisibility(visible = otherSettingsExpanded) {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            OutputEnginePicker(snapshot, outputEngine, onOutputEngineChange)
                            Text(stringResource(R.string.background_standby_title), style = MaterialTheme.typography.titleMedium)
                            Text(
                                backgroundStandbyStatus(snapshot, bound, running, connectionPending),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Text(backgroundGrantStatus(snapshot), style = MaterialTheme.typography.bodyMedium)
                            if (snapshot?.backgroundAuthorizationError?.isNotEmpty() == true) {
                                Text(stringResource(R.string.background_authorization_failed), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
                            }
                            if (connectionChoice != ConnectionChoice.WIFI && !running && !selectedNetworkAvailable) {
                                Text(stringResource(R.string.network_recovery_title), style = MaterialTheme.typography.titleSmall)
                                if (interfaces.isEmpty()) {
                                    Text(stringResource(R.string.no_private_ipv4_interfaces_found),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                                } else {
                                    Text(stringResource(R.string.network_recovery_help),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    InterfacePicker(interfaces, selectedAddress, selectedPrefix, false, onSelect)
                                }
                            }
                            when {
                                snapshot?.backgroundStandbyEnabled == true -> {
                                    if (snapshot.backgroundGrantKind != "untilRevoked") {
                                        TextButton(
                                            onClick = { trustConfirmationTarget = currentTrustTarget },
                                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                        ) { Text(stringResource(R.string.trust_binding_until_revoked)) }
                                    }
                                    if (snapshot.canRevokeBackgroundTrust
                                        && snapshot.backgroundGrantKind in setOf("once", "eightHours", "untilRevoked")) {
                                        TextButton(
                                            onClick = {
                                                currentTrustTarget?.let { onRevokeBackgroundTrust(it, snapshot.backgroundGrantKind) }
                                            },
                                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                        ) { Text(stringResource(R.string.revoke_background_trust_and_stop)) }
                                    }
                                }
                                snapshot != null && bound && !running && !connectionPending -> {
                                    TextButton(
                                        onClick = onStartBackgroundStandby,
                                        enabled = selectedAddress.isNotEmpty() && interfaces.any { it.address == selectedAddress && it.prefixLength == selectedPrefix },
                                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                    ) { Text(stringResource(R.string.start_background_standby)) }
                                    if (snapshot.canRevokeBackgroundTrust
                                        && snapshot.backgroundGrantKind in setOf("eightHours", "untilRevoked")) {
                                        TextButton(
                                            onClick = {
                                                currentTrustTarget?.let { onRevokeBackgroundTrust(it, snapshot.backgroundGrantKind) }
                                            },
                                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                        ) { Text(stringResource(if (snapshot.backgroundGrantKind == "untilRevoked") R.string.revoke_background_trust else R.string.revoke_background_trust_temporary)) }
                                    }
                                }
                                else -> Text(stringResource(R.string.background_standby_requires_idle_receiver), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
            }

            Spacer(Modifier.height(8.dp))
        }
    }
    }
    val confirmedTarget = trustConfirmationTarget
    if (aodSettingsOpen) AodSettingsDialog(
        settings = aodDisplaySettings,
        onChange = onAodDisplaySettingsChange,
        onClose = { aodSettingsOpen = false }
    )
    if (confirmedTarget != null && confirmedTarget == currentTrustTarget) {
        var showFullBindingId by remember(confirmedTarget) { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { trustConfirmationTarget = null },
            title = { Text(stringResource(R.string.confirm_background_trust_title)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(stringResource(R.string.confirm_background_trust_message))
                    Text(
                        stringResource(
                            R.string.background_binding_fingerprint_short,
                            confirmedTarget.bindingFingerprint.take(8),
                            confirmedTarget.bindingFingerprint.takeLast(8)
                        ),
                        style = MaterialTheme.typography.bodySmall
                    )
                    TextButton(
                        onClick = { showFullBindingId = !showFullBindingId },
                        modifier = Modifier.heightIn(min = 48.dp)
                    ) {
                        Text(stringResource(if (showFullBindingId) R.string.hide_paired_computer_id else R.string.show_paired_computer_id))
                    }
                    if (showFullBindingId) {
                        Text(
                            stringResource(R.string.background_binding_fingerprint, confirmedTarget.bindingFingerprint.chunked(4).joinToString(" ")),
                            style = MaterialTheme.typography.bodySmall
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = { trustConfirmationTarget = null; onTrustBinding(confirmedTarget) },
                    modifier = Modifier.heightIn(min = 48.dp)
                ) {
                    Text(stringResource(R.string.trust_binding_until_revoked))
                }
            },
            dismissButton = {
                TextButton(onClick = { trustConfirmationTarget = null }, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.dialog_cancel))
                }
            }
        )
    }
}

@Composable
private fun OutputEnginePicker(snapshot: UiSnapshot?, outputEngine: String, onSelect: (String) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    val selectedLabel = outputEngineLabel(outputEngine)
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.output_engine_title), style = MaterialTheme.typography.titleMedium)
            Text(
                stringResource(R.string.output_engine_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            OutlinedButton(
                onClick = { expanded = true },
                modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)
            ) { Text(selectedLabel) }
            DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.output_engine_audio_track)) },
                    onClick = { onSelect("audioTrack"); expanded = false }
                )
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.output_engine_oboe)) },
                    onClick = { onSelect("oboeExperimental"); expanded = false }
                )
            }
            val sessionActive = snapshot?.serviceState in setOf(
                "prepared", "authenticated", "receiverRunning", "streaming", "paused"
            )
            if (sessionActive) {
                Text(
                    stringResource(R.string.output_engine_actual, outputEngineLabel(snapshot?.actualOutputEngine.orEmpty())),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                if (snapshot?.actualOutputEngine == "oboeExperimental") {
                    Text(
                        if (snapshot.oboeXrunCount >= 0)
                            stringResource(R.string.output_engine_oboe_xruns, snapshot.oboeXrunCount)
                        else stringResource(R.string.output_engine_oboe_xruns_unknown),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            val fallbackReason = snapshot?.outputFallbackReason.orEmpty()
            if (fallbackReason.isNotEmpty()) {
                Text(
                    stringResource(R.string.output_engine_fallback, fallbackReason),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
        }
    }

@Composable
private fun outputEngineLabel(engine: String): String = stringResource(
    when (engine) {
        "audioTrack" -> R.string.output_engine_audio_track
        "oboeExperimental" -> R.string.output_engine_oboe
        else -> R.string.output_engine_unknown
    }
)

@Composable
private fun backgroundStandbyStatus(
    snapshot: UiSnapshot?,
    bound: Boolean,
    running: Boolean,
    connectionPending: Boolean
): String = when {
    snapshot == null -> stringResource(R.string.background_standby_status_loading)
    snapshot.backgroundStandbyEnabled && snapshot.backgroundListenerState == "starting" -> stringResource(R.string.background_standby_status_starting)
    snapshot.backgroundStandbyEnabled && snapshot.backgroundListenerState == "pending" -> stringResource(R.string.background_standby_status_pending)
    snapshot.backgroundStandbyEnabled && snapshot.backgroundListenerState == "listening" -> stringResource(R.string.background_standby_status_listening)
    snapshot.backgroundStandbyEnabled -> stringResource(R.string.background_standby_status_enabled)
    connectionPending -> stringResource(R.string.background_standby_status_starting)
    !bound -> stringResource(R.string.background_standby_requires_binding)
    running -> stringResource(R.string.background_standby_requires_idle_receiver)
    else -> stringResource(R.string.background_standby_status_disabled)
}

@Composable
private fun backgroundGrantStatus(snapshot: UiSnapshot?): String = when (snapshot?.backgroundGrantKind) {
    null -> stringResource(R.string.background_grant_status_loading)
    "once" -> stringResource(R.string.background_grant_once)
    "eightHours" -> {
        val totalMinutes = ((snapshot.backgroundGrantRemainingMs + 59_999L) / 60_000L).coerceAtLeast(1)
        stringResource(R.string.background_grant_eight_hours, totalMinutes / 60, totalMinutes % 60)
    }
    "untilRevoked" -> stringResource(R.string.background_grant_until_revoked)
    "unavailable" -> stringResource(R.string.background_authorization_failed)
    else -> stringResource(R.string.background_grant_none)
}

@Composable
private fun InterfacePicker(
    interfaces: List<UiInterface>,
    selectedAddress: String,
    selectedPrefix: Int,
    disabled: Boolean,
    onSelect: (UiInterface) -> Unit
) {
    var expanded by remember { mutableStateOf(false) }
    val selected = interfaces.firstOrNull { it.address == selectedAddress && it.prefixLength == selectedPrefix }
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = MaterialTheme.shapes.medium) {
        Column {
            OutlinedButton(
                onClick = { expanded = true },
                enabled = interfaces.isNotEmpty() && !disabled,
                modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)
            ) { Text(selected?.let { stringResource(R.string.network_interface_format, it.name, it.address, it.prefixLength) } ?: stringResource(R.string.select_network_interface)) }
            DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                interfaces.forEach { item ->
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.network_interface_format, item.name, item.address, item.prefixLength)) },
                        onClick = { onSelect(item); expanded = false }
                    )
                }
            }
        }
    }
}

@Composable
private fun stateLabel(
    snapshot: UiSnapshot?,
    connectionPending: Boolean,
    needsAttention: Boolean,
    usbPermissionPending: Boolean
): String =
    stringResource(chooseMainStateLabel(
        snapshot?.serviceState, snapshot?.operationState, snapshot != null,
        snapshot?.backgroundStandbyEnabled == true, connectionPending, needsAttention, snapshot?.playbackAdvancing,
        usbPermissionPending
    ))

internal fun chooseMainStateLabel(
    serviceState: String?,
    operationState: String?,
    snapshotAvailable: Boolean,
    backgroundStandbyEnabled: Boolean,
    connectionPending: Boolean,
    needsAttention: Boolean,
    playbackAdvancing: Boolean? = null,
    usbPermissionPending: Boolean = false
): Int = when {
    operationState == "stopping" -> R.string.stopping
    needsAttention -> R.string.state_needs_attention
    usbPermissionPending -> R.string.usb_direct_permission_pending
    !snapshotAvailable || connectionPending || serviceState == "starting"
            || operationState == "starting" ->
        R.string.state_connecting
    serviceState == "streaming" -> if (playbackAdvancing == true) {
        R.string.state_playing_computer_audio
    } else R.string.state_receiving
    serviceState == "receiverRunning" -> if (playbackAdvancing == true) {
        R.string.state_playing_computer_audio
    } else R.string.aod_connected
    serviceState == "paused" -> R.string.paused
    serviceState == "authenticated" -> R.string.state_connected_waiting_for_audio
    backgroundStandbyEnabled || serviceState == "prepared" -> R.string.state_waiting_for_computer
    else -> R.string.state_not_connected
}

internal fun actualConnectionLabel(method: String?): Int = when (method) {
    "usb-aoa" -> R.string.connection_usb_direct
    "usb-adb" -> R.string.connection_usb_adb_short
    "usb-or-local" -> R.string.usb_or_local_connection
    "wifi-adb" -> R.string.connection_wifi_adb
    "wireless-udp" -> R.string.connection_wifi
    else -> R.string.actual_connection_unknown
}

internal fun connectionChoiceForActualMethod(method: String?): ConnectionChoice? = when (method) {
    "usb-aoa" -> ConnectionChoice.USB_DIRECT
    "usb-adb" -> ConnectionChoice.USB_ADB
    "wifi-adb", "wireless-udp" -> ConnectionChoice.WIFI
    else -> null
}

internal fun displayedConnectionChoice(
    preferredChoice: ConnectionChoice,
    actualMethod: String?,
    selectionLocked: Boolean
): ConnectionChoice? = if (selectionLocked) connectionChoiceForActualMethod(actualMethod) else preferredChoice

internal fun shouldShowUsbDirectGuidance(
    displayedChoice: ConnectionChoice?,
    selectionLocked: Boolean
) = !selectionLocked && displayedChoice == ConnectionChoice.USB_DIRECT

internal fun isInformationalUsbPermissionMessage(message: String, pending: String, cancelled: String) =
    message == pending || message == cancelled

private fun ConnectionChoice.labelResource(): Int = when (this) {
    ConnectionChoice.USB_DIRECT -> R.string.connection_usb_direct
    ConnectionChoice.USB_ADB -> R.string.connection_usb_adb
    ConnectionChoice.WIFI -> R.string.connection_wifi
}

private fun ConnectionChoice.shortLabelResource(): Int = when (this) {
    ConnectionChoice.USB_DIRECT -> R.string.connection_usb_direct_short
    ConnectionChoice.USB_ADB -> R.string.connection_usb_adb_short
    ConnectionChoice.WIFI -> R.string.connection_wifi
}

private fun ConnectionChoice.descriptionResource(): Int? = when (this) {
    ConnectionChoice.USB_DIRECT -> null
    ConnectionChoice.USB_ADB -> R.string.connection_adb_description
    ConnectionChoice.WIFI -> R.string.connection_wifi_description
}

private fun PrimaryAction.labelResource(): Int = when (this) {
    PrimaryAction.CONNECT_USB_DIRECT, PrimaryAction.CONNECT_WIFI -> R.string.start_receiving
    PrimaryAction.WAIT_ADB -> R.string.waiting_for_computer_connection
    PrimaryAction.CANCEL_USB_PERMISSION -> R.string.usb_direct_cancel
    PrimaryAction.CANCEL_PENDING_CONNECTION -> R.string.cancel_connection
    PrimaryAction.STOP_RECEIVER -> R.string.stop_receiving
    PrimaryAction.STOP_BACKGROUND_STANDBY -> R.string.background_standby_primary_action
    PrimaryAction.DISABLED -> R.string.stopping
}

@Composable
private fun nextStep(
    snapshot: UiSnapshot?,
    bound: Boolean,
    connectionPending: Boolean,
    diagnostic: String,
    noNetwork: Boolean,
    networkChoiceNeeded: Boolean,
    notificationPermissionDenied: Boolean
): String = when {
    snapshot == null -> stringResource(R.string.next_step_reading_status)
    notificationPermissionDenied -> stringResource(R.string.notification_permission_next_step)
    diagnostic.contains("InvalidReceiverInterface") && networkChoiceNeeded -> stringResource(R.string.network_choice_next_step)
    snapshot.backgroundGrantKind == "unavailable" -> stringResource(R.string.background_authorization_failed)
    diagnostic.isNotEmpty() -> stringResource(R.string.next_step_retry_after_check)
    connectionPending || snapshot?.serviceState == "starting" -> stringResource(R.string.next_step_wait_connecting)
    else -> stringResource(chooseMainNextStep(
        snapshot.serviceState, snapshot.playbackAdvancing, snapshot.backgroundListenerState,
        snapshot.backgroundStandbyEnabled, snapshot.backgroundGrantKind,
        bound, noNetwork, networkChoiceNeeded
    ))
}

internal fun chooseMainNextStep(
    serviceState: String,
    playbackAdvancing: Boolean?,
    backgroundListenerState: String,
    backgroundStandbyEnabled: Boolean,
    backgroundGrantKind: String,
    bound: Boolean,
    noNetwork: Boolean,
    networkChoiceNeeded: Boolean
): Int = when {
    backgroundListenerState == "pending" -> R.string.next_step_allow_request
    serviceState == "receiverRunning" || serviceState == "streaming" -> when (playbackAdvancing) {
        true -> R.string.next_step_audio_received
        false -> R.string.next_step_check_computer_playback
        null -> R.string.next_step_play_on_computer
    }
    serviceState == "paused" -> R.string.next_step_resume_playback
    serviceState == "authenticated" -> R.string.next_step_play_on_computer
    serviceState == "prepared" -> R.string.next_step_choose_computer
    backgroundStandbyEnabled -> when (backgroundGrantKind) {
        "once" -> R.string.next_step_wait_once
        "eightHours", "untilRevoked" -> R.string.next_step_wait_auto
        else -> R.string.next_step_wait_and_allow
    }
    !bound -> R.string.next_step_pair_first
    noNetwork -> R.string.next_step_connect_wifi
    networkChoiceNeeded -> R.string.next_step_choose_network
    else -> R.string.next_step_start_receiving
}
