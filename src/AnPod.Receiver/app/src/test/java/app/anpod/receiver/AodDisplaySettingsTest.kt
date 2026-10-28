package app.anpod.receiver

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.sp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import kotlin.math.roundToInt
import kotlin.math.ceil
import java.nio.ByteBuffer
import java.nio.ByteOrder
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight

@RunWith(RobolectricTestRunner::class)
class AodDisplaySettingsTest {
    @Test
    fun defaultsAndPreferencesRoundTrip() {
        val preferences = ApplicationProvider.getApplicationContext<Context>()
            .getSharedPreferences("aod_test", Context.MODE_PRIVATE)
        preferences.edit().clear().commit()

        assertEquals(AodDisplaySettings(), AodDisplaySettings.read(preferences))

        val selected = AodDisplaySettings(115, 120, false, true, false, true,
            clockFont = AodDisplaySettings.CLOCK_FONT_SERIF,
            clockWeight = AodDisplaySettings.CLOCK_WEIGHT_NORMAL)
        selected.save(preferences)
        assertEquals(selected, AodDisplaySettings.read(preferences))
    }

    @Test
    fun defaultsKeepEveryTelemetryValueVisible() {
        val settings = AodDisplaySettings()
        assertTrue(settings.showProtocol)
        assertTrue(settings.showReceiveRate)
        assertTrue(settings.showBuffer)
        assertTrue(settings.showLoss)
        assertEquals(false, settings.showAudioFormat)
        assertEquals(false, settings.showUnderruns)
        assertEquals(false, settings.showOutputEngine)
        assertEquals(AodDisplaySettings.CLOCK_FONT_RUBIK, settings.clockFont)
        assertEquals(AodDisplaySettings.CLOCK_WEIGHT_BOLD, settings.clockWeight)
        assertEquals(100, settings.clockScale)
    }

    @Test
    fun existingPreferencesKeepOldMetricsAndDefaultNewMetricsOff() {
        val preferences = ApplicationProvider.getApplicationContext<Context>()
            .getSharedPreferences("aod_legacy_test", Context.MODE_PRIVATE)
        preferences.edit().clear()
            .putBoolean("aod_show_protocol", false)
            .putBoolean("aod_show_receive_rate", true)
            .putBoolean("aod_show_buffer", false)
            .putBoolean("aod_show_loss", true)
            .commit()

        val settings = AodDisplaySettings.read(preferences)
        assertEquals(false, settings.showProtocol)
        assertEquals(true, settings.showReceiveRate)
        assertEquals(false, settings.showBuffer)
        assertEquals(true, settings.showLoss)
        assertEquals(false, settings.showAudioFormat)
        assertEquals(false, settings.showUnderruns)
        assertEquals(false, settings.showOutputEngine)
    }

    @Test
    fun requestedClockScaleAndLargeFontScaleKeepClockInsideItsFixedBand() {
        val screenHeight = 300f
        val footerTop = screenHeight * 0.88f
        val topPadding = screenHeight * 0.10f
        val topSafe = 25f
        val maxDrift = 9f
        val clearance = 4f
        val baseSizeSp = 120f
        val referenceHeight = 96f
        val bottom = AodWaveformGeometry.clockBottom(
            screenHeight, referenceHeight, footerTop, maxDrift, clearance, 64f)
        val baselineWaveform = AodWaveformGeometry.layout(bottom, footerTop, maxDrift, clearance)

        for (fontScale in listOf(1f, 1.3f, 2f)) {
            val measuredHeight = referenceHeight * fontScale
            for (clockScale in listOf(80, 100, 120)) {
                val requestedSize = baseSizeSp * clockScale / 100f
                fun measure(size: Float) = IntSize(
                    (150f * fontScale * size / baseSizeSp - 20f * fontScale).roundToInt(),
                    (measuredHeight * size / baseSizeSp).roundToInt()
                )
                val fittedSize = AodDisplaySettings.fitClockSizeSp(
                    requestedSize, 180, (bottom - topSafe).toInt(), ::measure)
                val renderedHeight = measure(fittedSize).height.toFloat()
                val offset = AodWaveformGeometry.clockOffsetY(bottom, topPadding, renderedHeight)
                val renderedTop = topPadding + offset
                val renderedBottom = renderedTop + renderedHeight
                val waveform = AodWaveformGeometry.layout(bottom, footerTop, maxDrift, clearance)

                assertTrue("fontScale=$fontScale scale=$clockScale", renderedTop >= topSafe - 0.01f)
                assertEquals(bottom, renderedBottom, 0.01f)
                assertTrue(waveform.regionHeight > 0f)
                assertEquals(baselineWaveform.regionHeight, waveform.regionHeight, 0f)
                assertEquals(baselineWaveform.maximumAmplitude, waveform.maximumAmplitude, 0f)
            }
        }
    }

    @Test
    fun measuredClockFitHonorsWidthAndHeightBounds() {
        val measure: (Float) -> IntSize = { size ->
            IntSize((size * 1.4f + 20f).roundToInt(), (size * 1.1f + 12f).roundToInt())
        }
        val fitted = AodDisplaySettings.fitClockSizeSp(120f, 180, 100, measure)
        val result = measure(fitted)

        assertTrue(fitted < 120f)
        assertTrue(result.width <= 180)
        assertTrue(result.height <= 100)
    }

    @Test
    fun clockScaleLevelsRemainDistinctAndFitRealTypefaceTimes() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val fonts = listOf(
            Triple(AodDisplaySettings.CLOCK_FONT_RUBIK, AodDisplaySettings.CLOCK_WEIGHT_NORMAL,
                context.resources.getFont(R.font.rubik_regular)),
            Triple(AodDisplaySettings.CLOCK_FONT_RUBIK, AodDisplaySettings.CLOCK_WEIGHT_BOLD,
                context.resources.getFont(R.font.rubik_black)),
            Triple(AodDisplaySettings.CLOCK_FONT_SANS_SERIF, AodDisplaySettings.CLOCK_WEIGHT_NORMAL,
                Typeface.create("sans-serif", Typeface.NORMAL)),
            Triple(AodDisplaySettings.CLOCK_FONT_SANS_SERIF, AodDisplaySettings.CLOCK_WEIGHT_BOLD,
                Typeface.create("sans-serif", Typeface.BOLD)),
            Triple(AodDisplaySettings.CLOCK_FONT_SERIF, AodDisplaySettings.CLOCK_WEIGHT_NORMAL,
                Typeface.create("serif", Typeface.NORMAL)),
            Triple(AodDisplaySettings.CLOCK_FONT_SERIF, AodDisplaySettings.CLOCK_WEIGHT_BOLD,
                Typeface.create("serif", Typeface.BOLD)),
            Triple(AodDisplaySettings.CLOCK_FONT_MONOSPACE, AodDisplaySettings.CLOCK_WEIGHT_NORMAL,
                Typeface.create("monospace", Typeface.NORMAL)),
            Triple(AodDisplaySettings.CLOCK_FONT_MONOSPACE, AodDisplaySettings.CLOCK_WEIGHT_BOLD,
                Typeface.create("monospace", Typeface.BOLD))
        )
        val availableWidth = 420
        val availableHeight = 115
        val baseSize = 150f

        fonts.forEach { (font, weight, typeface) ->
            val style = clockTextStyle(font, weight, baseSize.sp)
            assertEquals(when (font) {
                AodDisplaySettings.CLOCK_FONT_SERIF -> FontFamily.Serif
                AodDisplaySettings.CLOCK_FONT_MONOSPACE -> FontFamily.Monospace
                AodDisplaySettings.CLOCK_FONT_SANS_SERIF -> FontFamily.SansSerif
                else -> style.fontFamily
            }, style.fontFamily)
            assertEquals(if (weight == AodDisplaySettings.CLOCK_WEIGHT_BOLD &&
                font == AodDisplaySettings.CLOCK_FONT_RUBIK) FontWeight.Black else if (
                weight == AodDisplaySettings.CLOCK_WEIGHT_BOLD) FontWeight.Bold else FontWeight.Normal,
                style.fontWeight)
            val times = (0..23).flatMap { hour ->
                (0..59).map { minute -> "%02d:%02d".format(java.util.Locale.US, hour, minute) }
            }
            val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { this.typeface = typeface }
            for (fontScale in listOf(1f, 1.3f, 2f)) {
                val fitCeiling = baseSize * 1.2f
                paint.textSize = fitCeiling * fontScale
                val maximumWidth = times.maxOf { ceil(paint.measureText(it)).toInt() }
                val metrics = paint.fontMetrics
                val maximumHeight = ceil(metrics.descent - metrics.ascent).toInt()
                val fittedBase = AodDisplaySettings.fitClockSizeSp(fitCeiling,
                    availableWidth, availableHeight) { size ->
                    AodDisplaySettings.scaleClockMeasurement(size, fitCeiling,
                        IntSize(maximumWidth, maximumHeight))
                }
                var previous = 0f
                for (scale in 80..120 step 5) {
                    val size = AodDisplaySettings.scaledClockSizeSp(fittedBase, scale)
                    assertTrue("font=$font weight=$weight fontScale=$fontScale scale=$scale", size > previous)
                    previous = size
                    paint.textSize = size * fontScale
                    assertTrue("width font=$font weight=$weight fontScale=$fontScale scale=$scale",
                        times.all { paint.measureText(it) <= availableWidth })
                    assertTrue("height font=$font weight=$weight fontScale=$fontScale scale=$scale",
                        paint.fontMetrics.descent - paint.fontMetrics.ascent <= availableHeight)
                }
            }
        }
        assertEquals(400, sfntWeight(context.resources.openRawResource(R.font.rubik_regular).use { it.readBytes() }))
        assertEquals(850, sfntWeight(context.resources.openRawResource(R.font.rubik_black).use { it.readBytes() }))
    }

    @Test
    fun clockFitUsesFullFittingCeilingAndCapsEveryScale() {
        val ceiling = 180f
        val available = IntSize(420, 115)
        val measuredAtCeiling = IntSize(available.width, available.height)
        val fitted = AodDisplaySettings.fitClockSizeSp(ceiling, available.width, available.height) { size ->
            AodDisplaySettings.scaleClockMeasurement(size, ceiling, measuredAtCeiling)
        }
        assertEquals(ceiling, fitted, 0f)

        val wideAtCeiling = IntSize(600, 160)
        val constrained = AodDisplaySettings.fitClockSizeSp(ceiling, available.width, available.height) { size ->
            AodDisplaySettings.scaleClockMeasurement(size, ceiling, wideAtCeiling)
        }
        var previous = 0f
        for (scale in 80..120 step 5) {
            val size = AodDisplaySettings.scaledClockSizeSp(constrained, scale)
            val measured = AodDisplaySettings.scaleClockMeasurement(size, ceiling, wideAtCeiling)
            assertTrue(size > previous)
            assertTrue(measured.width <= available.width)
            assertTrue(measured.height <= available.height)
            previous = size
        }
    }

    private fun sfntWeight(bytes: ByteArray): Int {
        val font = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN)
        val tableCount = font.getShort(4).toInt() and 0xffff
        for (index in 0 until tableCount) {
            val record = 12 + index * 16
            if (bytes.copyOfRange(record, record + 4).decodeToString() == "OS/2") {
                val offset = font.getInt(record + 8)
                return font.getShort(offset + 4).toInt() and 0xffff
            }
        }
        error("Rubik font has no OS/2 table")
    }

    @Test
    fun savedWaveformWeightsMapToThinDefaultAndBoldStrokeScales() {
        assertEquals(0.5f, AodDisplaySettings(waveformWeight = 80).waveformStrokeScale)
        assertEquals(1f, AodDisplaySettings().waveformStrokeScale)
        assertEquals(2f, AodDisplaySettings(waveformWeight = 120).waveformStrokeScale)
    }
}
