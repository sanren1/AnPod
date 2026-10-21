package app.anpod.receiver;

/** Serial state for user pauses, audio-focus changes, and writes already in flight. */
final class AudioFocusState {
    enum Action { NONE, DUCK, PAUSE, STOP, RESUME, HOLD, RESTORE_VOLUME, RESTORE_AND_RESUME, RESTORE_AND_HOLD }
    enum Change { LOSS, LOSS_TRANSIENT, LOSS_TRANSIENT_CAN_DUCK, GAIN }

    private boolean userPaused;
    private boolean focusPaused;
    private boolean ducked;
    private boolean cancelled;
    private boolean trackNeedsResume;
    private long writeGeneration;

    synchronized Action setUserPaused(boolean paused) {
        if (cancelled || userPaused == paused) return Action.NONE;
        userPaused = paused;
        writeGeneration++;
        if (paused) return Action.PAUSE;
        return !paused && !focusPaused && trackNeedsResume ? Action.RESUME : Action.NONE;
    }

    synchronized Action onFocusChange(Change change) {
        if (cancelled) return Action.NONE;
        switch (change) {
            case LOSS:
                cancelled = true;
                writeGeneration++;
                return Action.STOP;
            case LOSS_TRANSIENT:
                if (focusPaused) return Action.NONE;
                writeGeneration++;
                focusPaused = true;
                trackNeedsResume = true;
                return Action.PAUSE;
            case LOSS_TRANSIENT_CAN_DUCK:
                ducked = true;
                return Action.DUCK;
            case GAIN:
                boolean restoreVolume = ducked;
                ducked = false;
                if (!focusPaused) return restoreVolume ? Action.RESTORE_VOLUME : Action.NONE;
                focusPaused = false;
                if (userPaused) return restoreVolume ? Action.RESTORE_AND_HOLD : Action.HOLD;
                return restoreVolume ? Action.RESTORE_AND_RESUME : Action.RESUME;
            default:
                throw new AssertionError(change);
        }
    }

    synchronized void cancel() {
        if (cancelled) return;
        cancelled = true;
        writeGeneration++;
    }

    synchronized void markTrackPlaying() { trackNeedsResume = false; }
    synchronized boolean isCancelled() { return cancelled; }
    synchronized boolean isUserPaused() { return userPaused; }
    synchronized boolean isFocusPaused() { return focusPaused; }
    synchronized boolean canPlay() { return !cancelled && !userPaused && !focusPaused; }
    synchronized long writeGeneration() { return writeGeneration; }
    synchronized boolean acceptsWrite(long generation) {
        return generation == writeGeneration && canPlay();
    }
}
