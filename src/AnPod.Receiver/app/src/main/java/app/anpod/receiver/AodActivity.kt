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
import android.util.DisplayMetrics
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
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.systemGestures
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
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
import androidx.compose.ui.layout.positionInParent
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalConfiguration
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
    private var displaySettings by mutableStateOf(AodDisplaySettings())
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
        val preferences = getSharedPreferences("anpod_receiver", MODE_PRIVATE)
        displaySettings = AodDisplaySettings.read(preferences)
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
                settings = displaySettings,
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
    settings: AodDisplaySettings,
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
    val clockText = String.format(Locale.US, "%02d:%02d", now.get(Calendar.HOUR_OF_DAY), now.get(Calendar.MINUTE))
    AodVisualLayout(telemetry, waveform, settings, clockText, drift.first, drift.second,
        exitAlpha, exitVisible, onExit) { lastInteraction = SystemClock.elapsedRealtime() }
}

@Composable
private fun AodVisualLayout(
    telemetry: AodTelemetrySnapshot?,
    waveform: PcmWaveformTap.Waveform?,
    settings: AodDisplaySettings,
    clockText: String,
    driftX: Float,
    driftY: Float,
    exitAlpha: Float,
    exitVisible: Boolean,
    onExit: (() -> Unit)?,
    onInteraction: (() -> Unit)? = null
) {
    BoxWithConstraints(
        Modifier.fillMaxSize().background(Color.Black).then(
            if (onInteraction != null) Modifier.pointerInput(Unit) {
                awaitPointerEventScope {
                    while (true) {
                        awaitPointerEvent(PointerEventPass.Initial)
                        onInteraction()
                    }
                }
            } else Modifier
        )
    ) {
        val baseClockSize = (maxHeight.value * 0.40f).coerceIn(96f, 184f)
        val density = LocalDensity.current
        val textMeasurer = rememberTextMeasurer()
        val clockBaseStyle = clockTextStyle(settings.clockFont, settings.clockWeight, baseClockSize.sp)
        val screenHeightPx = with(density) { maxHeight.toPx() }
        val clearancePx = with(density) { 4.dp.toPx() }
        var footerTopPx by remember { mutableStateOf(Float.NaN) }
        val footerTop = if (footerTopPx.isFinite()) footerTopPx else screenHeightPx * 0.88f
        val maximumDriftY = with(density) { AodDrift.MAX_Y_DP.dp.toPx() }
        val clockTopPaddingPx = screenHeightPx * 0.10f
        val (clockBottomPx, maximumClockSize) = remember(
            textMeasurer, clockBaseStyle, density.density, density.fontScale, maxWidth, maxHeight,
            footerTop
        ) {
            fun measureClock(text: String, style: TextStyle, measureDensity: Density) = textMeasurer.measure(
                AnnotatedString(text),
                style = style,
                density = measureDensity,
                maxLines = 1,
                softWrap = false
            )

            val referenceStyle = clockTextStyle(AodDisplaySettings.CLOCK_FONT_RUBIK,
                AodDisplaySettings.CLOCK_WEIGHT_BLACK, baseClockSize.sp)
            val referenceMeasurement = measureClock("04:44", referenceStyle,
                Density(density.density, fontScale = 1f))
            val clockBottom = AodWaveformGeometry.clockBottom(
                screenHeightPx,
                referenceMeasurement.size.height.toFloat(),
                footerTop,
                maximumDriftY,
                clearancePx,
                with(density) { 64.dp.toPx() }
            )
            val maximumWidth = (with(density) { (maxWidth * 0.92f).toPx() } -
                with(density) { (2 * AodDrift.MAX_X_DP).dp.toPx() }).coerceAtLeast(0f)
            val maximumHeight = (clockBottom - with(density) { (16 + AodDrift.MAX_Y_DP).dp.toPx() })
                .coerceAtLeast(0f)
            val fitCeiling = baseClockSize * 1.2f
            val fitStyle = clockBaseStyle.copy(fontSize = fitCeiling.sp)
            val widestTime = (0..23).asSequence().flatMap { hour ->
                (0..59).asSequence().map { minute -> String.format(Locale.US, "%02d:%02d", hour, minute) }
            }.map { measureClock(it, fitStyle, density).size }
                .fold(androidx.compose.ui.unit.IntSize.Zero) { widest, size ->
                    androidx.compose.ui.unit.IntSize(maxOf(widest.width, size.width), maxOf(widest.height, size.height))
                }
            val sizeAtBase = AodDisplaySettings.fitClockSizeSp(
                fitCeiling,
                maximumWidth.toInt(),
                maximumHeight.toInt()
            ) { size -> AodDisplaySettings.scaleClockMeasurement(size, fitCeiling, widestTime) }
            clockBottom to sizeAtBase
        }
        val clockSize = AodDisplaySettings.scaledClockSizeSp(maximumClockSize, settings.clockScale)
        val clockHeightPx = remember(textMeasurer, clockBaseStyle, clockSize, density.density,
            density.fontScale, clockText) {
            textMeasurer.measure(AnnotatedString(clockText),
                style = clockBaseStyle.copy(fontSize = clockSize.sp), density = density,
                maxLines = 1, softWrap = false).size.height.toFloat()
        }
        val driftYPx = with(density) { driftY.dp.toPx() }
        val waveformLayout = AodWaveformGeometry.layout(
            clockBottomPx,
            footerTop,
            maximumDriftY,
            clearancePx
        )
        val waveformHeight = with(density) { waveformLayout.regionHeight.toDp() }
        val waveformTop = with(density) { (waveformLayout.centerY - waveformLayout.regionHeight / 2f).toDp() }
        Text(
            text = clockText,
            style = clockBaseStyle.copy(fontSize = clockSize.sp),
            maxLines = 1,
            softWrap = false,
            color = Color(0xFFF7F7F7),
            modifier = Modifier.align(Alignment.TopCenter)
                .padding(top = (maxHeight.value * 0.10f).dp)
                .offset(x = driftX.dp, y = driftY.dp + with(density) {
                    AodWaveformGeometry.clockOffsetY(
                        clockBottomPx,
                        clockTopPaddingPx,
                        clockHeightPx
                    ).toDp()
                })
        )
        WaveformCanvas(
            waveform,
            waveformLayout.maximumAmplitude,
            settings.waveformStrokeScale,
            Modifier.align(Alignment.TopCenter)
                .offset(y = waveformTop)
                .fillMaxWidth(0.90f)
                .height(waveformHeight)
                .offset(x = driftX.dp, y = driftY.dp)
        )
        MetricsRow(
            telemetry, settings,
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .padding(start = 28.dp, end = 28.dp, bottom = 22.dp)
                .offset(x = driftX.dp, y = driftY.dp)
                .onGloballyPositioned { coordinates ->
                    val top = coordinates.positionInParent().y - driftYPx
                    if (footerTopPx.isNaN() || kotlin.math.abs(footerTopPx - top) > 0.5f) footerTopPx = top
                }
        )
        if (onExit != null) {
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
            ) { Text("Close", color = Color(0x55FFFFFF), fontSize = 13.sp) }
        }
    }
}

@Composable
internal fun AodSettingsDialog(
    settings: AodDisplaySettings,
    onChange: (AodDisplaySettings) -> Unit,
    onClose: () -> Unit
) {
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text(stringResource(R.string.aod_settings)) },
        text = {
            BoxWithConstraints(Modifier.heightIn(max = 480.dp)) {
                val contentHeight = minOf(maxHeight, 480.dp)
                Column(Modifier.height(contentHeight)) {
                    AodSettingsPreview(
                        settings,
                        Modifier.height(aodSettingsPreviewHeight(contentHeight))
                    )
                    Column(
                        Modifier.weight(1f).verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        Text(stringResource(R.string.aod_clock_size), style = MaterialTheme.typography.titleMedium)
                        val clockSizeLabel = stringResource(R.string.aod_clock_size)
                        Slider(value = settings.clockScale.toFloat(), onValueChange = {
                            onChange(settings.copy(clockScale = it.toInt().coerceIn(80, 120)))
                        }, valueRange = 80f..120f, steps = 7,
                            modifier = Modifier.semantics { contentDescription = clockSizeLabel })
                        Text(stringResource(R.string.aod_clock_size_value, settings.clockScale))
                        Text(stringResource(R.string.aod_clock_font), style = MaterialTheme.typography.titleMedium)
                        val clockFonts = listOf(
                            AodDisplaySettings.CLOCK_FONT_RUBIK to R.string.aod_font_rubik,
                            AodDisplaySettings.CLOCK_FONT_SANS_SERIF to R.string.aod_font_sans_serif,
                            AodDisplaySettings.CLOCK_FONT_SERIF to R.string.aod_font_serif,
                            AodDisplaySettings.CLOCK_FONT_MONOSPACE to R.string.aod_font_monospace
                        )
                        var fontMenuExpanded by remember { mutableStateOf(false) }
                        Column {
                            OutlinedButton(
                                onClick = { fontMenuExpanded = true },
                                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                            ) {
                                Text(stringResource(clockFonts.first { it.first == settings.clockFont }.second))
                            }
                            DropdownMenu(
                                expanded = fontMenuExpanded,
                                onDismissRequest = { fontMenuExpanded = false }
                            ) {
                                clockFonts.forEach { (font, label) ->
                                    DropdownMenuItem(
                                        text = { Text(stringResource(label)) },
                                        onClick = {
                                            onChange(settings.copy(clockFont = font))
                                            fontMenuExpanded = false
                                        },
                                        modifier = Modifier.heightIn(min = 48.dp)
                                    )
                                }
                            }
                        }
                        Text(stringResource(R.string.aod_clock_weight), style = MaterialTheme.typography.titleMedium)
                        val clockWeightLabel = stringResource(R.string.aod_clock_weight)
                        Slider(
                            value = settings.clockWeightLevel.toFloat(),
                            onValueChange = { onChange(settings.copy(clockWeightLevel = it.roundToInt())) },
                            valueRange = 1f..9f,
                            steps = 7,
                            modifier = Modifier.semantics { contentDescription = clockWeightLabel }
                        )
                        Text(settings.clockWeight.toString())
                        Text(stringResource(R.string.aod_waveform_weight), style = MaterialTheme.typography.titleMedium)
                        val waveformWeightLabel = stringResource(R.string.aod_waveform_weight)
                        Slider(
                            value = settings.waveformWeight.toFloat(),
                            onValueChange = { onChange(settings.copy(waveformWeight = it.roundToInt())) },
                            valueRange = 80f..120f,
                            steps = 7,
                            modifier = Modifier.semantics { contentDescription = waveformWeightLabel }
                        )
                        Text(stringResource(R.string.aod_waveform_weight_value, settings.waveformStrokeScale))
                        Text(stringResource(R.string.aod_detail_metrics), style = MaterialTheme.typography.titleMedium)
                        AodSettingSwitch(R.string.aod_metric_protocol, settings.showProtocol) {
                            onChange(settings.copy(showProtocol = it))
                        }
                        AodSettingSwitch(R.string.aod_metric_receive_rate, settings.showReceiveRate) {
                            onChange(settings.copy(showReceiveRate = it))
                        }
                        AodSettingSwitch(R.string.aod_metric_buffer, settings.showBuffer) {
                            onChange(settings.copy(showBuffer = it))
                        }
                        AodSettingSwitch(R.string.aod_metric_loss, settings.showLoss) {
                            onChange(settings.copy(showLoss = it))
                        }
                        AodSettingSwitch(R.string.aod_metric_audio_format, settings.showAudioFormat) {
                            onChange(settings.copy(showAudioFormat = it))
                        }
                        AodSettingSwitch(R.string.aod_metric_underruns, settings.showUnderruns) {
                            onChange(settings.copy(showUnderruns = it))
                        }
                        AodSettingSwitch(R.string.aod_metric_output_engine, settings.showOutputEngine) {
                            onChange(settings.copy(showOutputEngine = it))
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onClose, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(R.string.aod_settings_close))
            }
        }
    )
}

internal fun aodSettingsPreviewHeight(availableHeight: Dp): Dp = minOf(140.dp, availableHeight * 0.30f)

@Composable
private fun AodSettingsPreview(settings: AodDisplaySettings, modifier: Modifier) {
    val context = LocalContext.current
    val configuration = LocalConfiguration.current
    val density = LocalDensity.current
    val (screenWidth, screenHeight) = remember(configuration, density) {
        val windowManager = context.getSystemService(WindowManager::class.java)
        val (widthPx, heightPx) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bounds = windowManager.currentWindowMetrics.bounds
            bounds.width() to bounds.height()
        } else {
            val metrics = DisplayMetrics()
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels to metrics.heightPixels
        }
        with(density) {
            maxOf(widthPx.toDp().value, heightPx.toDp().value) to
                minOf(widthPx.toDp().value, heightPx.toDp().value)
        }
    }
    val waveform = remember {
        val channel = FloatArray(PcmWaveformTap.POINTS) { index ->
            val envelope = sin(index * 0.23f)
            0.22f + 0.65f * envelope * envelope
        }
        PcmWaveformTap.Waveform(channel, channel.copyOf(), 0L, false)
    }
    BoxWithConstraints(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        val scale = minOf(maxWidth.value / screenWidth, maxHeight.value / screenHeight)
        Box(
            Modifier.requiredSize(screenWidth.dp * scale, screenHeight.dp * scale)
                .background(Color.Black, RoundedCornerShape(8.dp))
        ) {
            Box(
                Modifier.requiredSize(screenWidth.dp, screenHeight.dp)
                    .align(Alignment.Center)
                    .graphicsLayer(scaleX = scale, scaleY = scale)
            ) {
                AodVisualLayout(null, waveform, settings, "21:38", 0f, 0f, 0f, false, null)
            }
        }
    }
}

@Composable
private fun AodSettingSwitch(label: Int, checked: Boolean, onCheckedChange: (Boolean) -> Unit) {
    val labelText = stringResource(label)
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(labelText, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onCheckedChange,
            modifier = Modifier.semantics { contentDescription = labelText })
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
                Toast.makeText(context, "This device's media volume is fixed by the system.", Toast.LENGTH_SHORT).show()
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
                    Toast.makeText(context, "Android does not allow volume changes here. Use the volume buttons.", Toast.LENGTH_SHORT).show()
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
private fun WaveformCanvas(waveform: PcmWaveformTap.Waveform?, maximumAmplitude: Float, strokeScale: Float, modifier: Modifier) {
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
            drawLine(leftBrush, Offset(0f, center), Offset(size.width, center), 2f * strokeScale)
            drawLine(rightBrush, Offset(0f, center), Offset(size.width, center), 2f * strokeScale)
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
        drawPath(left, leftBrush, alpha = 0.15f, style = Stroke(AodWaveformGeometry.GLOW_STROKE_WIDTH * strokeScale, cap = StrokeCap.Butt, join = StrokeJoin.Round))
        drawPath(right, rightBrush, alpha = 0.15f, style = Stroke(AodWaveformGeometry.GLOW_STROKE_WIDTH * strokeScale, cap = StrokeCap.Butt, join = StrokeJoin.Round))
        drawPath(left, leftBrush, style = Stroke(AodWaveformGeometry.STROKE_WIDTH * strokeScale, cap = StrokeCap.Butt, join = StrokeJoin.Round))
        drawPath(right, rightBrush, style = Stroke(AodWaveformGeometry.STROKE_WIDTH * strokeScale, cap = StrokeCap.Butt, join = StrokeJoin.Round))
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
private fun MetricsRow(telemetry: AodTelemetrySnapshot?, settings: AodDisplaySettings, modifier: Modifier) {
    val items = buildList {
        if (settings.showProtocol) add("Protocol ${protocolLabel(telemetry)}")
        if (settings.showReceiveRate) add("Recv ${rateLabel(telemetry?.recvAudioPayloadBps)}")
        if (settings.showBuffer) add("Buffer ${latencyLabel(telemetry?.receiverBufferMs)}")
        if (settings.showLoss) add("Loss ${lossLabel(telemetry?.lossPercent)}")
        if (settings.showAudioFormat) add("Format ${audioFormatLabel(telemetry)}")
        if (settings.showUnderruns) add("Underruns ${countLabel(telemetry?.underrunCount)}")
        if (settings.showOutputEngine) add("Output ${outputEngineLabel(telemetry)}")
    }
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            StatusDot(telemetry)
            Text(statusLabel(telemetry), color = Color(0xFFE6E6E6), fontSize = 14.sp)
        }
        if (items.isNotEmpty()) Layout(
            content = {
                items.forEachIndexed { index, item ->
                    Text(item, color = Color(0xFFB3B3B3), fontSize = 14.sp, textAlign = TextAlign.Center)
                    if (index < items.lastIndex) Text("|", color = Color(0xFFB3B3B3), fontSize = 14.sp)
                }
            },
            modifier = Modifier.fillMaxWidth()
        ) { measurables, constraints ->
            val textConstraints = constraints.copy(minWidth = 0, minHeight = 0)
            val itemsMeasured = items.indices.map { measurables[it * 2].measure(textConstraints) }
            val separatorsMeasured = (0 until items.lastIndex).map { measurables[it * 2 + 1].measure(textConstraints) }
            val separatorPadding = 8.dp.roundToPx()
            val separatorWidth = separatorsMeasured.firstOrNull()?.width?.plus(separatorPadding * 2) ?: 0
            val rows = AodMetricsGeometry.wrap(itemsMeasured.map { it.width }.toIntArray(), separatorWidth, constraints.maxWidth)
            val rowHeights = rows.map { (start, end) ->
                (start until end).maxOf { itemsMeasured[it].height }
            }
            val rowSpacing = 2.dp.roundToPx()
            val height = rowHeights.sum() + rowSpacing * (rows.size - 1).coerceAtLeast(0)
            layout(constraints.maxWidth, height.coerceIn(constraints.minHeight, constraints.maxHeight)) {
                var y = 0
                rows.forEachIndexed { rowIndex, (start, end) ->
                    val count = end - start
                    val naturalWidth = (start until end).sumOf { itemsMeasured[it].width } +
                        separatorWidth * (count - 1)
                    val gap = if (count > 1) (constraints.maxWidth - naturalWidth).coerceAtLeast(0) / (count - 1) else 0
                    var x = if (count == 1) (constraints.maxWidth - naturalWidth).coerceAtLeast(0) / 2 else 0
                    for (index in start until end) {
                        val item = itemsMeasured[index]
                        item.placeRelative(x, y + (rowHeights[rowIndex] - item.height) / 2)
                        x += item.width
                        if (index < end - 1) {
                            val separator = separatorsMeasured[index]
                            separator.placeRelative(
                                x + separatorPadding + AodMetricsGeometry.gapBeforeSeparator(gap),
                                y + (rowHeights[rowIndex] - separator.height) / 2
                            )
                            x += separatorWidth + gap
                        }
                    }
                    y += rowHeights[rowIndex] + rowSpacing
                }
            }
        }
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

private fun statusLabel(telemetry: AodTelemetrySnapshot?): String = when (telemetry?.status) {
    AodTelemetrySnapshot.DisplayStatus.RECEIVING -> "Receiving"
    AodTelemetrySnapshot.DisplayStatus.CONNECTED -> "Connected"
    AodTelemetrySnapshot.DisplayStatus.RECONNECTING -> "Reconnecting"
    else -> "Not connected"
}

private fun protocolLabel(telemetry: AodTelemetrySnapshot?): String = when (telemetry?.transport) {
    AodTelemetrySnapshot.Transport.USB_DIRECT -> "USB Direct"
    AodTelemetrySnapshot.Transport.WIFI_UDP -> "UDP"
    AodTelemetrySnapshot.Transport.USB_ADB, AodTelemetrySnapshot.Transport.WIFI_ADB -> "ADB"
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

private fun audioFormatLabel(telemetry: AodTelemetrySnapshot?): String {
    if (telemetry == null || telemetry.outputSampleRate <= 0 || telemetry.outputChannels <= 0) return "—"
    val channels = when (telemetry.outputChannels) {
        1 -> "Mono"
        2 -> "Stereo"
        else -> "${telemetry.outputChannels} ch"
    }
    val rate = if (telemetry.outputSampleRate % 1000 == 0)
        "${telemetry.outputSampleRate / 1000}"
    else String.format(Locale.US, "%.1f", telemetry.outputSampleRate / 1000.0)
    return "$rate kHz $channels"
}

private fun countLabel(count: Int?): String = count?.takeIf { it >= 0 }?.toString() ?: "—"

private fun outputEngineLabel(telemetry: AodTelemetrySnapshot?): String = when (telemetry?.actualOutputEngine) {
    "audioTrack" -> "AudioTrack"
    "oboeExperimental" -> "Oboe"
    else -> "—"
}

internal fun clockFontResource(font: Int, weight: Int): Int {
    val resources = when (font) {
        AodDisplaySettings.CLOCK_FONT_SANS_SERIF -> listOf(
            R.font.sans_w300, R.font.sans_w400, R.font.sans_w450, R.font.sans_w500,
            R.font.sans_w550, R.font.sans_w600, R.font.sans_w700, R.font.sans_w775, R.font.sans_w850
        )
        AodDisplaySettings.CLOCK_FONT_SERIF -> listOf(
            R.font.serif_w300, R.font.serif_w400, R.font.serif_w450, R.font.serif_w500,
            R.font.serif_w550, R.font.serif_w600, R.font.serif_w700, R.font.serif_w775, R.font.serif_w850
        )
        AodDisplaySettings.CLOCK_FONT_MONOSPACE -> listOf(
            R.font.mono_w300, R.font.mono_w400, R.font.mono_w450, R.font.mono_w500,
            R.font.mono_w550, R.font.mono_w600, R.font.mono_w700, R.font.mono_w775, R.font.mono_w850
        )
        else -> listOf(
            R.font.rubik_w300, R.font.rubik_regular, R.font.rubik_w450, R.font.rubik_w500,
            R.font.rubik_w550, R.font.rubik_w600, R.font.rubik_w700, R.font.rubik_w775, R.font.rubik_black
        )
    }
    return resources[AodDisplaySettings.CLOCK_WEIGHT_VALUES.indexOf(weight)]
}

private fun clockFontFamily(font: Int) = FontFamily(
    *AodDisplaySettings.CLOCK_WEIGHT_VALUES.map { weight ->
        Font(clockFontResource(font, weight), FontWeight(weight))
    }.toTypedArray()
)

private val RubikClock = clockFontFamily(AodDisplaySettings.CLOCK_FONT_RUBIK)
private val SansClock = clockFontFamily(AodDisplaySettings.CLOCK_FONT_SANS_SERIF)
private val SerifClock = clockFontFamily(AodDisplaySettings.CLOCK_FONT_SERIF)
private val MonospaceClock = clockFontFamily(AodDisplaySettings.CLOCK_FONT_MONOSPACE)

internal fun clockTextStyle(font: Int, weight: Int, size: androidx.compose.ui.unit.TextUnit) = TextStyle(
    fontFamily = when (font) {
        AodDisplaySettings.CLOCK_FONT_SANS_SERIF -> SansClock
        AodDisplaySettings.CLOCK_FONT_SERIF -> SerifClock
        AodDisplaySettings.CLOCK_FONT_MONOSPACE -> MonospaceClock
        else -> RubikClock
    },
    fontWeight = FontWeight(weight),
    fontSize = size,
    letterSpacing = if (font == AodDisplaySettings.CLOCK_FONT_RUBIK) (-5).sp else 0.sp
)

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
