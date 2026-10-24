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
- Historical baseline (PC 1.10.2): USB Direct setup suppressed automatic
  connection before its configuration window opened. Closing the window
  released the temporary owner. Once a confirmed switch attempt began, a
  separate sticky suppression
  survives failure and AOA connection/disconnection; only an explicit normal
  USB/LAN connection or re-enabling automatic connection clears it. Before an
  attempt, stopping the local ADB server requires explicit consent and affects every ADB debugging connection
  on that PC. After consent, the app rereads and validates the iQOO 11
  `VID_2D95&PID_6013&MI_02`, runs `adb kill-server`, then makes one WinUSB
  attempt. Failure is surfaced and retains suppression. At that baseline,
  existing USB/ADB and LAN sessions allowed balanced/low-latency switching;
  AOA remained locked. The 1.10.3 candidate behavior is recorded as a prior
  baseline below.
  Quality/format changes still require a new session. The wireless offer
  listener stays running and pauses accepting offers during this flow; the
  saved preference is retained.
- Previous PC 1.10.3 / Receiver 0.17.2 code 36 requires an explicit PC click
  for every initial connection, including a single LAN offer; remembered
  authorization does not start a connection. Only an unexpected drop from an
  established session may trigger bounded reconnect, constrained to the same
  transport, target, binding, profile, mode, and output. A manual disconnect
  cancels it; an initial failure is not retried in the background, and AOA
  failure does not fall back to ADB.
- In the 1.10.3 candidate, balanced and low-latency mode switching was
  supported on every transport. AOA requires Receiver capability `0x200` and a
  newly authenticated session with fresh keys and queue; an older Receiver
  without this capability prompts for an update. Hi-Res requires actual 96 kHz
  Windows output. A 48 kHz source produces a clear prompt, with no resampling
  or silent downgrade. AOD's USB Direct receive statistics use actual PCM
  ingress and are shared by Oboe and AudioTrack.
- Previous Receiver `0.17.2-v3-flow-candidate` / code 36 release APK: SHA-256
  `0f34c03b5618182fcd90f5a33883bd7a6de5f5a4696d9c1a64603f3f4983194e`, signer
  SHA-256 `f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`,
  source commit `ce81f03b54285691b3ce602461180516b60a1d4b`,
  `sourceModified=false`, and source-input SHA-256
  `2599b5dcc07c92303fc4bf1f082dc17ef01113c28d897276253cd41fd9199f6e`.
- Offline verification for the signed 1.10.3 source snapshot: Android
  `ReceiverServiceRobolectricTest` passed 33/33 and
  `AoaReceiverCoordinatorTest` passed 6/6 in the same run (XML: 0 failures,
  errors, or skipped tests). Earlier 32/32 and 8/8 runs are separate and are
  not added to those counts. Frontend TypeScript, Vite, and the Playwright
  matrix for four transports, modes, manual offers, and quality passed. The
  older broad UI harness remains partial because some assertions target the
  previous UI. Rust formatting/checks, five focused test groups, APK artifact
  reading, and the serial locked/offline suite passed (145 passed, 0 failed,
  1 ignored). Hardware playback, quality, long-duration stability, and APK
  upgrade verification remain `NOT_TESTED`.
- Source and offline loopback work does not claim hardware playback, quality,
  long-duration stability or APK upgrade verification. These remain
  `NOT_TESTED` pending real-device validation.

## PC 1.10.4 / Receiver 0.17.3 code 37 review corrections and final offline validation

The review found three session-lifecycle defects: PC AOA session restoration
could re-enter a held session lock; an old Android `AoaSessionStopped` callback
could terminate a newer coordinator; and the canonical 13-byte
`USER_STOPPED` acknowledgement was rejected by PC length checks and mistaken
for a failed AOA progress response, causing reconnect. The corrections release
the prior lock before retry, limit stop cleanup to its owning coordinator,
report genuine playback failures against their context, and validate the exact
stop acknowledgement alongside capability, authentication, session, and
sequence. The regular `OK` response-length rules remain unchanged.

The USB Direct window prioritizes a ready AOA device and keeps its switch
controls collapsible while preserving explicit consent before stopping the
PC-wide ADB server. Connecting still requires a PC click. AOD labels an
otherwise unknown wired or wireless ADB transport simply as `ADB`.

Final offline verification: Android `ReceiverServiceRobolectricTest` and
`AoaReceiverCoordinatorTest` passed 41 tests in one complete run (35 + 6; both
XML suites report 0 failures, errors, and skipped tests). Rust formatting and
offline checks passed; the final serial offline suite passed 149, with 0
failures and 1 ignored. The three focused regressions are included in that
suite: shared terminal cause and owner cleanup, direct TCP deadline/request,
and Java trust management-owner handling. The DPAPI hardlink case passed with
the required elevation and no product fallback. Frontend TypeScript, Vite, and
the AOA-focused Playwright suite passed 14 assertions, including 360/375-wide
layouts, large text, keyboard access, explicit connect, consent, focus, and the
native marker. Evidence: `.tmp-frontend-check/astra-ui-run/astra-ui-result.json`
and two screenshots. Astra's final read-only review passed; the original three
P1 findings and two UX items are closed, with no remaining evidenced P1/P2.

`AppState::recover_aoa_session_locked` was not invoked at runtime in this verification, so the
tests do not establish runtime recovery through that entrypoint. Physical AOA,
audio playback/quality, installation/upgrade, and long-duration behavior remain
`NOT_TESTED`.

The local Rust and Java test suites share fixed vectors to compare descriptor,
handshake, transcript, keys and packet bytes. The Java P5-C loopback fixture exposes the authenticated
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
registered source commits. Portable ZIP checks and its checksum are retained
in local release records.

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
