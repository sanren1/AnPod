package app.anpod.receiver

import org.junit.Assert.assertEquals
import org.junit.Test

class MainActivityStatusDecisionTest {
    @Test
    fun activeSessionGuidanceWinsWhenWirelessSetupIsUnavailable() {
        fun next(serviceState: String, playbackAdvancing: Boolean? = null) = chooseMainNextStep(
            serviceState, playbackAdvancing, "listening", true, "untilRevoked",
            bound = false, noNetwork = true, networkChoiceNeeded = true
        )

        assertEquals(R.string.next_step_audio_received, next("streaming", true))
        assertEquals(R.string.next_step_check_computer_playback, next("receiverRunning", false))
        assertEquals(R.string.next_step_play_on_computer, next("streaming"))
        assertEquals(R.string.next_step_resume_playback, next("paused"))
        assertEquals(R.string.next_step_play_on_computer, next("authenticated"))
    }
}
