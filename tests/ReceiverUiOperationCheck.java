package app.anpod.receiver;

/** Run with javac/java on ReceiverUiOperationCheck and the production helper. */
public final class ReceiverUiOperationCheck {
    public static void main(String[] args) {
        ReceiverUiOperation operation = new ReceiverUiOperation();
        check(operation.begin("external-bootstrap"), "external startup is accepted");
        check(operation.running("external-bootstrap"), "external owner becomes stoppable");
        check(operation.stop("external-bootstrap"), "snapshot owner can stop external startup");
        check(operation.completeStop("external-bootstrap"), "stop completes");
        check(operation.startedServiceStopId(3) == 3, "stopped owner ends all delivered start requests");

        check(operation.begin("owner-with-ignored-start"), "new owner starts after cleanup");
        check(operation.running("owner-with-ignored-start"), "new owner is active");
        check(operation.startedServiceStopId(8) == 0, "ignored management start cannot stop an active owner");
        check(operation.bootstrapStopDisposition() == ReceiverUiOperation.BOOTSTRAP_STOP_OWNER,
                "Bootstrap STOP is routed to the current active owner");
        check(operation.stop("owner-with-ignored-start"), "current owner begins cleanup");
        check(operation.bootstrapStopDisposition() == ReceiverUiOperation.BOOTSTRAP_STOP_PENDING,
                "a duplicate Bootstrap STOP cannot replace in-flight owner cleanup");
        check(operation.completeStop("owner-with-ignored-start"), "current owner cleanup completes");
        check(operation.bootstrapStopDisposition() == ReceiverUiOperation.BOOTSTRAP_STOP_IDLE,
                "idle Bootstrap STOP only releases the started service state");
        check(operation.startedServiceStopId(8) == 8, "cleanup stops through the latest delivered startId");

        check(operation.begin("cancelled-start"), "pending UI start is accepted");
        check(operation.stop("cancelled-start"), "pending UI start can be cancelled");
        check(operation.completeStop("cancelled-start"), "cancelled UI start reaches terminal state");
        check(operation.shouldStopRejectedStart(), "late cancelled start is safe to reject and stop");
        check(operation.startedServiceStopId(9) == 9, "late rejected start is included in stop decision");

        check(operation.begin("failed-ui-start"), "restart after stop is accepted");
        check(operation.stop("failed-ui-start"), "failed startup enters the shared cleanup path");
        check(operation.completeFailure("failed-ui-start", "interface unavailable"), "failed start becomes terminal");
        check(operation.begin("next-ui-start"), "restart after failure is accepted");
        long priorGeneration = 12;
        long currentGeneration = 13;
        check(!operation.shouldStopRejectedStart(), "late prior start cannot stop a newer owner");
        check(operation.startedServiceStopId(10) == 0, "new active owner suppresses stopSelf decision");
        check(!operation.owns("failed-ui-start", priorGeneration, currentGeneration),
                "a stale notification owner/generation cannot target the newer owner");
        check(!operation.owns("next-ui-start", priorGeneration, currentGeneration),
                "a stale generation cannot stop the current owner");
        check(operation.owns("next-ui-start", currentGeneration, currentGeneration),
                "the current notification owner/generation is accepted");
        check(!operation.running("failed-ui-start"), "late old-owner callback is rejected");
        check(operation.running("next-ui-start"), "current owner callback is accepted");
        check(ReceiverUiOperation.RUNNING.equals(operation.phase()), "stale stop leaves the current owner running");
        check(operation.stop("next-ui-start"), "current owner remains stoppable");

        check("prepared".equals(ReceiverUiOperation.displayState("prepared", false, false)),
                "an open listener is not reported as connected");
        check("receiverRunning".equals(ReceiverUiOperation.displayState("prepared", true, true)),
                "audio start reflects the service session");
        check("streaming".equals(ReceiverUiOperation.displayState("streaming", true, true)),
                "streaming reflects the coordinator status");
        System.out.println("ReceiverUiOperationCheck: PASS");
    }

    private static void check(boolean condition, String description) {
        if (!condition) throw new AssertionError(description);
    }
}
