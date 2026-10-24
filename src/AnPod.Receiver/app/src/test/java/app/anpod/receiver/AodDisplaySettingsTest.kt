package app.anpod.receiver

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

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
    fun clockScaleChangesSizeUntilAvailableWidthIsReached() {
        assertEquals(80f, AodDisplaySettings.clockSizeSp(100f, 240f, 120f, 400f, 300f, 80), 0.01f)
        assertEquals(100f, AodDisplaySettings.clockSizeSp(100f, 240f, 120f, 400f, 300f, 100), 0.01f)
        assertEquals(120f, AodDisplaySettings.clockSizeSp(100f, 240f, 120f, 400f, 300f, 120), 0.01f)
        assertEquals(100f, AodDisplaySettings.clockSizeSp(100f, 240f, 120f, 240f, 300f, 120), 0.01f)
        assertEquals(100f, AodDisplaySettings.clockSizeSp(100f, 240f, 200f, 400f, 200f, 120), 0.01f)
    }
}
