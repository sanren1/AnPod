package app.anpod.receiver

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
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
    private var aodShownForSession: String? = null

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
        when {
            snapshot?.operationState == "running" && activeInterface != null -> {
                selectedAddress = activeInterface.address
                selectedPrefix = activeInterface.prefixLength
            }
            currentSelection != null -> Unit
            interfaces.size == 1 -> {
                selectedAddress = interfaces.single().address
                selectedPrefix = interfaces.single().prefixLength
            }
            else -> {
                selectedAddress = ""
                selectedPrefix = 0
            }
        }
        advancePendingConnection(snapshot)
        maybeEnterAod()
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
        mainHandler.removeCallbacks(refreshTask)
        if (bound) unbindService(connection)
        bound = false
        receiver = null
        super.onStop()
    }

    override fun onDestroy() {
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        aodEnabled = getSharedPreferences("anpod_receiver", MODE_PRIVATE).getBoolean("aod_enabled", false)
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
                    onSelect = { selectedAddress = it.address; selectedPrefix = it.prefixLength },
                    onConnectComputer = { connectComputer() },
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
                    onToggleAod = { updateAodEnabled(it) },
                    onOpenAod = { openAod() }
                )
            }
        }
    }

    private fun connectComputer() = beginReceive(backgroundStandby = false)

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
    interfaces: List<UiInterface>,
    selectedAddress: String,
    selectedPrefix: Int,
    localMessage: String,
    connectionPending: Boolean,
    onSelect: (UiInterface) -> Unit,
    onConnectComputer: () -> Unit,
    onStartBackgroundStandby: () -> Unit,
    onStopBackgroundStandby: () -> Unit,
    serviceConnected: Boolean,
    onTrustBinding: (BackgroundTrustTarget) -> Unit,
    onRevokeBackgroundTrust: (BackgroundTrustTarget, String) -> Unit,
    onStop: () -> Unit,
    aodEnabled: Boolean,
    onToggleAod: (Boolean) -> Unit,
    onOpenAod: () -> Unit
) {
    var trustConfirmationTarget by remember { mutableStateOf<BackgroundTrustTarget?>(null) }
    val currentTrustTarget = snapshot?.takeIf {
        serviceConnected && (it.backgroundStandbyEnabled || it.canRevokeBackgroundTrust)
            && it.bindingFingerprint.isNotEmpty()
    }?.let { BackgroundTrustTarget(it.operationId, it.operationGeneration, it.bindingFingerprint) }
    LaunchedEffect(trustConfirmationTarget, currentTrustTarget) {
        if (trustConfirmationTarget != null && trustConfirmationTarget != currentTrustTarget) trustConfirmationTarget = null
    }
    val state = snapshot?.let { stateLabel(it, connectionPending) } ?: stringResource(R.string.reading_receive_status)
    val bound = snapshot?.bindingState == "saved"
    val running = snapshot?.operationState in setOf("starting", "running", "stopping") ||
            snapshot?.serviceState in setOf("starting", "prepared", "authenticated", "receiverRunning", "streaming", "paused")

    Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(
            modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(start = 20.dp, top = 28.dp, end = 20.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text("AnPod Receiver", style = MaterialTheme.typography.headlineSmall)
            }

            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(state, style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.primary)
                    Text(stateDetail(snapshot), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (bound) Text(stringResource(R.string.target_bound_computer), style = MaterialTheme.typography.bodyMedium)
                    if (!bound) Text(
                        if (snapshot?.bindingState == "unavailable") stringResource(R.string.binding_status_temporarily_unavailable)
                        else stringResource(R.string.not_bound_use_usb_first_pairing),
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }

            val canStartReceiver = snapshot != null && bound && !running && selectedAddress.isNotEmpty() &&
                    interfaces.any { it.address == selectedAddress && it.prefixLength == selectedPrefix }
            val canStopReceiver = running && snapshot?.operationState != "stopping"
            val backgroundStandbyEnabled = snapshot?.backgroundStandbyEnabled == true
            Button(
                onClick = when {
                    backgroundStandbyEnabled -> onStopBackgroundStandby
                    running -> onStop
                    else -> onConnectComputer
                },
                enabled = if (backgroundStandbyEnabled) snapshot?.operationState != "stopping"
                    else canStartReceiver || canStopReceiver,
                modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)
            ) {
                Text(when {
                    backgroundStandbyEnabled -> stringResource(R.string.background_standby_primary_action)
                    snapshot?.operationState == "stopping" -> stringResource(R.string.stopping)
                    canStopReceiver -> stringResource(R.string.stop_receiving)
                    connectionPending -> stringResource(R.string.processing)
                    else -> stringResource(R.string.start_receiving)
                })
            }


            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(stringResource(R.string.aod_title), style = MaterialTheme.typography.titleMedium)
                            Text(
                                stringResource(R.string.aod_toggle_description),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        Switch(checked = aodEnabled, onCheckedChange = onToggleAod)
                    }
                    TextButton(
                        onClick = onOpenAod,
                        enabled = serviceConnected,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    ) { Text(stringResource(R.string.aod_preview)) }
                }
            }

            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)) {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
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

            if (!running && interfaces.size != 1) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.network_interface), style = MaterialTheme.typography.titleMedium)
                    InterfacePicker(interfaces, selectedAddress, selectedPrefix, false, onSelect)
                    if (interfaces.isEmpty()) Text(stringResource(R.string.no_private_ipv4_interfaces_found), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }

            val message = localMessage.takeIf { it.isNotEmpty() }
                ?: snapshot?.error?.takeIf { it.isNotEmpty() }
                ?: snapshot?.backgroundAuthorizationError?.takeIf { it.isNotEmpty() }.orEmpty()
            var errorDetailsExpanded by remember(message) { mutableStateOf(false) }
            if (message.isNotEmpty()) {
                Text(stringResource(R.string.operation_error_summary), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
                TextButton(onClick = { errorDetailsExpanded = !errorDetailsExpanded }, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.show_error_details))
                }
                if (errorDetailsExpanded) Text(message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }

            Spacer(Modifier.height(8.dp))
        }
    }
    val confirmedTarget = trustConfirmationTarget
    if (confirmedTarget != null && confirmedTarget == currentTrustTarget) {
        AlertDialog(
            onDismissRequest = { trustConfirmationTarget = null },
            title = { Text(stringResource(R.string.confirm_background_trust_title)) },
            text = { Text(stringResource(R.string.confirm_background_trust_message, confirmedTarget.bindingFingerprint.chunked(4).joinToString(" "))) },
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
private fun stateLabel(snapshot: UiSnapshot, connectionPending: Boolean): String = when {
    connectionPending || snapshot.serviceState == "starting" || snapshot.operationState == "starting" -> stringResource(R.string.connecting_computer)
    snapshot.operationState == "stopping" -> stringResource(R.string.stopping_receiver)
    snapshot.operationState == "error" -> stringResource(R.string.operation_failed_retry)
    snapshot.serviceState == "prepared" -> stringResource(R.string.waiting_for_computer)
    snapshot.serviceState == "authenticated" -> stringResource(R.string.connected_waiting_for_audio)
    snapshot.serviceState == "paused" -> stringResource(R.string.paused)
    snapshot.serviceState == "failed" -> stringResource(R.string.connection_failed_retry)
    snapshot.serviceState == "stopping" -> stringResource(R.string.stopping_receiver)
    snapshot.serviceState in setOf("receiverRunning", "streaming") -> playbackAdvancingLabel(snapshot.playbackAdvancing)
    else -> stringResource(R.string.waiting_for_connection)
}

@Composable
private fun stateDetail(snapshot: UiSnapshot?): String = when (snapshot?.serviceState) {
    null -> stringResource(R.string.reading_receive_status_period)
    "streaming", "receiverRunning" -> when (snapshot.playbackAdvancing) {
        true -> stringResource(R.string.receiver_playback_progress_advancing_detail)
        false -> stringResource(R.string.receiver_playback_progress_still_detail)
        null -> stringResource(R.string.receiver_playback_progress_unknown_detail)
    }
    "authenticated" -> stringResource(R.string.computer_connected_waiting_receiver)
    "paused" -> stringResource(R.string.playback_session_paused)
    "failed" -> stringResource(R.string.view_error_then_retry_receiving)
    else -> stringResource(R.string.connected_does_not_mean_playing)
}

@Composable
private fun playbackAdvancingLabel(advancing: Boolean?): String = when (advancing) {
    true -> stringResource(R.string.audio_playback_progress_advancing)
    false -> stringResource(R.string.audio_received_playback_not_advancing)
    null -> stringResource(R.string.audio_playback_progress_unknown)
}
