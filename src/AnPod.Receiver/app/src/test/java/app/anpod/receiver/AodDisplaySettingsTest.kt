package app.anpod.receiver

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import android.graphics.Paint
import android.graphics.Typeface
import androidx.core.content.res.ResourcesCompat
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.sp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.math.roundToInt
import kotlin.math.ceil
import java.nio.ByteBuffer
import java.nio.ByteOrder
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

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
            clockWeightLevel = 2)
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
        assertEquals(AodDisplaySettings.DEFAULT_CLOCK_WEIGHT_LEVEL, settings.clockWeightLevel)
        assertEquals(AodDisplaySettings.CLOCK_WEIGHT_BLACK, settings.clockWeight)
        assertEquals(100, settings.clockScale)
    }

    @Test
    fun legacyClockWeightsMigrateByFamilyAndNewLevelsStayStable() {
        val preferences = ApplicationProvider.getApplicationContext<Context>()
            .getSharedPreferences("aod_weight_migration_test", Context.MODE_PRIVATE)

        preferences.edit().clear()
            .putInt("aod_clock_weight", AodDisplaySettings.CLOCK_WEIGHT_NORMAL)
            .putInt("aod_clock_font", AodDisplaySettings.CLOCK_FONT_RUBIK)
            .commit()
        val regular = AodDisplaySettings.read(preferences)
        assertEquals(2, regular.clockWeightLevel)
        assertEquals(400, regular.clockWeight)

        preferences.edit().clear()
            .putInt("aod_clock_weight", AodDisplaySettings.CLOCK_WEIGHT_BOLD)
            .putInt("aod_clock_font", AodDisplaySettings.CLOCK_FONT_RUBIK)
            .commit()
        val legacyRubikBold = AodDisplaySettings.read(preferences)
        assertEquals(9, legacyRubikBold.clockWeightLevel)
        assertEquals(850, legacyRubikBold.clockWeight)

        preferences.edit().clear()
            .putInt("aod_clock_weight", AodDisplaySettings.CLOCK_WEIGHT_BOLD)
            .putInt("aod_clock_font", AodDisplaySettings.CLOCK_FONT_SERIF)
            .commit()
        val legacySystemBold = AodDisplaySettings.read(preferences)
        assertEquals(7, legacySystemBold.clockWeightLevel)
        assertEquals(700, legacySystemBold.clockWeight)

        legacySystemBold.copy(clockFont = AodDisplaySettings.CLOCK_FONT_RUBIK).save(preferences)
        val saved700 = AodDisplaySettings.read(preferences)
        assertEquals(7, saved700.clockWeightLevel)
        assertEquals(700, saved700.clockWeight)
        assertEquals(7, preferences.getInt("aod_clock_weight_level", -1))
        assertEquals(700, preferences.getInt("aod_clock_weight", -1))
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
    @Config(sdk = [23])
    fun allBundledClockFontsLoadAtNineWeightsAndFitClockBounds() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val families = listOf(
            AodDisplaySettings.CLOCK_FONT_RUBIK,
            AodDisplaySettings.CLOCK_FONT_SANS_SERIF,
            AodDisplaySettings.CLOCK_FONT_SERIF,
            AodDisplaySettings.CLOCK_FONT_MONOSPACE
        )
        val availableWidth = 420
        val availableHeight = 115
        val baseSize = 150f
        val expectedCmap = (0x30..0x39).toSet() + 0x3a
        val times = (0..23).flatMap { hour ->
            (0..59).map { minute -> "%02d:%02d".format(java.util.Locale.US, hour, minute) }
        }

        families.forEach { font ->
            AodDisplaySettings.CLOCK_WEIGHT_VALUES.forEach { weight ->
                val resource = clockFontResource(font, weight)
                val bytes = context.resources.openRawResource(resource).use { it.readBytes() }
                assertEquals("font=$font weight=$weight", weight, sfntWeight(bytes))
                assertFalse("font=$font weight=$weight retains fvar", "fvar" in sfntTables(bytes))
                val cmap = sfntCmapCodepoints(bytes)
                if (font == AodDisplaySettings.CLOCK_FONT_RUBIK && weight in listOf(400, 850)) {
                    assertTrue("font=$font weight=$weight", cmap.containsAll(expectedCmap))
                } else {
                    assertEquals("font=$font weight=$weight", expectedCmap, cmap)
                }
                assertEquals(FontWeight(weight), clockTextStyle(font, weight, baseSize.sp).fontWeight)
                val typeface = ResourcesCompat.getFont(context, resource)
                    ?: error("Unable to load clock font $font/$weight at API 23")
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
        }
        assertEquals(400, sfntWeight(context.resources.openRawResource(R.font.rubik_regular).use { it.readBytes() }))
        assertEquals(850, sfntWeight(context.resources.openRawResource(R.font.rubik_black).use { it.readBytes() }))
    }

    @Test
    fun waveformWeightHasNineIncreasingStepsAndPreservesLegacyAnchors() {
        var previous = 0f
        for (weight in 80..120 step 5) {
            val scale = AodDisplaySettings(waveformWeight = weight).waveformStrokeScale
            assertTrue("waveformWeight=$weight", scale > previous)
            previous = scale
        }
        assertEquals(0.5f, AodDisplaySettings(waveformWeight = 80).waveformStrokeScale)
        assertEquals(1f, AodDisplaySettings(waveformWeight = 100).waveformStrokeScale)
        assertEquals(2f, AodDisplaySettings(waveformWeight = 120).waveformStrokeScale)
    }

    @Test
    fun previewKeepsUsableControlsSpaceAsWindowHeightShrinks() {
        assertEquals(140.dp, aodSettingsPreviewHeight(480.dp))
        assertEquals(90.dp, aodSettingsPreviewHeight(300.dp))
        assertEquals(30f, aodSettingsPreviewHeight(100.dp).value, 0.001f)
        assertEquals(0.dp, aodSettingsPreviewHeight(0.dp))
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
        val offset = sfntTables(bytes)["OS/2"] ?: error("Font has no OS/2 table")
        return ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN).getShort(offset + 4).toInt() and 0xffff
    }

    private fun sfntTables(bytes: ByteArray): Map<String, Int> {
        val font = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN)
        val tableCount = font.getShort(4).toInt() and 0xffff
        return (0 until tableCount).associate { index ->
            val record = 12 + index * 16
            bytes.copyOfRange(record, record + 4).decodeToString() to font.getInt(record + 8)
        }
    }

    private fun sfntCmapCodepoints(bytes: ByteArray): Set<Int> {
        val font = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN)
        val cmapOffset = sfntTables(bytes)["cmap"] ?: error("Font has no cmap table")
        val subtableCount = font.getShort(cmapOffset + 2).toInt() and 0xffff
        val subtableOffsets = (0 until subtableCount).mapNotNull { index ->
            val record = cmapOffset + 4 + index * 8
            val offset = cmapOffset + font.getInt(record + 4)
            if ((font.getShort(offset).toInt() and 0xffff) == 4) offset else null
        }
        val subtable = subtableOffsets.firstOrNull() ?: error("Font has no cmap format 4 table")
        val segmentCount = (font.getShort(subtable + 6).toInt() and 0xffff) / 2
        val endCodes = subtable + 14
        val startCodes = endCodes + segmentCount * 2 + 2
        val deltas = startCodes + segmentCount * 2
        val rangeOffsets = deltas + segmentCount * 2
        val codepoints = mutableSetOf<Int>()
        for (segment in 0 until segmentCount) {
            val start = font.getShort(startCodes + segment * 2).toInt() and 0xffff
            val end = font.getShort(endCodes + segment * 2).toInt() and 0xffff
            val delta = font.getShort(deltas + segment * 2).toInt()
            val rangeAddress = rangeOffsets + segment * 2
            val rangeOffset = font.getShort(rangeAddress).toInt() and 0xffff
            for (codepoint in start..end) {
                val glyph = if (rangeOffset == 0) {
                    (codepoint + delta) and 0xffff
                } else {
                    val glyphAddress = rangeAddress + rangeOffset + 2 * (codepoint - start)
                    val glyphIndex = font.getShort(glyphAddress).toInt() and 0xffff
                    if (glyphIndex == 0) 0 else (glyphIndex + delta) and 0xffff
                }
                if (glyph != 0) codepoints.add(codepoint)
            }
        }
        return codepoints
    }

}
