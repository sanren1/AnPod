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
import kotlin.math.cos
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
                    binder?.let { waveform = it.aodWaveform() }
                    delay(33)
                }
            }
            LaunchedEffect(binder) {
                var disconnectedAt = 0L
                while (isActive) {
                    val current = binder?.aodSnapshot()
                    telemetry = current
                    if (current == null || current.status == AodTelemetrySnapshot.DisplayStatus.DISCONNECTED) {
                        if (disconnectedAt == 0L) disconnectedAt = SystemClock.elapsedRealtime()
                        if (SystemClock.elapsedRealtime() - disconnectedAt > 3_000L) finish()
                    } else {
                        disconnectedAt = 0L
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
    Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.offset(x = drift.x.dp, y = drift.y.dp)
        ) {
            Text(
                text = String.format(Locale.US, "%02d:%02d", now.get(Calendar.HOUR_OF_DAY), now.get(Calendar.MINUTE)),
                fontFamily = RubikClock,
                fontWeight = FontWeight.ExtraBold,
                fontSize = 168.sp,
                letterSpacing = (-6).sp,
                color = Color(0xFFF2F2F2)
            )
            Spacer(Modifier.height(18.dp))
            WaveformCanvas(waveform, Modifier.fillMaxWidth(0.82f).height(120.dp))
            Spacer(Modifier.height(26.dp))
            MetricsRow(telemetry)
        }
        TextButton(
            onClick = onExit,
            modifier = Modifier.align(Alignment.BottomEnd).padding(20.dp)
        ) {
            Text(stringResource(R.string.aod_exit), color = Color(0x66FFFFFF), fontSize = 14.sp)
        }
    }
}

@Composable
private fun WaveformCanvas(waveform: PcmWaveformTap.Waveform?, modifier: Modifier) {
    Canvas(modifier) {
        val left = waveform?.left
        val right = waveform?.right
        drawEnvelope(left, size.height * 0.48f, Color(0xFF22D3EE), 0.40f)
        drawEnvelope(right, size.height * 0.54f, Color(0xFF0891B2), 0.34f)
    }
}

private fun DrawScope.drawEnvelope(values: FloatArray?, centerY: Float, color: Color, amplitude: Float) {
    if (values == null || values.isEmpty()) return
    val path = Path()
    val step = size.width / (values.size - 1).coerceAtLeast(1)
    val points = ArrayList<Offset>(values.size)
    for (index in values.indices) {
        val x = step * index
        val y = centerY - values[index].coerceIn(0f, 1f) * size.height * amplitude
        points.add(Offset(x, y))
    }
    path.moveTo(points.first().x, points.first().y)
    for (index in 0 until points.size - 1) {
        val start = points[index]
        val end = points[index + 1]
        val controlX = (start.x + end.x) / 2f
        path.cubicTo(controlX, start.y, controlX, end.y, end.x, end.y)
    }
    drawPath(path, color.copy(alpha = 0.16f), style = Stroke(width = 12f, cap = StrokeCap.Round))
    drawPath(path, color, style = Stroke(width = 2.5f, cap = StrokeCap.Round))
}

@Composable
private fun MetricsRow(telemetry: AodTelemetrySnapshot?) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(18.dp)
    ) {
        StatusDot(telemetry)
        Text(statusLabel(telemetry), color = Color(0xFFE6E6E6), fontSize = 17.sp)
        Divider()
        Text(protocolLabel(telemetry), color = Color(0xFFB3B3B3), fontSize = 17.sp)
        Divider()
        Text("Recv ${rateLabel(telemetry?.recvAudioPayloadBps)}", color = Color(0xFFB3B3B3), fontSize = 17.sp)
        Divider()
        Text("Latency ${latencyLabel(telemetry?.receiverBufferMs)}", color = Color(0xFFB3B3B3), fontSize = 17.sp)
        Divider()
        Text("Loss ${lossLabel(telemetry?.udpLossPercent)}", color = Color(0xFFB3B3B3), fontSize = 17.sp)
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
    Box(Modifier.size(10.dp).background(color, CircleShape))
}

@Composable
private fun Divider() {
    Text("|", color = Color(0xFF2F3B3F), fontSize = 17.sp)
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

private val RubikClock = FontFamily(Font(R.font.rubik_variable, FontWeight.ExtraBold))

@Composable
private fun rememberDrift(): Offset {
    var offset by remember { mutableStateOf(Offset.Zero) }
    LaunchedEffect(Unit) {
        while (isActive) {
            val seconds = SystemClock.elapsedRealtime() / 1_000.0
            offset = Offset(
                (sin(seconds / 120.0 * 2 * Math.PI) * 10.0).toFloat(),
                (cos(seconds / 150.0 * 2 * Math.PI) * 5.0).toFloat()
            )
            delay(1_000)
        }
    }
    return offset
}
