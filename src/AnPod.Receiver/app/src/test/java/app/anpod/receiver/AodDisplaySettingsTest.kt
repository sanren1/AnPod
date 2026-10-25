package app.anpod.receiver

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import androidx.compose.ui.unit.IntSize
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import kotlin.math.roundToInt

@RunWith(RobolectricTestRunner::class)
class AodDisplaySettingsTest {
    @Test
    fun defaultsAndPreferencesRoundTrip() {
        val preferences = ApplicationProvider.getApplicationContext<Context>()
            .getSharedPreferences("aod_test", Context.MODE_PRIVATE)
        preferences.edit().clear().commit()

        assertEquals(AodDisplaySettings(), AodDisplaySettings.read(preferences))

        val selected = AodDisplaySettings(115, 120, false, true, false, true)
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
    fun savedWaveformWeightsMapToThinDefaultAndBoldStrokeScales() {
        assertEquals(0.5f, AodDisplaySettings(waveformWeight = 80).waveformStrokeScale)
        assertEquals(1f, AodDisplaySettings().waveformStrokeScale)
        assertEquals(2f, AodDisplaySettings(waveformWeight = 120).waveformStrokeScale)
    }
}
