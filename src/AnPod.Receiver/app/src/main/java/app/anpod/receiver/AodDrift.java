package app.anpod.receiver;

/**
 * Burn-in protection for the AOD: one shared drift for every visible element plus the idle fade of
 * the exit label. Kept free of Android UI types so the motion and the fade threshold stay testable.
 */
final class AodDrift {
    /**
     * Horizontal and vertical travel of the clock, waveform and metrics row. The 10% top inset, the
     * 0.90 waveform width and the 28dp footer side padding all leave more room than this.
     */
    static final float MAX_X_DP = 16f;
    static final float MAX_Y_DP = 9f;
    /** The exit label fades out after this much untouched time; any touch brings it back. */
    static final long EXIT_IDLE_MILLIS = 6_000L;

    private AodDrift() {}

    static float xDp(double elapsedSeconds) {
        return (float) (Math.sin(elapsedSeconds / 120.0 * 2.0 * Math.PI) * MAX_X_DP);
    }

    static float yDp(double elapsedSeconds) {
        return (float) (Math.cos(elapsedSeconds / 150.0 * 2.0 * Math.PI) * MAX_Y_DP);
    }

    static boolean exitVisible(long idleMillis) {
        return idleMillis < EXIT_IDLE_MILLIS;
    }
}
