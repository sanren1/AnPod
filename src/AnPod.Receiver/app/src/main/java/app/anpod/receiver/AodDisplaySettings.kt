package app.anpod.receiver

import android.content.SharedPreferences

internal data class AodDisplaySettings(
    val clockScale: Int = DEFAULT_CLOCK_SCALE,
    val waveformWeight: Int = DEFAULT_WAVEFORM_WEIGHT,
    val showProtocol: Boolean = true,
    val showReceiveRate: Boolean = true,
    val showBuffer: Boolean = true,
    val showLoss: Boolean = true,
    val showAudioFormat: Boolean = false,
    val showUnderruns: Boolean = false,
    val showOutputEngine: Boolean = false
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

        fun clockSizeSp(
            baseSizeSp: Float,
            naturalWidthAtBasePx: Float,
            naturalHeightAtBasePx: Float,
            availableWidthPx: Float,
            availableHeightPx: Float,
            scalePercent: Int
        ): Float {
            val scale = scalePercent / 100f
            val widthScale = if (naturalWidthAtBasePx <= 0f)
                1f
                else minOf(1f, availableWidthPx / (naturalWidthAtBasePx * scale))
            val heightScale = if (naturalHeightAtBasePx <= 0f)
                1f
                else minOf(1f, availableHeightPx / (naturalHeightAtBasePx * scale))
            return baseSizeSp * scale * minOf(widthScale, heightScale)
        }

        fun read(preferences: SharedPreferences) = AodDisplaySettings(
            clockScale = preferences.getInt(KEY_CLOCK_SCALE, DEFAULT_CLOCK_SCALE),
            waveformWeight = preferences.getInt(KEY_WAVEFORM_WEIGHT, DEFAULT_WAVEFORM_WEIGHT),
            showProtocol = preferences.getBoolean(KEY_PROTOCOL, true),
            showReceiveRate = preferences.getBoolean(KEY_RECEIVE_RATE, true),
            showBuffer = preferences.getBoolean(KEY_BUFFER, true),
            showLoss = preferences.getBoolean(KEY_LOSS, true),
            showAudioFormat = preferences.getBoolean(KEY_AUDIO_FORMAT, false),
            showUnderruns = preferences.getBoolean(KEY_UNDERRUNS, false),
            showOutputEngine = preferences.getBoolean(KEY_OUTPUT_ENGINE, false)
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
            .apply()
    }
}
