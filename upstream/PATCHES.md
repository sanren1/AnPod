# Patches applied to reused code

AnPod modifies LGPL-3.0-or-later code from ysbing/AudioShare. This file records
every change so the corresponding source can be produced and the obligation in
`LICENSES/LGPL-3.0-or-later.txt` satisfied (plan §2.2.5).

Base revision: `70db6b531db2c21b5a41a289722aa3996de67ebe`
(see `SOURCE_VERSIONS.md`).

---

## A. `audio_capture.cpp` — native WASAPI capture

### A0. Optional PCM callback sink *(P3 prototype)*

The additive `AudioCapture_SetPcmCallback` C ABI selects a borrowed PCM callback
before capture starts. When present, the capture thread routes converted PCM to
that callback instead of the legacy TCP socket; with no callback the original
TCP path remains unchanged. Callback/user-data lifetime is protected by a
mutex, and cleanup joins the capture thread before clearing the registration.
The Rust P3 prototype callback performs a nonblocking copy to a preallocated,
bounded queue. Product connection flows do not select this sink yet.

Each item maps to a defect identified in plan §3.4.

### A1. Listener binds to loopback only *(§3.4.3)*

**Before:** `addr.sin_addr.s_addr = htonl(INADDR_ANY);`
**After:** `addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);`

The ADB reverse tunnel terminates on the host's loopback, so the port never
needs to be reachable from the LAN. This closes an accidental exposure where any
host on the network could connect and receive the audio stream.

### A2. Handshake connect code is verified *(§3.4.4, §7.1)*

**Before:** the first byte sequence received was accepted unconditionally, then
handed to the callback.

**After:** a new export `AudioCapture_SetConnectToken(const char*)` records the
token the current session expects. `ListenThread` compares the received code and
drops any mismatching client, continuing to wait for the real one. An empty
token preserves the upstream permissive behaviour.

### A3. Format fallback validates the sample subtype *(§3.4.6)*

**Before:** anything that was not PCM16 was cast to `const float*` and
converted. A 24-bit or 32-bit integer mix format therefore produced noise.

**After:** `AnalyseMixFormat` inspects `WAVEFORMATEXTENSIBLE.SubFormat`,
`wBitsPerSample` and `Samples.wValidBitsPerSample`. Only
`KSDATAFORMAT_SUBTYPE_PCM` with 16 bits (16 valid bits) and
`KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` with 32 bits are accepted; anything else
returns `ANPOD_ERR_UNSUPPORTED_FORMAT` and `Initialize` fails instead of
playing garbage. Sample rate is range-checked to 8000–192000 Hz.

### A4. Bounded send backlog *(§3.4.5)*

**Before:** `SendAll` looped on blocking `send` with no timeout. A slow consumer
stalled the capture thread, and because WASAPI kept buffering, playback drifted
further behind real time with every packet.

**After:** the client socket gets `SO_SNDTIMEO = 200 ms`. Both the format header
and PCM payload use `SendExactBytes` from `send_exact.h` to finish partial
writes. A timeout or failed write closes the audio session and reports the
failure. The PCM stream has no packet framing, so skipping an unsent remainder
would corrupt sample alignment. The desktop can reconnect instead of allowing
the capture backlog to grow indefinitely.

### A5. Endpoint invalidation is reported *(§3.4.8)*

**Before:** `AUDCLNT_E_DEVICE_INVALIDATED` from `GetBuffer`/`GetNextPacketSize`
fell into the generic `FAILED(hr)` branch and silently ended capture.

**After:** it sets `ANPOD_ERR_DEVICE_INVALIDATED`, which the managed side polls
and turns into a session rebuild.

### A6. Diagnostics exports *(§3.1 note)*

Added `AudioCapture_GetLastErrorCode`, `AudioCapture_GetLastErrorMessage` and
`AudioCapture_ClearLastError`. Upstream only implemented these for macOS, so the
Windows DLL previously had no way to explain a failure. The six original exports
are unchanged and still resolve.

### A7. COM lifetime is balanced *(§5.5)*

**Before:** `CoInitializeEx` was called on every `Initialize` with no matching
`CoUninitialize`.

**After:** `g_bComInitialized` tracks whether this module performed the
initialisation, and `Cleanup` only calls `CoUninitialize` when it did. The
`RPC_E_CHANGED_MODE` case (another apartment already owns the thread) is
explicitly not balanced.

### A8. Resource leaks on error paths

**Before:** several early returns in `Initialize` leaked `IMMDevice`,
`IAudioClient` or the `WAVEFORMATEX` allocation.

**After:** every failure path funnels through `AudioCapture_Cleanup`, which
releases whatever was acquired. `IMMDevice` is released exactly once on the
success path.

### A9. New public header

`audio_capture.h` did not exist upstream. It declares the exports, the callback
signature, the calling convention and the error-code table, so the managed side
has one authoritative source for the ABI.

### A10. `AudioCapture_SetStreamingEnabled` — pause without dropping the peer

**Before:** the only way to stop forwarding audio was `AudioCapture_Stop`, which
closes `g_clientSocket` and `g_listenSocket`. The Android payload's `read()` then
returns -1, its `try (LocalSocket …)` block unwinds, and the process exits — so
muting and unmuting the phone cost a full reconnect (reverse mapping, payload
restart, handshake) every time.

**After:** a `g_bStreamingEnabled` flag that the capture thread reads once per
iteration. While it is false the thread still drains and releases the WASAPI
buffer — so the audio engine never stalls — but writes zeros into the output
buffer instead of converting the captured frames:

```c
if (!g_bStreamingEnabled) {
    memset(outBuf, 0, (size_t)frames * 2 * sizeof(short));
}
```

The socket stays open, the peer stays connected, its `AudioTrack` stays alive
and primed, and resuming is seamless with no re-handshake. Reset to `TRUE` by
`ResetSessionState` so a new session always starts audible.

These exports are **required** by the managed side (`NativeAudioCapture`): an
old DLL would silently ignore the call and keep playing, which is a worse
failure than refusing to load.

### A11. Deliberately unchanged

* The `Sleep(1)` + `timeBeginPeriod(1)` polling loop is kept as-is, per plan
  §3.4.7. `timeEndPeriod(1)` is still paired on every exit path.
* The 48 kHz stereo PCM16 preferred-format negotiation.
* `SO_SNDBUF = 32 KiB`, `TCP_NODELAY`.
* Release-before-send ordering.

### A12. Per-session PCM byte counter

`AudioCapture_GetSentPcmBytes` is an additive C ABI export. Each successful TCP
write of PCM payload bytes increments the counter, including partial writes;
the 8-byte format header is excluded. The counter resets before a new capture
and after cleanup. All original exports remain available.

---

## B. `Main.java` — Android playback

### B1. Full header validation *(§3.3)*

**Before:** only `sampleRate` was read; `bitsPerSample` was parsed and then
ignored, so a 24-bit stream would have been fed to a 16-bit `AudioTrack`.

**After:** `validateFormat` rejects a sample rate outside 8000–192000, a channel
count other than 1 or 2, and any bit depth other than 16, throwing
`UNSUPPORTED_*` before the `AudioTrack` is created.

### B2. Machine-readable lifecycle logging *(§5.6)*

Lifecycle events are written to stderr as `ANPOD START|FORMAT|PLAYING|STOP|
RELEASED|ERROR ...`, which `AndroidProcessSession` mirrors into the Windows log.

### B3. Guaranteed release

`AudioTrack.stop()` and `release()` are each wrapped against `Throwable` and
their outcome logged as `STOP_FAILED` / `RELEASE_FAILED`, so `release()` always
runs and a failed stream cannot leak the track.

**Why `Throwable` and not `IllegalStateException`:** the original upstream guard
only caught `IllegalStateException`. Under `app_process` a `NullPointerException`
escaped from `stop()` and *replaced* the genuine exception on the way out,
destroying the only evidence of the real fault. Catching the widest type here
keeps the original cause intact for the caller.

### B4. Argument length limits *(§3.4.4)*

`Options.parse` rejects a socket name or connect code longer than 128 bytes.

### B5. Package rename

`com.ysbing.audioshare` → `com.anpod.server`. The wire protocol is unchanged, so
either side can be swapped independently; the class name in
`AndroidServerLayout.MainClass` must match whichever payload is deployed.

### B6. `AudioTrack` state is checked before use

**Before:** the track was built and `play()` called without confirming the
constructor had actually produced a usable instance. On an unsupported format or
an exhausted device the object stayed in `STATE_UNINITIALIZED` and the failure
surfaced later as an opaque exception from the audio stack.

**After:** `getState()` is compared against `STATE_INITIALIZED` immediately
after construction; anything else raises `IOException("AUDIO_TRACK_NOT_INITIALIZED
state=… sampleRate=… channelMask=… bufferBytes=…")`, which the Windows log shows
verbatim.

### B7. `Context` is injected into `PlayerBase` *(the reconnect-loop fix)*

**Symptom:** the desktop app connected, started streaming, then ~120 ms later
reported `Android audio process exited unexpectedly` and reconnected — repeating
every ~1.5 s indefinitely.

**Root cause:** AnPod runs its payload with `app_process`, which never creates an
`ActivityThread` application instance. `AudioTrack.play()` calls up into
`PlayerBase.updateState()`, which dereferences
`mContext.getPackageManager()` — with `mContext == null` this throws
`NullPointerException` the moment playback starts. The upstream code, written for
an in-app (`Activity`) context, never had to care.

**After:** `resolveSystemContext()` obtains a real system `Context` from a bare
`ActivityThread` instance:

```java
android.os.Looper.prepare();
ActivityThread thread = ActivityThread.class.getDeclaredConstructor().newInstance();
return (Context) ActivityThread.class.getMethod("getSystemContext").invoke(thread);
```

`attachPlayerContext(track)` then walks the instance's class hierarchy looking
for a `Context`-typed field and injects it, logging
`CONTEXT_ATTACHED <Type>.<field>` on success or `CONTEXT_FIELD_NOT_FOUND`.
The track is built *without* a context and patched afterwards, because
`AudioTrack.Builder.setContext()` throws `UnsupportedOperationException:
Cannot create AudioTrack` on these builds.

**Two dead ends, recorded so they are not retried:**

* `AudioTrack.Builder.setContext()` — rejected by the platform, see above.
* `ActivityThread.systemMain()` — *permanently poisons* `AudioTrack` creation in
  the process: tracks built before the call still play, but every track built
  afterwards fails. The bare constructor avoids the side effect entirely.

---

### B8. Playback buffer modes and complete PCM frames

`playbackMode` selects balanced or low-latency playback. Balanced retains the
existing track capacity and minimum-buffer pre-roll. On API 24+, low-latency
requests a 15 ms effective buffer; API 31+ also requests a 5 ms start threshold.
Actual sizes, rejected requests, fallback mode and underruns are logged. API 23
uses the supported buffer-size getter and retains balanced playback.

`Pcm16FrameStream.java` retains incomplete frames across socket reads and finishes
partial AudioTrack writes. Zero or failed writes stop the session with a
diagnostic. Pre-roll reads are limited to the actual read-array size even when
the effective playback buffer is larger. The checks in `tests/` exercise
fragmented frames, partial writes and large pre-roll targets.

### B9. Installed Receiver service adaptation

`src/AnPod.Receiver/app/src/main/java/app/anpod/receiver/ReceiverService.java`
adapts the AudioTrack playback path from `Main.java` to an installed Android
`Service`. The 8-byte little-endian header, abstract `LocalSocket` handshake,
PCM16 format and playback modes remain compatible with the Windows native
capture implementation. AnPod changes add the `mediaPlayback` foreground
service lifecycle, audio focus, cancellation-owned session resources, and a
protected provider for playback readiness and actual buffer/underrun metrics.
Resource creation and use belong to the audio worker; stop closes its socket
and cancels it, while that worker releases its own `AudioTrack` and audio focus.

`src/AnPod.AndroidProtocol/src/main/java/com/anpod/server/Pcm16FrameStream.java`
is new AnPod code, shared by the installed Receiver and `app_process` path. It preserves complete PCM16
frames across socket reads and partial AudioTrack writes; it is not copied
from the upstream repository.

### B10. Internal UDP parser and playback adapter *(P4-A, not enabled)*

The new `UdpSessionContext`, `UdpPacketDecoder`, `UdpPcmQueue`, and `UdpReceiver`
classes are AnPod code and add no third-party dependency. A caller inside the
app process must supply the already authenticated peer, session ID, key, and
local bind address in memory. The adapter validates and decrypts only Format
and Audio packets, then presents aligned PCM through a queue-backed
`InputStream` to the existing `Pcm16FrameStream` and AudioTrack worker. The
network thread owns only its UDP socket and short-lived packet handling; the
existing audio worker owns AudioTrack and focus. No Intent, exported component,
discovery, pairing, network control message, or product flow starts this path.

### B11. Offline binding and authenticated key derivation *(P5-A, in-memory)*

`src-tauri/src/security.rs`, Android `PairingProtocol`, and Android
`BindingStore` are new AnPod code. They define the versioned 69-byte device
binding, Windows current-user DPAPI storage, Android app-private `AtomicFile`
storage, matching in-memory mutual-authentication messages, transcript-bound
proofs, and four direction/purpose-separated HKDF-SHA256 outputs. The Android
record uses app-private storage and is not encrypted at rest.

Rust dependencies added for the protocol are `hkdf 0.12.4` and `hmac 0.12.1`
(`MIT OR Apache-2.0`); `subtle 2.6.1` (`BSD-3-Clause`) is pulled transitively
for constant-time HMAC verification. Windows DPAPI uses the existing `windows`
crate with its Cryptography feature enabled; Android uses the platform
`AtomicFile` and JCA APIs.

### B12. USB binding stream and remote confirmation *(P5-B, no product entry)*

`receiver::provision` accepts an explicit DPAPI binding path and reuses its
persisted material on retry. It sends the ANPB record only through stdin to
the existing selected USB ADB child, then verifies the fixed provider status
URI against the record commitment while rechecking current user and installed
APK bytes. The existing DUMP-protected `ReceiverStateProvider` accepts only
the fixed binding URI and write mode, uses one bounded in-progress pipe
worker, validates exact record length/version, and writes under the fixed
app-private files directory. Same-record writes confirm idempotently;
different records conflict without replacement. The Rust entry is not
registered with Tauri IPC. No real-device ADB or Android storage test is
claimed.

### B13. Shared TCP/USB PCM backlog trim *(P6-B)*

The existing AnPod PCM stream path trims stale queued frames when the consumer
falls behind and applies a short fade at the trim boundary. This is new AnPod
code in the shared Android protocol helper; it adds no dependency and does not
change the Native DLL protocol or the legacy Android service wire format.

### B14. Optional Oboe output adapter *(P5, experimental)*

`AudioTrackPlaybackOutput.java` extracts the existing AudioShare-adapted
`AudioTrack` writes and release path from the installed Receiver service; the
default engine and legacy PCM16 byte path remain `AudioTrack`. The optional
Oboe selection is a local Receiver preference for a new playback session. Its
Java/native implementation—`OboeNativeBridge.java`, `OboePlaybackOutput.java`,
`OboeOutputException.java`, `PlaybackOutput.java`, `OboeNativeBridge.cpp`, and
`SpscFloatRing.h`—including the preallocated Float32 SPSC ring, is AnPod code.
No Oboe source is copied: the build consumes the pinned
`com.google.oboe:oboe:1.10.0` Apache-2.0 Prefab AAR described in
`THIRD_PARTY_NOTICES.md` and `SOURCE_VERSIONS.md`. Runtime Oboe failure closes
the active audio generation; a later session reports a one-shot same-format
AudioTrack fallback rather than hot-switching the current stream. An initial
open failure can use same-format AudioTrack in that session.

## C. Not modified

* The 8-byte little-endian format header layout.
* The `LocalSocket` abstract-namespace handshake ordering.
* `MODE_STREAM` + `ENCODING_PCM_16BIT` + `PERFORMANCE_MODE_LOW_LATENCY` on
  Android 8.0+.
* Balanced-mode pre-roll of one minimum buffer before `play()`.

---

## D. Rebuilding

The corresponding source for the LGPL portions is exactly the contents of
`src/AnPod.Native/`, `src/AnPod.AndroidServer/`, and the AudioShare-derived
playback implementation in `src/AnPod.Receiver/` in this repository. The
shared PCM helper in `src/AnPod.AndroidProtocol/` is new AnPod code. Build
instructions are in `README.md`; `packaging/build-native.ps1`,
`packaging/build-android.ps1`, and `packaging/build-receiver.ps1` reproduce the
native library and both Android paths from source.
