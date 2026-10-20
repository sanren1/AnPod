# Oboe output JNI contract

This section is the shared Java/native API contract for the Receiver Oboe output. Keep the class, signatures, array lengths, and index meanings synchronized. The implementation uses Oboe 1.10.0 and keeps AudioTrack as the same-format fallback.

## Java entry point

The JNI class is `app.anpod.receiver.OboeNativeBridge`; it lazily loads `System.loadLibrary("anpod_oboe_output")` only when Oboe output is selected. The CMake library basename is `anpod_oboe_output`. `OboePlaybackOutput` owns one native handle for one playback session and adapts these methods to the output lifecycle. The single Java audio worker is the only writer. Java serializes writes and control calls for a handle; `generation` remains Java-owned and is checked before and after any bounded wait. Native code does not receive or interpret a generation value.

```java
static native long nativeOpen(
    int sampleRate, int channels, int totalBudgetFrames,
    int requestedBridgeCapacityFrames, int[] info);
static native int nativeWriteFloat(
    long handle, float[] interleaved, int offsetFrames, int frameCount);
static native int nativeRequestPause(long handle);
static native int nativePauseAndFlush(long handle);
static native int nativeStart(long handle, int[] info);
static native int nativeSetVolume(long handle, float volume);
static native int nativeGetMetrics(long handle, int[] metrics, long[] timestamp);
static native int nativeClose(long handle);
static native String nativeGetLastErrorMessage();
```

`nativeOpen` returns a nonzero handle on success and `0` on failure. If opening fails but Oboe cannot confirm that the stream is closed, it returns a nonzero cleanup handle with the failure in `info[0]`; Java must close that handle before falling back. `info` must have at least 15 elements; on every non-null result path, element 0 contains the status. If writing `info` itself fails and no handle can be returned, native code retains callback-visible state for process lifetime rather than risking use-after-free. `nativeGetLastErrorMessage()` is a control-thread diagnostic for the calling thread's most recent failed JNI operation, including `nativeOpen`; it is never called from an Oboe callback.

`nativeWriteFloat` uses frame units for `offsetFrames` and `frameCount`; the opened channel count determines interleaved sample indexing. It accepts and copies any prefix in `[0, frameCount]` and returns accepted frames. `0` means the ring is currently full. Negative values are bridge error codes. Java advances the offset by the accepted count, checks its session generation, and performs any bounded wait outside JNI before retrying. The producer must not drop a partially accepted block or hold `GetPrimitiveArrayCritical` across waits or control operations. JNI validates array bounds and all frame/channel/sample multiplications before accessing the array.

`nativeRequestPause` only sets the lock-free mute gate; it does not reset ring state. `nativeOpen` allocates the ring and opens (but does not start) the stream. The initial `nativeStart` validates the open stream and starts it without resetting the prefilled ring. After the Java producer has stopped and invalidated the generation, `nativePauseAndFlush` stops/closes the stream on a control thread, waits for callback quiescence, and resets ring cursors. A later `nativeStart(handle, info)` reopens the stream, revalidates actual format and budget, updates `info`, then starts callbacks; the ring remains empty and at its original capacity. Each `nativeStart` fills the same 15-element info array as `nativeOpen`, so Java diagnostics reflect the active stream after every reopen. `nativeClose` is serialized with every other JNI call for the handle, stops and closes the stream, waits for callbacks to finish, then releases the handle and callback state. Calls with an invalid or already-closed handle return `ERR_INVALID_HANDLE`.

## Open result (`info`, 15 integers)

All indices are fixed. Rates/capacities use frames and samples per second, not bytes. Unknown hardware values are `0`; zero is never evidence of a real hardware rate or format.

| Index | Meaning | Values |
| --- | --- | --- |
| 0 | Status | `0` success; otherwise a negative `ERR_*` code below |
| 1 | Actual client stream sample rate | Positive rate; `0` on failure |
| 2 | Actual client stream channels | Positive count; `0` on failure |
| 3 | Actual client stream format | `1` = Float32; any other value fails open |
| 4 | Frames per burst | Positive when reported; `0` unknown |
| 5 | Actual stream buffer size/effective frames | `AudioStream::getBufferSizeInFrames()` after configuration |
| 6 | Actual stream buffer capacity | `AudioStream::getBufferCapacityInFrames()` |
| 7 | Actual bridge ring capacity | Frames, after budget and burst validation |
| 8 | Audio API | `0` unknown, `1` OpenSL ES, `2` AAudio |
| 9 | Sharing mode | `0` unknown, `1` Shared, `2` Exclusive |
| 10 | Performance mode | `0` unknown, `1` None, `2` PowerSaving, `3` LowLatency, `4` PowerSavingOffloaded |
| 11 | Total allowed output budget | Frames, derived from the selected 15 ms or 50 ms budget at the requested rate |
| 12 | Hardware sample rate | Positive only when genuinely available; otherwise `0` |
| 13 | Hardware channels | Positive only when genuinely available; otherwise `0` |
| 14 | Hardware format | `0` unknown, `1` PCM16, `2` packed PCM24, `3` Float32, `4` PCM32, `5` other |

The bridge must request Shared/Media Float32 with the exact requested rate and channels, disable Oboe sample-rate conversion (`SampleRateConversionQuality::None`), format conversion, and channel conversion. `None` means Oboe itself does not resample; the underlying Android API may still convert. After every open/reopen, actual client rate/channels/format are checked before start. A mismatch or a budget that cannot hold at least one burst in the ring is an open/start failure; Java closes the failed Oboe output and falls back to AudioTrack at the same negotiated rate/channels/format.

The ring and actual stream buffer share the existing output budget: `ringCapacityFrames + actualStreamBufferSizeFrames <= totalBudgetFrames`. Java requests a 5 ms bridge ring (`sampleRate * 5 / 1000` frames); native may reduce that request only as needed to fit the actual stream buffer, and must reject the Oboe open if the resulting ring cannot hold at least one burst. For Oboe only, Java sets `effectivePreRollFrames = min(existingPreRollFrames, ringCapacityFrames)`, writes no more than that amount into the stopped ring, then calls `nativeStart`. The AudioTrack pre-roll algorithm is unchanged. No additional staging queue or latency budget is introduced.

## Runtime metrics (`metrics`, 11 integers; `timestamp`, 2 longs)

`nativeGetMetrics` is called from the Java worker/control path only. Native snapshots Oboe stream counters under the per-session control mutex so it cannot race close/reopen; the real-time callback never takes that mutex. Except where stated otherwise, cumulative counters are the low 32 bits and Java extends them with unsigned deltas within the current output epoch. `nativePauseAndFlush` resets ring cursors and callback counters, so Java starts a new unsigned-counter baseline after each successful `nativeStart`; it must not interpret the reset as a 2^32-frame advance. None of these counters means acoustic presentation time.

| Index | Meaning | Semantics |
| --- | --- | --- |
| 0 | Accepted source frames | Cumulative frames copied into the ring, modulo 2^32 |
| 1 | Ring frames consumed | Real source frames read by the callback, modulo 2^32 |
| 2 | Callback frames requested | Total callback frame demand, modulo 2^32 |
| 3 | Zero-filled frames | Callback output frames filled with silence because no ring data was available, modulo 2^32 |
| 4 | Ring pending frames | Current `writeCursor - readCursor` |
| 5 | Stream API pending frames | API-level pending estimate only; `-1` when unavailable |
| 6 | Stream frames written | Low 32 bits of Oboe `getFramesWritten()` when available; otherwise not interpretable |
| 7 | Stream frames read | Low 32 bits of Oboe `getFramesRead()` when available; otherwise not interpretable |
| 8 | XRun count | Oboe XRun count; `-1` when unavailable |
| 9 | Producer full retries | Ring-full observations, modulo 2^32 |
| 10 | Last callback error | `0` when none, otherwise raw Oboe callback error code |

`timestamp[0]` is the Oboe timestamp frame position and `timestamp[1]` its timestamp in nanoseconds; both are `-1` if either value is unavailable. Indices 6 and 7 carry modulo-2^32 bit patterns when counters are available; every `int` bit pattern, including `-1`, is valid and must be interpreted as unsigned. Java treats these two counters as unavailable whenever index 5 is `-1`; it must not infer a counter error from a negative `int`, nor infer pending from counter subtraction when index 5 is unknown. Native writes index 5 as `-1` if either underlying API counter is negative, or their pending difference exceeds the configured budget. `getFramesRead`, callback-consumed frames, and stream pending are API observations, not a physical speaker playhead or measured acoustic latency. Callback-requested frames are not playback progress.

## Error codes and real-time ownership

JNI success is `0`. Stable negative errors are: `ERR_INVALID_ARGUMENT=-1`, `ERR_INVALID_HANDLE=-2`, `ERR_INVALID_STATE=-3`, `ERR_UNSUPPORTED_FORMAT=-4`, `ERR_OPEN_FAILED=-5`, `ERR_BUDGET=-6`, `ERR_STREAM_FAILED=-7`, and `ERR_JNI_ARRAY=-8`. `nativeOpen` reports the code in `info[0]`; it returns handle `0` if there is no live stream requiring cleanup, or a nonzero cleanup handle if close could not establish a Closed state. `nativeClose` treats a confirmed Closed state as quiesced (including an `ErrorClosed` result from an already-closed Oboe stream); any other close failure retains the handle/session for a safe retry. Other JNI calls return their code directly. The last-error message may include the underlying Oboe result for diagnosis without changing this API code.

The data callback performs only bounded SPSC copy, optional gain multiplication, silence fill, and lock-free 32-bit counter/error updates. It does not allocate, lock, enter JNI/Java, perform I/O, or stop/close/reopen the stream. Callback-visible state is separately owned until control code has stopped/closed the stream and observed callback quiescence. Real-time atomics are 32-bit and verified lock-free for every packaged ABI; no 64-bit atomic is used by the callback. Error callbacks only record state for later worker/control handling.

The SPSC ring is preallocated Float32 interleaved audio. Its single Java worker is producer and Oboe callback is consumer. PCM16 is converted to Float32 by the Java worker using preallocated scratch only on the Oboe path; the existing AudioTrack PCM16 byte path is unchanged. V2 Float32 is passed without an intermediate PCM16 conversion.

## Phase 5 implementation and offline verification

The Receiver keeps `AudioTrack` as the default. The local `anpod_receiver/output_engine` preference can request `oboeExperimental` for a new audio session; it does not hot-switch an active session. The session snapshot reports the actual engine and any same-format fallback reason. Oboe XRun count is reported separately; the legacy AudioTrack underrun field is unknown while Oboe is active. An initial Oboe open failure can use AudioTrack at the same negotiated format. A runtime Oboe failure closes the current generation and records a reason for the next requested Oboe session, which then uses AudioTrack; the preference remains unchanged. No SRC was added.

Offline verification on 2026-10-20 used JDK 17 and Android SDK 35:

- Equivalent reproducible command: `gradlew.bat -p src/AnPod.Receiver :app:assembleDebug :app:testDebugUnitTest --offline --no-daemon --console=plain -PreceiverBuildDir=<absolute-repo>\artifacts\phase5-android-build`. It completed successfully; all 55 Receiver JVM tests passed (0 failures, errors, or skips), including 23 `ReceiverServiceRobolectricTest` cases. Replace `<absolute-repo>` with the local checkout path.
- Four Android ABI CMake targets built. The packaged APK contains arm64-v8a, armeabi-v7a, x86, and x86_64 libraries; generated JNI declarations and APK exports match at 9 JNI symbols per ABI. `libanpod_oboe_output.so` and `liboboe.so` have ELF LOAD alignment 0x4000 on all four ABIs. This does not claim that every packaged dependency has 16 KiB ELF alignment; the 32-bit `libc++_shared.so` remains 0x1000.
- A standalone MSVC CPU test of the SPSC ring passed. The Windows MSVC CMake/CTest route did not pass: its build encountered C1902, so no Windows CTest result is claimed.
- The debug APK SHA-256 is `96101F5451004E039778196E5CBFF7ECB08B88EEB1547EC9EEB29C5BA2C8DA40`. APK inspection verified all 16 packaged `.so` ZIP data offsets are divisible by 16384 bytes. Android Build Tools 34 does not support `zipalign -P 16`; that command was not reported as passing. The APK contains the Oboe attribution and Apache-2.0 license assets. `apksigner` reports v1/v2 signatures from the debug signer only; this does not verify a release-signed upgrade.
- Reproduction logs are `artifacts/phase5-final-controls-gradle.log`, `artifacts/phase5-final-apk-validation.log`, `artifacts/phase5-final-apksigner.log`, and `artifacts/phase5-final-zipalign.log`. The Gradle command above used JDK 17, SDK 35, cached `GRADLE_USER_HOME`, and isolated `user.home`/`java.io.tmpdir` through `JAVA_TOOL_OPTIONS`; it did not install dependencies or use a device.

These are source/build/host-offline results only. No Receiver was installed on a device. Android UI visual inspection, physical output format, acoustic latency/quality, XRun behavior, routing/focus behavior on hardware, and long-duration stability remain `NOT_TESTED`. The debug APK is not a release-signature upgrade artifact.
