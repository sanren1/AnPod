# Audio protocol compatibility

- Standard product playback remains the original v1 path, including its two
  distinct seven-byte parameter encodings, transcript, labels and vectors.
- ANPB binding records remain 69-byte version 1. No new pairing, trust,
  provisioning or transport is introduced.
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
playback. Real WASAPI capture, physical output format or precision, acoustic
quality/latency, hardware routing/focus behavior, and long-duration stability
remain `NOT_TESTED`. The verified APK is debug-signed and does not verify a
release-signed upgrade.
