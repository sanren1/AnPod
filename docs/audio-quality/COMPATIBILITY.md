# Audio protocol compatibility

- Standard playback over the existing LAN/UDP and USB/ADB routes remains the
  original v1 path, including its two distinct seven-byte parameter encodings,
  transcript, labels and vectors. AOA is a separate accessory transport and
  uses the authenticated v2 audio format for all three profiles, including
  Standard PCM16.
- ANPB binding records remain 69-byte version 1. No new pairing, trust, or
  provisioning flow is introduced. AOA is an explicit, separate transport.
- P5-C frame version 1, management major 3 and audio major 1 remain unchanged.
  `CAP_AUDIO_V2` (`0x80`) is additive and is advertised only when the Receiver
  playback endpoint provides a v2 consumer. P5-C major versions stay 3/1.
- The host discovers v2 support through an authenticated v1 capability-only
  probe requiring `0x82`. The probe does not claim the pending connection intent.
  The host half-closes that socket and waits for EOF before authenticating a new
  v2 session, so the receiver never has two authenticated owners.
- A v2 session binds the format descriptor to fresh authentication proofs and
  uses the existing P5-C bind, mode, start/stop and background authorization
  lifecycle. The endpoint checks each descriptor before BIND. Phase 3 enabled
  48 kHz stereo packed PCM24; Phase 4 adds experimental profile 3 at exactly
  96 kHz stereo packed PCM24. Both host and Receiver require exact profile/rate/
  representation agreement. A 48 kHz mix is not upsampled to satisfy Hi-Res;
  unsupported requests fail and require the user to select a new session format.
- A v2 playback progress response preserves the original 70-byte payload and
  appends the 8-byte actual-output schema. V1 sessions and older hosts retain the
  70-byte shape.
- Old host/receiver combinations continue using the original v1 path. A v2
  request to a receiver without `CAP_AUDIO_V2` is reported as unsupported and
  does not silently start playback with a different format.
- AOA derives a session-only binding secret from the existing ANPB binding;
  it does not change the 69-byte saved ANPB record or reuse the LAN secret as
  an AOA packet key. All AOA audio profiles use the independent ANPU stream
  envelope and ANPD v2 format. The PC app can explicitly switch the supported
  iQOO 11 V2243A from `VID_2D95&PID_6013&MI_02` into accessory mode after user
  confirmation; this does not start playback. The product accessory identity
  is `AnPod` / `AnPod USB Direct`; the separate
  `AnPod Lab` / `AOA Bulk Probe` identity belongs only to the echo probe.
- The AOA Receiver path requires Android API 24 or later for cancellable
  `FileChannel` I/O; the app's minimum SDK remains 23 and its existing UDP and
  ADB routes remain available there. Windows accessory access is restricted to
  the selected WinUSB accessory interface; on composite 18D1:2D01 devices,
  MI_01 remains ADB and is not opened by the audio path.
- PC 1.10.2 USB Direct setup suppresses automatic connection before its
  configuration window opens. Closing the window releases this temporary
  owner. Once a confirmed switch attempt begins, a separate sticky suppression
  survives failure and AOA connection/disconnection; only an explicit normal
  USB/LAN connection or re-enabling automatic connection clears it. Before an
  attempt, stopping the local ADB server requires explicit consent and affects every ADB debugging connection
  on that PC. After consent, the app rereads and validates the iQOO 11
  `VID_2D95&PID_6013&MI_02`, runs `adb kill-server`, then makes one WinUSB
  attempt. Failure is surfaced and retains suppression. Existing USB/ADB and
  LAN sessions allow balanced/low-latency switching; AOA remains locked.
  Quality/format changes still require a new session. The wireless offer
  listener stays running and pauses accepting offers during this flow; the
  saved preference is retained.
- Source and offline loopback work does not claim hardware playback, quality,
  long-duration stability or APK upgrade verification. These remain
  `NOT_TESTED` pending real-device validation.

The shared fixed vectors are in `tests/audio-v2-vectors.txt`; Rust and Java
checks independently compare descriptor, handshake, transcript, keys and packet
bytes against that file. The Java P5-C loopback fixture exposes the authenticated
probe/fresh-session path to the Rust host and uses synthetic playback. Focused
Rust↔Java upgrade/rejection and two-generation owner pause/restart checks pass;
the fixture's final PCM24 low-bit result is true only if every requested session
decoded the Q.23 samples before its own PAUSE.

Offline Receiver JVM counts are phase-specific: Phase 3 had 44 passing tests,
Phase 4 had 47, and the Phase 5 Oboe integration has 55 passing tests (0
failures, errors, or skips). The Phase 4 profile-3 Rust↔Java loopback and owner
pause/stop/mode fresh-session checks passed. Its `cargo fmt -- --check`, normal
strict Clippy, and locked offline single-thread Rust suite passed with 114
passed, 0 failed, 1 ignored, and 2 filtered in 93.89 seconds. The filtered cases
were the manual PowerShell artifact check and explicitly requested real-USB
Receiver test. Phase 5 keeps the existing wire formats and transport paths; the
experimental output engine is a local Receiver preference, defaults to
AudioTrack, and only affects a new session. Oboe XRun counts remain separate
from AudioTrack underruns; unsupported/open failures use same-format AudioTrack
fallback, while runtime failures are reflected on the next audio generation.

Phase 5 source and offline build/JVM acceptance does not establish hardware
playback. Phase 6a–6c source integration and offline gates are complete,
including the product USB permission/state entrypoint. The final Phase 6c
Rust full suite passed 137/0/0 ignored/2 filtered before the last opaque-label
change; after that change, fmt, AOA-focused 18/18, and the Windows target check
passed. Android passed 70 JVM tests across 12 suites; a final callback-only
test then passed focused 1/1, with Java offline checks 7/7. The frontend
production build and persistent AOA smoke check passed. These are source and
offline results, not hardware validation. The signed Receiver APK is built.
Portable build metadata records the actual packaged component hashes and
registered source commits. Portable ZIP checks and its checksum are recorded
in `plan_dev.md`, which is not included in the package.

The previous Receiver review candidate was version `0.17.0-v3-flow-candidate` / code 34,
source commit `8ca9638db128f99e36769f42de18ed9244f457c2`, with
`sourceModified=false`; its APK SHA-256 is
`711c20c4348b1865d0475244271271f1fc32d70888f1f77054481bba138fed83`. Its
verified release signer is
`f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`, and the
exact APK passed the Rust artifact-read test. This verifies the built
candidate artifact only; installation and upgrade on a device remain
`NOT_TESTED`. The prior crash-fix package was PC 1.10.1 with Receiver
`0.17.1-v3-flow-candidate` / code 35. Its release-signed APK SHA-256 is
`8f8e76e4b9e2dc1e3bb8d4575f5e21e77838056b2b55536c5736eedcccf4f15f`, built
from source `2bd998d65fe650bb225c09d7353c5c2ba39febfc` with
`sourceModified=false` and the existing release signer. The exact artifact
passed the Rust artifact-read test; installation and upgrade on a device
remain `NOT_TESTED`.

Real WASAPI capture, physical output format or precision, AOA attach/permission/
detach behavior on hardware, acoustic quality/latency, hardware routing/focus
behavior, and long-duration stability remain `NOT_TESTED`. The Phase 5 debug
APK is not release-upgrade evidence; a release-signed APK build alone also
does not prove successful installation or upgrade on a device.
