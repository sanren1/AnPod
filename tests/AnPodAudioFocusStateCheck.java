package app.anpod.receiver;

/** Offline production-state check: javac AudioFocusState.java tests/AnPodAudioFocusStateCheck.java. */
public final class AnPodAudioFocusStateCheck {
    public static void main(String[] args) {
        AudioFocusState state = new AudioFocusState();
        check(state.canPlay(), "new session should be writable");
        long inFlightWrite = state.writeGeneration();
        check(state.onFocusChange(AudioFocusState.Change.LOSS_TRANSIENT_CAN_DUCK) == AudioFocusState.Action.DUCK,
                "duck transition");
        check(state.acceptsWrite(inFlightWrite), "duck must not invalidate queued writes");
        check(state.onFocusChange(AudioFocusState.Change.GAIN) == AudioFocusState.Action.RESTORE_VOLUME,
                "duck gain restores volume only");

        state.markTrackPlaying();
        inFlightWrite = state.writeGeneration();
        check(state.onFocusChange(AudioFocusState.Change.LOSS_TRANSIENT) == AudioFocusState.Action.PAUSE,
                "transient focus loss pauses");
        check(!state.canPlay() && !state.acceptsWrite(inFlightWrite), "pause invalidates in-flight PCM");
        long focusPauseGeneration = state.writeGeneration();
        check(state.onFocusChange(AudioFocusState.Change.LOSS_TRANSIENT) == AudioFocusState.Action.NONE
                        && state.writeGeneration() == focusPauseGeneration,
                "duplicate focus loss must not repeat a pause generation");
        check(state.onFocusChange(AudioFocusState.Change.GAIN) == AudioFocusState.Action.RESUME,
                "gain resumes an intended session");
        check(state.canPlay(), "gain restores write permission");

        state.markTrackPlaying();
        check(state.setUserPaused(true) == AudioFocusState.Action.PAUSE, "first user pause pauses the track");
        long userPauseGeneration = state.writeGeneration();
        check(state.setUserPaused(true) == AudioFocusState.Action.NONE
                        && state.writeGeneration() == userPauseGeneration,
                "duplicate user pause must not flush or invalidate another write generation");
        check(state.onFocusChange(AudioFocusState.Change.LOSS_TRANSIENT) == AudioFocusState.Action.PAUSE,
                "focus loss while user-paused still flushes the track");
        check(state.onFocusChange(AudioFocusState.Change.GAIN) == AudioFocusState.Action.HOLD,
                "gain must preserve a user pause");
        check(!state.canPlay(), "user pause remains in force after gain");
        check(state.setUserPaused(false) == AudioFocusState.Action.RESUME && state.canPlay(),
                "later user resume restarts a track held by focus loss");
        check(state.setUserPaused(false) == AudioFocusState.Action.NONE,
                "duplicate user resume must not issue another play");

        AudioFocusState stopped = new AudioFocusState();
        stopped.onFocusChange(AudioFocusState.Change.LOSS_TRANSIENT);
        stopped.cancel();
        check(stopped.onFocusChange(AudioFocusState.Change.GAIN) == AudioFocusState.Action.NONE,
                "late gain cannot revive a stopped session");
        check(!stopped.canPlay(), "stopped session rejects writes");

        AudioFocusState permanentlyLost = new AudioFocusState();
        check(permanentlyLost.onFocusChange(AudioFocusState.Change.LOSS) == AudioFocusState.Action.STOP,
                "permanent focus loss stops the session");
        check(permanentlyLost.onFocusChange(AudioFocusState.Change.GAIN) == AudioFocusState.Action.NONE,
                "gain after permanent loss is ignored");
        System.out.println("Android audio focus state and write-generation checks passed.");
    }

    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}
