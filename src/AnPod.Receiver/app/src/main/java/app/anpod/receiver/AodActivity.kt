package app.anpod.receiver

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.os.SystemClock
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemGestures
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import java.util.Calendar
import java.util.Locale
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

/** Full-screen pure-black pseudo-AOD: clock, real PCM waveform, five receive metrics. */
class AodActivity : ComponentActivity() {
    private companion object {
        /**
         * PCM gap tolerated before the AOD closes. Switching Balanced/LowLatency restarts the
         * Receiver audio session on the PC (a few seconds without PCM), so a 3s rule made the AOD
         * exit and the new session reopen it; a longer grace keeps the same AOD visible.
         */
        const val SESSION_GAP_EXIT_MS = 15_000L
    }

    private var binder by mutableStateOf<ReceiverUiBinder?>(null)
    private var telemetry by mutableStateOf<AodTelemetrySnapshot?>(null)
    private var waveform by mutableStateOf<PcmWaveformTap.Waveform?>(null)
    private var bound = false

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            binder = service as? ReceiverUiBinder
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            binder = null
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        volumeControlStream = AudioManager.STREAM_MUSIC
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        bound = bindService(Intent(this, ReceiverService::class.java), connection, Context.BIND_AUTO_CREATE)
        setContent {
            LaunchedEffect(binder) {
                while (isActive) {
                    waveform = try {
                        ReceiverService.aodWaveform()
                    } catch (error: Throwable) {
                        android.util.Log.e("AnPodAod", "waveform failed", error)
                        null
                    }
                    delay(33)
                }
            }
            LaunchedEffect(binder) {
                var disconnectedAt = 0L
                var sawSession = false
                while (isActive) {
                    val current = binder?.aodSnapshot()
                    telemetry = current
                    if (current != null && (current.status != AodTelemetrySnapshot.DisplayStatus.DISCONNECTED
                            || current.receivedAudioRecently)) {
                        sawSession = true
                        disconnectedAt = 0L
                    } else if (sawSession) {
                        if (disconnectedAt == 0L) disconnectedAt = SystemClock.elapsedRealtime()
                        if (SystemClock.elapsedRealtime() - disconnectedAt > SESSION_GAP_EXIT_MS) finish()
                    }
                    delay(250)
                }
            }
            AodScreen(
                telemetry = telemetry,
                waveform = waveform,
                onExit = { finish() }
            )
        }
    }

    override fun onDestroy() {
        if (bound) unbindService(connection)
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        super.onDestroy()
    }
}

@Composable
private fun AodScreen(
    telemetry: AodTelemetrySnapshot?,
    waveform: PcmWaveformTap.Waveform?,
    onExit: () -> Unit
) {
    var now by remember { mutableStateOf(Calendar.getInstance()) }
    LaunchedEffect(Unit) {
        while (isActive) {
            now = Calendar.getInstance()
            delay(1_000)
        }
    }
    val drift = rememberDrift()
    // Any touch or drag restores the exit label; it fades out after a few untouched seconds.
    var lastInteraction by remember { mutableStateOf(SystemClock.elapsedRealtime()) }
    var exitVisible by remember { mutableStateOf(true) }
    LaunchedEffect(Unit) {
        while (isActive) {
            val visible = AodDrift.exitVisible(SystemClock.elapsedRealtime() - lastInteraction)
            if (visible != exitVisible) exitVisible = visible
            delay(400)
        }
    }
    val exitAlpha by animateFloatAsState(
        targetValue = if (exitVisible) 1f else 0f,
        animationSpec = tween(durationMillis = 700),
        label = "aodExitAlpha"
    )
    BoxWithConstraints(
        Modifier.fillMaxSize().background(Color.Black).pointerInput(Unit) {
            awaitPointerEventScope {
                while (true) {
                    awaitPointerEvent(PointerEventPass.Initial)
                    lastInteraction = SystemClock.elapsedRealtime()
                }
            }
        }
    ) {
        val clockSize = (maxHeight.value * 0.40f).coerceIn(96f, 184f)
        val density = LocalDensity.current
        val screenHeightPx = with(density) { maxHeight.toPx() }
        val driftXPx = with(density) { drift.first.dp.toPx() }
        val driftYPx = with(density) { drift.second.dp.toPx() }
        val clearancePx = with(density) { 4.dp.toPx() }
        var clockBottomPx by remember { mutableStateOf(Float.NaN) }
        var footerTopPx by remember { mutableStateOf(Float.NaN) }
        val waveformLayout = AodWaveformGeometry.layout(
            if (clockBottomPx.isFinite()) clockBottomPx else screenHeightPx * 0.54f,
            if (footerTopPx.isFinite()) footerTopPx else screenHeightPx * 0.88f,
            with(density) { AodDrift.MAX_Y_DP.dp.toPx() },
            clearancePx
        )
        val waveformHeight = with(density) { waveformLayout.regionHeight.toDp() }
        val waveformTop = with(density) { (waveformLayout.centerY - waveformLayout.regionHeight / 2f).toDp() }
        Text(
            text = String.format(Locale.US, "%02d:%02d", now.get(Calendar.HOUR_OF_DAY), now.get(Calendar.MINUTE)),
            fontFamily = RubikClock,
            fontWeight = FontWeight.Black,
            fontSize = clockSize.sp,
            letterSpacing = (-5).sp,
            color = Color(0xFFF7F7F7),
            modifier = Modifier.align(Alignment.TopCenter)
                .padding(top = (maxHeight.value * 0.10f).dp)
                .offset(x = drift.first.dp, y = drift.second.dp)
                .onGloballyPositioned { coordinates ->
                    // The anchors describe the undrifted layout so the waveform region keeps its size
                    // while every element drifts together.
                    val bottom = coordinates.positionInRoot().y + coordinates.size.height - driftYPx
                    if (clockBottomPx.isNaN() || kotlin.math.abs(clockBottomPx - bottom) > 0.5f)
                        clockBottomPx = bottom
                }
        )
        WaveformCanvas(
            waveform,
            waveformLayout.maximumAmplitude,
            Modifier.align(Alignment.TopCenter)
                .offset(y = waveformTop)
                .fillMaxWidth(0.90f)
                .height(waveformHeight)
                .offset(x = drift.first.dp, y = drift.second.dp)
        )
        MetricsRow(
            telemetry,
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .padding(start = 28.dp, end = 28.dp, bottom = 22.dp)
                .offset(x = drift.first.dp, y = drift.second.dp)
                .onGloballyPositioned { coordinates ->
                    val top = coordinates.positionInRoot().y - driftYPx
                    if (footerTopPx.isNaN() || kotlin.math.abs(footerTopPx - top) > 0.5f)
                        footerTopPx = top
                }
        )
        VolumeGestureArea(
            Modifier.align(Alignment.CenterEnd).fillMaxWidth(1f / 3f).fillMaxHeight()
                .windowInsetsPadding(WindowInsets.systemGestures)
                .padding(top = 64.dp, end = 16.dp, bottom = 48.dp)
        )
        TextButton(
            onClick = onExit,
            enabled = exitVisible,
            modifier = Modifier.align(Alignment.TopEnd).padding(top = 6.dp, end = 10.dp)
                .graphicsLayer { alpha = exitAlpha }
        ) {
            Text(stringResource(R.string.aod_exit), color = Color(0x55FFFFFF), fontSize = 13.sp)
        }
    }
}

/** Fraction of the gesture-area height that spans the full media-volume range. */
private const val VOLUME_GESTURE_TRAVEL = 0.60f

/** Left channel (upper trace) and right channel (lower trace) keep the existing two-tone palette. */
private val LEFT_CHANNEL_COLOR = Color(0xFF67E8F9)
private val RIGHT_CHANNEL_COLOR = Color(0xFF22D3EE)

@Composable
private fun VolumeGestureArea(modifier: Modifier) {
    val context = LocalContext.current
    val audioManager = remember(context) { context.getSystemService(AudioManager::class.java) }
    Box(modifier.pointerInput(audioManager) {
        // No touch-slop wait: this area only adjusts volume, so the first moved pixel already counts
        // and a short swipe reaches the next level. detectVerticalDragGestures delayed both.
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            if (audioManager.isVolumeFixed) {
                Toast.makeText(context, R.string.aod_volume_fixed, Toast.LENGTH_SHORT).show()
                return@awaitEachGesture
            }
            val minimum = if (Build.VERSION.SDK_INT >= 28)
                audioManager.getStreamMinVolume(AudioManager.STREAM_MUSIC) else 0
            val drag = AodVolumeDrag(
                audioManager.getStreamVolume(AudioManager.STREAM_MUSIC), minimum,
                audioManager.getStreamMaxVolume(AudioManager.STREAM_MUSIC),
                size.height * VOLUME_GESTURE_TRAVEL
            )
            while (true) {
                val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                if (!change.pressed) break
                val delta = change.positionChange().y
                if (delta == 0f) continue
                change.consume()
                val target = drag.move(delta) ?: continue
                try {
                    audioManager.setStreamVolume(AudioManager.STREAM_MUSIC, target, AudioManager.FLAG_SHOW_UI)
                } catch (error: SecurityException) {
                    android.util.Log.w("AnPodAod", "Media volume adjustment denied", error)
                    Toast.makeText(context, R.string.aod_volume_unavailable, Toast.LENGTH_SHORT).show()
                    break
                }
            }
        }
    })
}

/** Retains sub-step movement and discards overscroll at the phone's volume limits. */
internal class AodVolumeDrag(
    initial: Int,
    private val minimum: Int,
    private val maximum: Int,
    private val heightPixels: Float
) {
    private var level = initial.toFloat()
    private var requested = initial

    fun move(deltaY: Float): Int? {
        level = (level - deltaY / heightPixels * (maximum - minimum)).coerceIn(minimum.toFloat(), maximum.toFloat())
        val target = level.roundToInt()
        if (target == requested) return null
        requested = target
        return target
    }
}

@Composable
private fun WaveformCanvas(waveform: PcmWaveformTap.Waveform?, maximumAmplitude: Float, modifier: Modifier) {
    Canvas(modifier) {
        val center = size.height / 2f
        fun ribbonBrush(color: Color) = Brush.horizontalGradient(
            0f to color.copy(alpha = 0f),
            AodWaveformGeometry.EDGE_FADE_FRACTION to color,
            (1f - AodWaveformGeometry.EDGE_FADE_FRACTION) to color,
            1f to color.copy(alpha = 0f),
            endX = size.width
        )
        val leftBrush = ribbonBrush(LEFT_CHANNEL_COLOR)
        val rightBrush = ribbonBrush(RIGHT_CHANNEL_COLOR)
        if (waveform == null || waveform.silent || maximumAmplitude <= 0f
            || (waveform.left.all { it == 0f } && waveform.right.all { it == 0f })) {
            drawLine(leftBrush, Offset(0f, center), Offset(size.width, center), 2f)
            drawLine(rightBrush, Offset(0f, center), Offset(size.width, center), 2f)
            return@Canvas
        }
        // Both traces share one baseline. The right channel is inverted, so identical stereo audio
        // shows one curve above the baseline and its mirror below instead of overlapping lines.
        val left = smoothWaveformPath(AodWaveformGeometry.centeredCurve(
            waveform.left, maximumAmplitude, waveform.sampledAtNanos, false
        ), center)
        val right = smoothWaveformPath(AodWaveformGeometry.centeredCurve(
            waveform.right, maximumAmplitude, waveform.sampledAtNanos, true
        ), center)
        drawPath(left, leftBrush, alpha = 0.15f, style = Stroke(AodWaveformGeometry.GLOW_STROKE_WIDTH, cap = StrokeCap.Butt, join = StrokeJoin.Round))
        drawPath(right, rightBrush, alpha = 0.15f, style = Stroke(AodWaveformGeometry.GLOW_STROKE_WIDTH, cap = StrokeCap.Butt, join = StrokeJoin.Round))
        drawPath(left, leftBrush, style = Stroke(AodWaveformGeometry.STROKE_WIDTH, cap = StrokeCap.Butt, join = StrokeJoin.Round))
        drawPath(right, rightBrush, style = Stroke(AodWaveformGeometry.STROKE_WIDTH, cap = StrokeCap.Butt, join = StrokeJoin.Round))
    }
}

/** Catmull-Rom samples become one bounded Bezier path, avoiding visible segment end-caps. */
private fun DrawScope.smoothWaveformPath(offsets: FloatArray, centerY: Float): Path {
    val path = Path()
    if (offsets.size < 2) return path
    val stepX = size.width / (offsets.size - 1)
    fun y(index: Int) = centerY - offsets[index]
    path.moveTo(0f, y(0))
    for (index in 0 until offsets.size - 1) {
        val x1 = index * stepX
        val x2 = (index + 1) * stepX
        val y1 = y(index)
        val y2 = y(index + 1)
        val y0 = y((index - 1).coerceAtLeast(0))
        val y3 = y((index + 2).coerceAtMost(offsets.lastIndex))
        val low = minOf(y1, y2)
        val high = maxOf(y1, y2)
        val control1Y = (y1 + (y2 - y0) / 6f).coerceIn(low, high)
        val control2Y = (y2 - (y3 - y1) / 6f).coerceIn(low, high)
        path.cubicTo(x1 + stepX / 3f, control1Y, x2 - stepX / 3f, control2Y, x2, y2)
    }
    return path
}

@Composable
private fun MetricsRow(telemetry: AodTelemetrySnapshot?, modifier: Modifier) {
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceEvenly
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            StatusDot(telemetry)
            Text(statusLabel(telemetry), color = Color(0xFFE6E6E6), fontSize = 14.sp)
        }
        Divider()
        Text(protocolLabel(telemetry), color = Color(0xFFB3B3B3), fontSize = 14.sp)
        Divider()
        Text("Recv ${rateLabel(telemetry?.recvAudioPayloadBps)}", color = Color(0xFFB3B3B3), fontSize = 14.sp)
        Divider()
        Text("Latency ${latencyLabel(telemetry?.receiverBufferMs)}", color = Color(0xFFB3B3B3), fontSize = 14.sp)
        Divider()
        Text("Loss ${lossLabel(telemetry?.udpLossPercent)}", color = Color(0xFFB3B3B3), fontSize = 14.sp)
    }
}

@Composable
private fun StatusDot(telemetry: AodTelemetrySnapshot?) {
    val color = when (telemetry?.status) {
        AodTelemetrySnapshot.DisplayStatus.RECEIVING -> Color(0xFF22D3EE)
        AodTelemetrySnapshot.DisplayStatus.CONNECTED -> Color(0xFF0891B2)
        AodTelemetrySnapshot.DisplayStatus.RECONNECTING -> Color(0xFFF59E0B)
        else -> Color(0xFF4B5563)
    }
    Box(Modifier.size(8.dp).background(color, CircleShape))
}

@Composable
private fun Divider() {
    Text("|", color = Color(0xFF2F3B3F), fontSize = 14.sp)
}

private fun statusLabel(telemetry: AodTelemetrySnapshot?): String = when (telemetry?.status) {
    AodTelemetrySnapshot.DisplayStatus.RECEIVING -> "Receiving"
    AodTelemetrySnapshot.DisplayStatus.CONNECTED -> "Connected"
    AodTelemetrySnapshot.DisplayStatus.RECONNECTING -> "Reconnecting"
    else -> "Disconnected"
}

private fun protocolLabel(telemetry: AodTelemetrySnapshot?): String = when (telemetry?.transport) {
    AodTelemetrySnapshot.Transport.WIFI_UDP -> "UDP"
    AodTelemetrySnapshot.Transport.USB_ADB -> "USB ADB"
    AodTelemetrySnapshot.Transport.WIFI_ADB -> "Wi-Fi ADB"
    else -> "—"
}

private fun rateLabel(bitsPerSecond: Long?): String = when {
    bitsPerSecond == null -> "—"
    bitsPerSecond >= 1_000_000 -> String.format(Locale.US, "%.2f Mbps", bitsPerSecond / 1_000_000.0)
    else -> String.format(Locale.US, "%.0f kbps", bitsPerSecond / 1_000.0)
}

private fun latencyLabel(millis: Float?): String =
    millis?.let { String.format(Locale.US, "%.0f ms", it) } ?: "—"

private fun lossLabel(percent: Float?): String =
    percent?.let { String.format(Locale.US, "%.1f%%", it) } ?: "—"

private val RubikClock = FontFamily(Font(R.font.rubik_black, FontWeight.Black))

@Composable
private fun rememberDrift(): Pair<Float, Float> {
    var offset by remember { mutableStateOf(0f to 0f) }
    LaunchedEffect(Unit) {
        while (isActive) {
            val seconds = SystemClock.elapsedRealtime() / 1_000.0
            offset = AodDrift.xDp(seconds) to AodDrift.yDp(seconds)
            delay(1_000)
        }
    }
    return offset
}
