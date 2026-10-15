package app.anpod.receiver;

/** Small production owner and phase gate shared by the Binder UI and service callbacks. */
final class ReceiverUiOperation {
    static final String IDLE = "idle";
    static final String STARTING = "starting";
    static final String RUNNING = "running";
    static final String STOPPING = "stopping";
    static final String STOPPED = "stopped";
    static final String FAILED = "error";
    static final int BOOTSTRAP_STOP_OWNER = 1;
    static final int BOOTSTRAP_STOP_PENDING = 2;
    static final int BOOTSTRAP_STOP_IDLE = 3;

    private String ownerId = "";
    private String phase = IDLE;
    private String error = "";

    String ownerId() { return ownerId; }
    String phase() { return phase; }
    String error() { return error; }

    int bootstrapStopDisposition() {
        if (STARTING.equals(phase) || RUNNING.equals(phase)) return BOOTSTRAP_STOP_OWNER;
        if (STOPPING.equals(phase)) return BOOTSTRAP_STOP_PENDING;
        return BOOTSTRAP_STOP_IDLE;
    }

    boolean canBegin() {
        return IDLE.equals(phase) || STOPPED.equals(phase) || FAILED.equals(phase);
    }

    boolean shouldStopRejectedStart() { return canBegin(); }

    int startedServiceStopId(int latestDeliveredStartId) {
        return canBegin() && latestDeliveredStartId > 0 ? latestDeliveredStartId : 0;
    }

    boolean begin(String owner) {
        if (!canBegin() || owner == null || owner.isEmpty()) return false;
        ownerId = owner;
        phase = STARTING;
        error = "";
        return true;
    }

    boolean owns(String owner) { return owner != null && !owner.isEmpty() && owner.equals(ownerId); }

    boolean owns(String owner, long requestedGeneration, long currentGeneration) {
        return owns(owner) && requestedGeneration == currentGeneration;
    }

    boolean running(String owner) {
        if (!owns(owner) || !STARTING.equals(phase)) return false;
        phase = RUNNING;
        return true;
    }

    boolean reportError(String owner, String reason) {
        if (!owns(owner) || !RUNNING.equals(phase)) return false;
        error = reason == null ? "" : reason;
        return true;
    }

    boolean stop(String owner) {
        if (!owns(owner) || !(STARTING.equals(phase) || RUNNING.equals(phase))) return false;
        phase = STOPPING;
        error = "";
        return true;
    }

    boolean completeStop(String owner) {
        if (!owns(owner) || !STOPPING.equals(phase)) return false;
        phase = STOPPED;
        error = "";
        return true;
    }

    boolean completeFailure(String owner, String reason) {
        if (!owns(owner) || !STOPPING.equals(phase)) return false;
        phase = FAILED;
        error = reason == null ? "" : reason;
        return true;
    }

    static String displayState(String networkState, boolean hasAudioSession, boolean audioActive) {
        if (hasAudioSession) {
            if ("paused".equals(networkState)) return "paused";
            if ("failed".equals(networkState)) return "failed";
            if (!audioActive) return "starting";
            return "streaming".equals(networkState) ? "streaming" : "receiverRunning";
        }
        return networkState == null ? "stopped" : networkState;
    }
}
