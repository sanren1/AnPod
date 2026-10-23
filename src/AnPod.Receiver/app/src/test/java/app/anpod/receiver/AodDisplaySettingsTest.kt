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
        assertEquals(100, settings.clockScale)
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
