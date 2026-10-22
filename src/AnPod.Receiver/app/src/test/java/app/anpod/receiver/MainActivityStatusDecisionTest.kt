package app.anpod.receiver

import org.junit.Assert.assertEquals
import org.junit.Test

class MainActivityStatusDecisionTest {
    @Test
    fun mainStatusDistinguishesConnectedPausedAndPreparedStates() {
        fun label(state: String, standby: Boolean = false, attention: Boolean = false) = chooseMainStateLabel(
            state, "idle", snapshotAvailable = true, backgroundStandbyEnabled = standby,
            connectionPending = false, needsAttention = attention
        )

        assertEquals(R.string.aod_connected, label("authenticated"))
        assertEquals(R.string.aod_connected, label("receiverRunning"))
        assertEquals(R.string.state_receiving, label("streaming"))
        assertEquals(R.string.paused, label("paused"))
        assertEquals(R.string.state_waiting_for_computer, label("prepared"))
        assertEquals(R.string.state_needs_attention, label("streaming", attention = true))
    }

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
