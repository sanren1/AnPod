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
import android.util.Log
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
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import java.util.concurrent.Executors
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
    private val discoveryLifecycleLock = Any()
    private var discoveryBusy by mutableStateOf(false)
    private val discoveryGeneration = AtomicLong()
    private val discoveryExecutor = Executors.newSingleThreadExecutor()
    private data class PendingConnection(
        val generation: Long,
        val previousOperationId: String,
        val operationId: String? = null,
        val automaticSearchStarted: Boolean = false
    )
    private var pendingConnection by mutableStateOf<PendingConnection?>(null)

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            synchronized(discoveryLifecycleLock) {
                discoveryGeneration.incrementAndGet()
                discoveryBusy = false
            }
            pendingConnection = null
            receiver = service as ReceiverService.UiBinder
            refresh()
        }
        override fun onServiceDisconnected(name: ComponentName?) {
            synchronized(discoveryLifecycleLock) {
                discoveryGeneration.incrementAndGet()
                discoveryBusy = false
            }
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
        synchronized(discoveryLifecycleLock) { activityStarted = true }
        bound = bindService(Intent(this, ReceiverService::class.java), connection, Context.BIND_AUTO_CREATE)
        mainHandler.post(refreshTask)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        refresh()
    }

    override fun onStop() {
        val service = receiver
        val pendingSearchOwner = synchronized(discoveryLifecycleLock) {
            activityStarted = false
            discoveryGeneration.incrementAndGet()
            discoveryBusy = false
            val pending = pendingConnection
            pendingConnection = null
            pending?.takeIf { it.automaticSearchStarted }?.operationId
        }
        if (pendingSearchOwner != null && service != null) {
            runCatching { service.stopSearch(pendingSearchOwner) }
                .onFailure { Log.w("MainActivity", "Could not cancel pending receiver search", it) }
        }
        mainHandler.removeCallbacks(refreshTask)
        if (bound) unbindService(connection)
        bound = false
        receiver = null
        super.onStop()
    }

    override fun onDestroy() {
        discoveryExecutor.shutdown()
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
                    discoveryBusy = discoveryBusy,
                    connectionPending = pendingConnection?.let { !it.automaticSearchStarted } == true,
                    onSelect = { selectedAddress = it.address; selectedPrefix = it.prefixLength },
                    onConnectComputer = { connectComputer() },
                    onStopSearch = {
                        cancelPendingConnection()
                        runDiscoveryOperation(operation = { binder, owner -> binder.stopSearch(owner); "" })
                    },
                    onStop = {
                        val current = snapshot
                        cancelPendingConnection()
                        if (current != null) receiver?.stop(current.operationId)
                    }
                )
            }
        }
    }

    private fun runDiscoveryOperation(
        operation: (ReceiverService.UiBinder, String) -> String,
        ownerId: String = snapshot?.operationId.orEmpty(),
        operationGeneration: Long = discoveryGeneration.incrementAndGet()
    ) {
        val service = receiver ?: run { localMessage = "接收服务正在连接，请稍后重试。"; return }
        if (ownerId.isEmpty()) { localMessage = "接收会话已结束，请先重新启动 Receiver。"; return }
        val generation = operationGeneration
        discoveryBusy = true
        localMessage = ""
        discoveryExecutor.execute {
            val result = synchronized(discoveryLifecycleLock) {
                if (generation != discoveryGeneration.get() || !activityStarted) null
                else runCatching { operation(service, ownerId) }
            } ?: return@execute
            mainHandler.post {
                if (!activityStarted || !bound || generation != discoveryGeneration.get() || receiver !== service) return@post
                discoveryBusy = false
                if (snapshot?.operationId != ownerId) { refresh(); return@post }
                result.fold(
                    onSuccess = { refresh() },
                    onFailure = { error ->
                        localMessage = error.message ?: error.toString()
                        if (pendingConnection?.let { it.generation == generation && it.operationId == ownerId && it.automaticSearchStarted } == true)
                            pendingConnection = null
                        refresh()
                    }
                )
            }
        }
    }

    private fun connectComputer() {
        val current = snapshot
        val service = receiver
        if (current == null || service == null) { localMessage = "接收服务尚未准备好，请稍后重试。"; return }
        if (current.serviceState in setOf("authenticated", "receiverRunning", "streaming", "paused") || current.operationState in setOf("starting", "stopping")) return
        if (current.serviceState == "prepared" && current.operationState == "running") {
            if (current.searchActive) runDiscoveryOperation({ binder, owner -> binder.stopSearch(owner); "" })
            else {
                val generation = discoveryGeneration.incrementAndGet()
                pendingConnection = PendingConnection(generation, current.operationId, current.operationId, true)
                runDiscoveryOperation({ binder, owner -> binder.startAutomaticSearch(owner); "" }, current.operationId, generation)
            }
            return
        }
        if (current.operationState == "running") return
        if (selectedAddress.isEmpty() || interfaces.none { it.address == selectedAddress && it.prefixLength == selectedPrefix }) {
            localMessage = "请选择一个可用的私有 IPv4 网络接口。"
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
            localMessage = "需要通知权限才能显示接收服务状态。"
        }
    }

    private fun startReceiverForPendingConnection() {
        val pending = pendingConnection ?: return
        val service = receiver ?: run { pendingConnection = null; localMessage = "接收服务正在连接，请稍后重试。"; return }
        if (!activityStarted || pending.generation != discoveryGeneration.get()) { pendingConnection = null; return }
        if (selectedAddress.isEmpty()) { pendingConnection = null; localMessage = "当前没有可用的私有 IPv4 网络接口。"; return }
        localMessage = ""
        localMessage = service.start(selectedAddress, selectedPrefix, UUID.randomUUID().toString())
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
        if (current.serviceState == "prepared" && current.operationState == "running" && current.networkPort > 0
            && !pending.automaticSearchStarted) {
            pending = pending.copy(automaticSearchStarted = true)
            pendingConnection = pending
            runDiscoveryOperation({ binder, owner -> binder.startAutomaticSearch(owner); "" }, targetOperationId, pending.generation)
        } else if (current.serviceState in setOf("failed", "stopped", "receiverRunning", "streaming", "paused")
            || current.operationState in setOf("error", "stopped", "stopping")) {
            pendingConnection = null
        }
    }

    private fun cancelPendingConnection() {
        pendingConnection = null
        discoveryGeneration.incrementAndGet()
        discoveryBusy = false
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
    discoveryBusy: Boolean,
    connectionPending: Boolean,
    onSelect: (ReceiverService.UiInterface) -> Unit,
    onConnectComputer: () -> Unit,
    onStopSearch: () -> Unit,
    onStop: () -> Unit
) {
    val state = snapshot?.serviceState?.let(::stateLabel) ?: "正在读取接收状态…"
    val bound = snapshot?.bindingState == "saved"
    val running = snapshot?.operationState in setOf("starting", "running", "stopping") ||
            snapshot?.serviceState in setOf("starting", "authenticated", "receiverRunning", "streaming", "paused")
    val prepared = snapshot?.serviceState == "prepared" && snapshot.operationState == "running"

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
                    if (bound) Text("目标：已绑定电脑", style = MaterialTheme.typography.bodyMedium)
                    if (!bound) Text(
                        if (snapshot?.bindingState == "unavailable") "绑定状态暂时不可读取。"
                        else "尚未绑定电脑；请使用 USB 完成首次绑定。",
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }

            val activeInterfaceMatches = snapshot?.let { it.networkAddress == selectedAddress && it.networkPort > 0 } == true
            val canStartReceiver = snapshot != null && bound && !running && selectedAddress.isNotEmpty() &&
                    interfaces.any { it.address == selectedAddress && it.prefixLength == selectedPrefix }
            val canSearch = prepared && activeInterfaceMatches
            val canStopReceiver = running && snapshot?.operationState != "stopping"
            Button(
                onClick = when {
                    prepared && snapshot?.searchActive == true -> onStopSearch
                    prepared -> onConnectComputer
                    running -> onStop
                    else -> onConnectComputer
                },
                enabled = !discoveryBusy && (canStartReceiver || canSearch || canStopReceiver),
                modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)
            ) {
                Text(when {
                    discoveryBusy -> "正在处理…"
                    connectionPending -> "取消连接"
                    prepared && snapshot?.searchActive == true -> "取消连接"
                    prepared -> "开始接收"
                    snapshot?.operationState == "stopping" -> "正在停止…"
                    canStopReceiver -> "停止接收"
                    else -> "开始接收"
                })
            }

            Text("声音输出", style = MaterialTheme.typography.titleMedium)
            OutputCard(icon = R.drawable.ic_phone, title = "手机", subtitle = when (snapshot?.serviceState) {
                "streaming" -> "正在播放电脑音频"
                "receiverRunning" -> "接收已就绪"
                "paused" -> "已暂停"
                else -> "尚未播放"
            })
            Text("连接方式：${methodLabel(snapshot?.connectionMethod)}", style = MaterialTheme.typography.bodyMedium)

            if (!running && interfaces.size != 1) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("网络接口", style = MaterialTheme.typography.titleMedium)
                    InterfacePicker(interfaces, selectedAddress, selectedPrefix, false, onSelect)
                    if (interfaces.isEmpty()) Text("未发现可用的私有 IPv4 接口。", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
            ) { Text(selected?.let { "${it.name} · ${it.address}/${it.prefixLength}" } ?: "选择网络接口") }
            DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                interfaces.forEach { item ->
                    DropdownMenuItem(
                        text = { Text("${item.name} · ${item.address}/${item.prefixLength}") },
                        onClick = { onSelect(item); expanded = false }
                    )
                }
            }
        }
    }
}

private fun stateLabel(state: String): String = when (state) {
    "starting" -> "正在连接电脑…"
    "prepared" -> "接收已准备，等待电脑连接"
    "authenticated" -> "已连接，等待音频"
    "receiverRunning" -> "接收已启动"
    "streaming" -> "正在播放电脑音频"
    "paused" -> "播放已暂停"
    "failed" -> "连接失败"
    "stopping" -> "正在停止…"
    else -> "等待连接"
}

private fun methodLabel(method: String?): String = when (method) {
    "wireless-udp" -> "无线 · UDP"
    "usb-or-local" -> "USB / 本地连接"
    else -> "等待电脑连接"
}

private fun playbackLabel(mode: String?): String = when (mode) {
    "low-latency" -> "低延迟（电脑报告）"
    "low-latency-limited" -> "低延迟（系统受限）"
    "balanced" -> "均衡（电脑报告）"
    "balanced-limited", "balanced-system", "balanced-fallback" -> "均衡（系统调整）"
    else -> "电脑尚未报告实际播放模式"
}
