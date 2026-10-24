package app.anpod.receiver;

/** JNI surface shared with the native SPSC/Oboe implementation. */
final class OboeNativeBridge {
    static final int INFO_LENGTH = 15;
    static final int METRICS_LENGTH = 11;
    static final int TIMESTAMP_LENGTH = 2;

    static final int ERR_INVALID_ARGUMENT = -1;
    static final int ERR_INVALID_HANDLE = -2;
    static final int ERR_INVALID_STATE = -3;
    static final int ERR_UNSUPPORTED_FORMAT = -4;
    static final int ERR_OPEN_FAILED = -5;
    static final int ERR_BUDGET = -6;
    static final int ERR_STREAM_FAILED = -7;
    static final int ERR_JNI_ARRAY = -8;

    private static volatile boolean loaded;
    private static volatile NativeCalls testCalls;

    interface NativeCalls {
        long open(int sampleRate, int channels, int totalBudgetFrames, int requestedBridgeCapacityFrames, int[] info);
        int writeFloat(long handle, float[] interleaved, int offsetFrames, int frameCount);
        int requestPause(long handle);
        int pauseAndFlush(long handle);
        int start(long handle, int[] info);
        int setVolume(long handle, float volume);
        int getMetrics(long handle, int[] metrics, long[] timestamp);
        int close(long handle);
        String lastErrorMessage();
    }

    private OboeNativeBridge() { }

    static synchronized void load() {
        if (testCalls != null) return;
        if (loaded) return;
        System.loadLibrary("anpod_oboe_output");
        loaded = true;
    }

    static void setTestCalls(NativeCalls calls) { testCalls = calls; }

    static long open(int rate, int channels, int budget, int bridgeCapacity, int[] info) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeOpen(rate, channels, budget, bridgeCapacity, info)
                : calls.open(rate, channels, budget, bridgeCapacity, info);
    }
    static int writeFloat(long handle, float[] samples, int offset, int frames) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeWriteFloat(handle, samples, offset, frames)
                : calls.writeFloat(handle, samples, offset, frames);
    }
    static int requestPause(long handle) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeRequestPause(handle) : calls.requestPause(handle);
    }
    static int pauseAndFlush(long handle) {
        NativeCalls calls = testCalls;
        return calls == null ? nativePauseAndFlush(handle) : calls.pauseAndFlush(handle);
    }
    static int start(long handle, int[] info) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeStart(handle, info) : calls.start(handle, info);
    }
    static int setVolume(long handle, float volume) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeSetVolume(handle, volume) : calls.setVolume(handle, volume);
    }
    static int getMetrics(long handle, int[] metrics, long[] timestamp) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeGetMetrics(handle, metrics, timestamp)
                : calls.getMetrics(handle, metrics, timestamp);
    }
    static int close(long handle) {
        NativeCalls calls = testCalls;
        return calls == null ? nativeClose(handle) : calls.close(handle);
    }
    static String lastErrorMessage() {
        NativeCalls calls = testCalls;
        return calls == null ? nativeGetLastErrorMessage() : calls.lastErrorMessage();
    }

    static native long nativeOpen(int sampleRate, int channels, int totalBudgetFrames,
            int requestedBridgeCapacityFrames, int[] info);
    static native int nativeWriteFloat(long handle, float[] interleaved, int offsetFrames, int frameCount);
    static native int nativeRequestPause(long handle);
    static native int nativePauseAndFlush(long handle);
    static native int nativeStart(long handle, int[] info);
    static native int nativeSetVolume(long handle, float volume);
    static native int nativeGetMetrics(long handle, int[] metrics, long[] timestamp);
    static native int nativeClose(long handle);
    static native String nativeGetLastErrorMessage();
}
