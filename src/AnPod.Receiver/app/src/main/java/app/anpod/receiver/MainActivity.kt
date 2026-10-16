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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import java.util.concurrent.atomic.AtomicLong
import java.util.UUID

class MainActivity : ComponentActivity() {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var receiver by mutableStateOf<ReceiverService.UiBinder?>(null)
    private var snapshot by mutableStateOf<ReceiverService.UiSnapshot?>(null)
    private val interfaces = mutableStateListOf<ReceiverService.UiInterface>()
    private var selectedAddress by mutableStateOf("")
    private var selectedPrefix by mutableStateOf(0)
    private var localMessage by mutableStateOf("")
    private var bound = false
    @Volatile private var activityStarted = false
    private val discoveryGeneration = AtomicLong()
    private data class PendingConnection(
        val generation: Long,
        val previousOperationId: String,
        val operationId: String? = null
    )
    private var pendingConnection by mutableStateOf<PendingConnection?>(null)

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            discoveryGeneration.incrementAndGet()
            pendingConnection = null
            receiver = service as ReceiverService.UiBinder
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
                    onStop = {
                        val current = snapshot
                        cancelPendingConnection()
                        if (current != null) receiver?.stop(current.operationId)
                    }
                )
            }
        }
    }

    private fun connectComputer() {
        val current = snapshot
        val service = receiver
        if (current == null || service == null) { localMessage = getString(R.string.receiver_service_not_ready_retry); return }
        if (current.serviceState in setOf("prepared", "authenticated", "receiverRunning", "streaming", "paused")
            || current.operationState in setOf("starting", "stopping")) return
        if (current.operationState == "running") return
        if (selectedAddress.isEmpty() || interfaces.none { it.address == selectedAddress && it.prefixLength == selectedPrefix }) {
            localMessage = getString(R.string.select_available_private_ipv4_interface)
            return
        }
        val generation = discoveryGeneration.incrementAndGet()
        pendingConnection = PendingConnection(generation, current.operationId)
        localMessage = ""
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestNotificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            return
        }
        startReceiverForPendingConnection()
    }

    private val requestNotificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startReceiverForPendingConnection() else {
            pendingConnection = null
            localMessage = getString(R.string.notification_permission_required)
        }
    }

    private fun startReceiverForPendingConnection() {
        val pending = pendingConnection ?: return
        val service = receiver ?: run { pendingConnection = null; localMessage = getString(R.string.receiver_service_connecting_retry); return }
        if (!activityStarted || pending.generation != discoveryGeneration.get()) { pendingConnection = null; return }
        if (selectedAddress.isEmpty()) { pendingConnection = null; localMessage = getString(R.string.no_private_ipv4_interface_available); return }
        localMessage = ""
        localMessage = service.startReceiving(selectedAddress, selectedPrefix, UUID.randomUUID().toString())
        if (localMessage.isNotEmpty()) pendingConnection = null
        refresh()
    }

    private fun advancePendingConnection(current: ReceiverService.UiSnapshot?) {
        var pending = pendingConnection ?: return
        if (!activityStarted || pending.generation != discoveryGeneration.get()) { pendingConnection = null; return }
        if (current == null) { pendingConnection = null; return }
        if (pending.operationId == null && current.operationId.isNotEmpty() && current.operationId != pending.previousOperationId) {
            pending = pending.copy(operationId = current.operationId)
            pendingConnection = pending
        }
        val targetOperationId = pending.operationId ?: return
        if (current.operationId != targetOperationId) { pendingConnection = null; return }
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
    snapshot: ReceiverService.UiSnapshot?,
    interfaces: List<ReceiverService.UiInterface>,
    selectedAddress: String,
    selectedPrefix: Int,
    localMessage: String,
    connectionPending: Boolean,
    onSelect: (ReceiverService.UiInterface) -> Unit,
    onConnectComputer: () -> Unit,
    onStop: () -> Unit
) {
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
            Button(
                onClick = when {
                    running -> onStop
                    else -> onConnectComputer
                },
                enabled = canStartReceiver || canStopReceiver,
                modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)
            ) {
                Text(when {
                    snapshot?.operationState == "stopping" -> stringResource(R.string.stopping)
                    canStopReceiver -> stringResource(R.string.stop_receiving)
                    connectionPending -> stringResource(R.string.processing)
                    else -> stringResource(R.string.start_receiving)
                })
            }

            Text(stringResource(R.string.audio_output), style = MaterialTheme.typography.titleMedium)
            OutputCard(icon = R.drawable.ic_phone, title = stringResource(R.string.phone), subtitle = when (snapshot?.serviceState) {
                "streaming", "receiverRunning" -> playbackAdvancingLabel(snapshot.playbackAdvancing)
                "receiverRunning" -> stringResource(R.string.receiving_ready_waiting_computer_audio)
                "paused" -> stringResource(R.string.paused)
                else -> stringResource(R.string.not_playing)
            })
            Text(stringResource(R.string.connection_method, methodLabel(snapshot?.connectionMethod)), style = MaterialTheme.typography.bodyMedium)
            Text(stringResource(R.string.playback_mode, playbackLabel(snapshot?.playbackMode)), style = MaterialTheme.typography.bodyMedium)

            if (!running && interfaces.size != 1) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.network_interface), style = MaterialTheme.typography.titleMedium)
                    InterfacePicker(interfaces, selectedAddress, selectedPrefix, false, onSelect)
                    if (interfaces.isEmpty()) Text(stringResource(R.string.no_private_ipv4_interfaces_found), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }

            val message = snapshot?.error?.takeIf { it.isNotEmpty() } ?: localMessage
            if (message.isNotEmpty()) Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)

            Spacer(Modifier.height(8.dp))
        }
    }
}

@Composable
private fun OutputCard(icon: Int, title: String, subtitle: String) {
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)) {
        Row(
            modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            androidx.compose.foundation.Image(
                painter = painterResource(icon), contentDescription = null,
                colorFilter = ColorFilter.tint(MaterialTheme.colorScheme.onSurfaceVariant),
                modifier = Modifier.size(24.dp)
            )
            Spacer(Modifier.width(12.dp))
            Column {
                Text(title, style = MaterialTheme.typography.titleSmall)
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun InterfacePicker(
    interfaces: List<ReceiverService.UiInterface>,
    selectedAddress: String,
    selectedPrefix: Int,
    disabled: Boolean,
    onSelect: (ReceiverService.UiInterface) -> Unit
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
private fun stateLabel(snapshot: ReceiverService.UiSnapshot, connectionPending: Boolean): String = when {
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
private fun stateDetail(snapshot: ReceiverService.UiSnapshot?): String = when (snapshot?.serviceState) {
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

@Composable
private fun methodLabel(method: String?): String = when (method) {
    "wireless-udp" -> stringResource(R.string.wireless_udp)
    "usb-or-local" -> stringResource(R.string.usb_or_local_connection)
    else -> stringResource(R.string.waiting_for_computer)
}

@Composable
private fun playbackLabel(mode: String?): String = when (mode) {
    "low-latency" -> stringResource(R.string.low_latency_reported_by_computer)
    "low-latency-limited" -> stringResource(R.string.low_latency_limited_by_system)
    "balanced" -> stringResource(R.string.balanced_reported_by_computer)
    "balanced-limited", "balanced-system", "balanced-fallback" -> stringResource(R.string.balanced_adjusted_by_system)
    else -> stringResource(R.string.playback_mode_not_reported)
}
