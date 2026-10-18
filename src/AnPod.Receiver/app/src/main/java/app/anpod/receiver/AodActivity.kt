package app.anpod.receiver

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Bundle
import android.os.IBinder
import android.os.SystemClock
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
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
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import java.util.Calendar
import java.util.Locale
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.sin
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

/** Full-screen pure-black pseudo-AOD: clock, real PCM waveform, five receive metrics. */
class AodActivity : ComponentActivity() {
    private var binder by mutableStateOf<ReceiverUiBinder?>(null)
    private var telemetry by mutableStateOf<AodTelemetrySnapshot?>(null)
    private var waveform by mutableStateOf<PcmWaveformTap.Waveform?>(null)
    private var previousBrightness = -1f
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
        previousBrightness = window.attributes.screenBrightness
        window.attributes = window.attributes.apply { screenBrightness = 0.05f }
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
                        if (SystemClock.elapsedRealtime() - disconnectedAt > 3_000L) finish()
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
        if (previousBrightness >= 0f) {
            window.attributes = window.attributes.apply { screenBrightness = previousBrightness }
        }
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
    BoxWithConstraints(Modifier.fillMaxSize().background(Color.Black)) {
        val clockSize = (maxHeight.value * 0.40f).coerceIn(96f, 184f)
        val waveformHeight = (maxHeight.value * 0.24f).coerceIn(64f, 132f)
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
        )
        WaveformCanvas(
            waveform,
            Modifier.align(Alignment.Center)
                .fillMaxWidth(0.86f)
                .height(waveformHeight.dp)
                .offset(x = drift.first.dp, y = drift.second.dp)
        )
        MetricsRow(
            telemetry,
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .padding(start = 28.dp, end = 28.dp, bottom = 22.dp)
        )
        TextButton(
            onClick = onExit,
            modifier = Modifier.align(Alignment.TopEnd).padding(top = 6.dp, end = 10.dp)
        ) {
            Text(stringResource(R.string.aod_exit), color = Color(0x55FFFFFF), fontSize = 13.sp)
        }
    }
}

@Composable
private fun WaveformCanvas(waveform: PcmWaveformTap.Waveform?, modifier: Modifier) {
    Canvas(modifier) {
        val center = size.height / 2f
        drawRibbon(waveform?.left, waveform?.right, center, Color(0xFF22D3EE))
    }
}

/** The two lines overlap at center when silent and bend independently around it when audio plays. */
private fun DrawScope.drawRibbon(left: FloatArray?, right: FloatArray?, centerY: Float, color: Color) {
    if (left == null || right == null || left.size < 2 || right.size < 2) return
    val count = minOf(left.size, right.size)
    val amplitude = size.height * 0.46f
    for (index in 0 until count - 1) {
        val x1 = size.width * index / (count - 1)
        val x2 = size.width * (index + 1) / (count - 1)
        val taper1 = sin(PI * index / (count - 1)).toFloat().pow(0.7f)
        val taper2 = sin(PI * (index + 1) / (count - 1)).toFloat().pow(0.7f)
        val up1 = (left[index] * 2.2f).coerceIn(0f, 1f) * taper1 * amplitude
        val up2 = (left[index + 1] * 2.2f).coerceIn(0f, 1f) * taper2 * amplitude
        val down1 = (right[index] * 2.2f).coerceIn(0f, 1f) * taper1 * amplitude
        val down2 = (right[index + 1] * 2.2f).coerceIn(0f, 1f) * taper2 * amplitude
        drawLine(color.copy(alpha = 0.14f), Offset(x1, centerY - up1), Offset(x2, centerY - up2),
            strokeWidth = 10f, cap = StrokeCap.Round)
        drawLine(color.copy(alpha = 0.14f), Offset(x1, centerY + down1), Offset(x2, centerY + down2),
            strokeWidth = 10f, cap = StrokeCap.Round)
        drawLine(color, Offset(x1, centerY - up1), Offset(x2, centerY - up2),
            strokeWidth = 2.4f, cap = StrokeCap.Round)
        drawLine(Color(0xFF0891B2), Offset(x1, centerY + down1), Offset(x2, centerY + down2),
            strokeWidth = 2.4f, cap = StrokeCap.Round)
    }
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
            offset = (sin(seconds / 120.0 * 2 * PI) * 9.0).toFloat() to
                (cos(seconds / 150.0 * 2 * PI) * 4.0).toFloat()
            delay(1_000)
        }
    }
    return offset
}
