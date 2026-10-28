package app.anpod.receiver

import android.content.SharedPreferences
import androidx.compose.ui.unit.IntSize

internal data class AodDisplaySettings(
    val clockScale: Int = DEFAULT_CLOCK_SCALE,
    val waveformWeight: Int = DEFAULT_WAVEFORM_WEIGHT,
    val showProtocol: Boolean = true,
    val showReceiveRate: Boolean = true,
    val showBuffer: Boolean = true,
    val showLoss: Boolean = true,
    val showAudioFormat: Boolean = false,
    val showUnderruns: Boolean = false,
    val showOutputEngine: Boolean = false,
    val clockFont: Int = CLOCK_FONT_RUBIK,
    val clockWeight: Int = CLOCK_WEIGHT_BOLD
) {
    val waveformStrokeScale: Float
        get() = when (waveformWeight) {
            80 -> 0.5f
            120 -> 2f
            else -> 1f
        }

    companion object {
        const val DEFAULT_CLOCK_SCALE = 100
        const val DEFAULT_WAVEFORM_WEIGHT = 100
        private const val KEY_CLOCK_SCALE = "aod_clock_scale"
        private const val KEY_WAVEFORM_WEIGHT = "aod_waveform_weight"
        private const val KEY_PROTOCOL = "aod_show_protocol"
        private const val KEY_RECEIVE_RATE = "aod_show_receive_rate"
        private const val KEY_BUFFER = "aod_show_buffer"
        private const val KEY_LOSS = "aod_show_loss"
        private const val KEY_AUDIO_FORMAT = "aod_show_audio_format"
        private const val KEY_UNDERRUNS = "aod_show_underruns"
        private const val KEY_OUTPUT_ENGINE = "aod_show_output_engine"
        private const val KEY_CLOCK_FONT = "aod_clock_font"
        private const val KEY_CLOCK_WEIGHT = "aod_clock_weight"

        const val CLOCK_FONT_RUBIK = 0
        const val CLOCK_FONT_SANS_SERIF = 1
        const val CLOCK_FONT_SERIF = 2
        const val CLOCK_FONT_MONOSPACE = 3
        const val CLOCK_WEIGHT_NORMAL = 400
        const val CLOCK_WEIGHT_BOLD = 700

        fun fitClockSizeSp(
            requestedSizeSp: Float,
            availableWidthPx: Int,
            availableHeightPx: Int,
            measure: (Float) -> IntSize
        ): Float {
            if (measure(requestedSizeSp).let {
                    it.width <= availableWidthPx && it.height <= availableHeightPx
                }) return requestedSizeSp

            var lower = 0f
            var upper = requestedSizeSp
            repeat(10) {
                val candidate = (lower + upper) / 2f
                val size = measure(candidate)
                if (size.width <= availableWidthPx && size.height <= availableHeightPx) {
                    lower = candidate
                } else {
                    upper = candidate
                }
            }
            return lower
        }

        fun scaleClockMeasurement(sizeSp: Float, measuredAtSp: Float, measuredSize: IntSize): IntSize {
            val ratio = sizeSp / measuredAtSp
            return IntSize(
                kotlin.math.ceil(measuredSize.width * ratio).toInt(),
                kotlin.math.ceil(measuredSize.height * ratio).toInt()
            )
        }

        fun scaledClockSizeSp(maximumSizeSp: Float, clockScale: Int): Float =
            maximumSizeSp * clockScale.coerceIn(80, 120) / 120f

        fun read(preferences: SharedPreferences) = AodDisplaySettings(
            clockScale = preferences.getInt(KEY_CLOCK_SCALE, DEFAULT_CLOCK_SCALE),
            waveformWeight = preferences.getInt(KEY_WAVEFORM_WEIGHT, DEFAULT_WAVEFORM_WEIGHT),
            showProtocol = preferences.getBoolean(KEY_PROTOCOL, true),
            showReceiveRate = preferences.getBoolean(KEY_RECEIVE_RATE, true),
            showBuffer = preferences.getBoolean(KEY_BUFFER, true),
            showLoss = preferences.getBoolean(KEY_LOSS, true),
            showAudioFormat = preferences.getBoolean(KEY_AUDIO_FORMAT, false),
            showUnderruns = preferences.getBoolean(KEY_UNDERRUNS, false),
            showOutputEngine = preferences.getBoolean(KEY_OUTPUT_ENGINE, false),
            clockFont = preferences.getInt(KEY_CLOCK_FONT, CLOCK_FONT_RUBIK),
            clockWeight = preferences.getInt(KEY_CLOCK_WEIGHT, CLOCK_WEIGHT_BOLD)
        )
    }

    fun save(preferences: SharedPreferences) {
        preferences.edit()
            .putInt(KEY_CLOCK_SCALE, clockScale)
            .putInt(KEY_WAVEFORM_WEIGHT, waveformWeight)
            .putBoolean(KEY_PROTOCOL, showProtocol)
            .putBoolean(KEY_RECEIVE_RATE, showReceiveRate)
            .putBoolean(KEY_BUFFER, showBuffer)
            .putBoolean(KEY_LOSS, showLoss)
            .putBoolean(KEY_AUDIO_FORMAT, showAudioFormat)
            .putBoolean(KEY_UNDERRUNS, showUnderruns)
            .putBoolean(KEY_OUTPUT_ENGINE, showOutputEngine)
            .putInt(KEY_CLOCK_FONT, clockFont)
            .putInt(KEY_CLOCK_WEIGHT, clockWeight)
            .apply()
    }
}
