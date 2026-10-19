# Phase 1 compatibility

- Standard product playback remains the original v1 path, including its two
  distinct seven-byte parameter encodings, transcript, labels and vectors.
- ANPB binding records remain 69-byte version 1. No new pairing, trust,
  provisioning or transport is introduced.
- P5-C frame version 1, management major 3 and audio major 1 remain unchanged.
  No v2 capability is advertised in this phase. Authenticated CAPABILITIES_INFO
  is the future upgrade discovery point; unauthenticated discovery is not one.
- v2 negotiation and packets are checked offline. They do not create an
  AudioTrack, send captured audio or claim PCM24 playback support.
- v2 selection changes are rejected. A future lower-quality choice requires
  user agreement and a fresh authenticated session, never silent fallback.
- Hardware playback, quality, long-duration stability and APK upgrade checks
  remain NOT_TESTED for this phase.

The shared fixed vectors are in `tests/audio-v2-vectors.txt`; Rust and Java
checks independently compare descriptor, handshake, transcript, keys and packet
bytes against that file. Run the targeted Rust security/protocol tests and
`packaging/check-java-offline.ps1`.
