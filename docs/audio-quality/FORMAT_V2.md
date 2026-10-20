# Audio format and authentication v2

This document defines the v2 descriptor, authentication and UDP wire formats. The
Receiver coordinator now has an additive P5-C v2 capability and a fresh-session
v2 authentication path. Product format support is still decided by the playback
endpoint; current Phase 3 work targets only stereo 48 kHz packed PCM24. Source
integration and synthetic loopback checks do not establish real playback quality
or device compatibility. All integer fields are unsigned big endian; PCM sample
bytes are little endian.

## P5-C negotiation and output reporting

P5-C remains frame version 1 with management major 3 and audio major 1. Capability
bit `0x80` (`CAP_AUDIO_V2`) is additive and is advertised only when the playback
endpoint has a v2 consumer. The host probes it in a mutually authenticated v1
session using a capability query whose required mask includes `0x82`; that
session is query-only and does not claim the pending connection intent. The host
half-closes its write side and waits for receiver EOF before opening a new socket
and performing a fresh v2 authentication. A receiver without the capability
returns its supported capability bitmap with a rejected result, allowing the
host to report that v2 is unavailable.

The fresh v2 session carries the selected 16-byte descriptor inside the
authenticated handshake transcript. It then uses the existing P5-C control and
background authorization lifecycle. Each receiver endpoint checks the session
descriptor before BIND; descriptors it does not support are rejected without
entering the audio decoder. The descriptor schema permits standard PCM16, hifi
48 kHz PCM24 and hires 96 kHz PCM24, while current Phase 3 product policy supports
only hifi 48 kHz stereo PCM24. A 96 kHz session remains unavailable until Phase 4.

Legacy playback progress remains exactly 70 bytes. A v2 session appends an
8-byte extension: schema `1`, actual opened output encoding (`0` unknown, `1`
PCM16, `2` packed24, `3` Float32), opened channel count (`0` unknown), reserved
zero, and actual opened sample rate as a big-endian u32 (`0` unknown). Before a
track opens, all fields after the schema byte are zero. These values describe the
AudioTrack format reported by the app; they do not prove the physical DAC's
effective precision.

**Implementation status (2026-10-20):** Java coordinator and synthetic loopback
fixture cover capability probing, fresh v2 authentication, per-format endpoint
gating, the 78-byte progress layout, and per-session PCM24 low-bit decode checks.
P5-C javac, protocol self-checks, Rust↔Java upgrade/rejection loopbacks, and the
two-generation owner pause/restart test pass. Java offline checks, 44 Android
Receiver JVM tests, and the focused frontend build/Playwright check pass. The
locked, offline, single-thread Rust suite passed **111 tests, 0 failed, 1
ignored, 2 filtered** (93.98 seconds); the filtered tests are the manual
PowerShell artifact check and explicitly requested real-USB Receiver test.
Real WASAPI capture, device AudioTrack opening, physical playback, and
acoustic/long-duration measurements remain `NOT_TESTED`.

## Descriptor (16 bytes)

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 1 | descriptor version = 2 |
| 1 | 1 | requested profile: 1 standard, 2 hifi, 3 hires |
| 2 | 1 | sample format: 1 PCM16LE, 2 PCM24 packed LE |
| 3 | 1 | channels = 2 |
| 4 | 1 | valid bits: 16 or 24 |
| 5 | 1 | bytes per sample: 2 or 3 |
| 6 | 1 | playback mode: 0 balanced, 1 low latency |
| 7 | 1 | flags = 0 |
| 8 | 4 | sample rate Hz |
| 12 | 2 | frame bytes = channels * bytes per sample |
| 14 | 2 | reserved = 0 |

Only standard=48000/16/4, hifi=48000/24/6 and hires=96000/24/6
are valid. Unknown fields, inconsistent combinations and nonzero reserved bits
are rejected. The selected descriptor must exactly equal the request.

## ANP5 v2 handshake

Header: `ANP5[4] | version=2:u8 | type:u8 | totalLength:u16`.
The length includes the eight-byte header. Identities are 16 bytes, nonces 32.

| Type | Total bytes | Body |
| --- | --- | --- |
| 1 HELLO | 96 | hostId, receiverId, hostNonce, sessionId:u64, requested descriptor |
| 2 CHALLENGE | 104 | receiverId, receiverNonce, selected descriptor, receiverProof[32] |
| 3 FINISH | 40 | hostProof[32] |
| 4 READY | 40 | readyProof[32] |

Transcript (143 bytes): `ANP5 | 2 | hostRole=1 | receiverRole=2 | hostId |
receiverId | hostNonce | receiverNonce | sessionId:u64 | requested | selected`.

Reuse HMAC-SHA256: `HMAC(bindingSecret, ASCII(label) | 00 | transcript | extra)`.
Labels are `AnPod P5-A proof host v2`, `AnPod P5-A proof receiver v2`, and
`AnPod P5-A ready v2`. Proofs have empty extra; READY extra is hostProof followed
by receiverProof. HKDF-SHA256 salt is SHA256 of
`ASCII("AnPod P5-A HKDF-SHA256 salt v2") | 00 | transcript`.
The existing 32-byte USB-provisioned secret is IKM. Four HKDF info labels match
v1's host/receiver audio AES-256-GCM and control HMAC-SHA256 labels with `v2`.
No terminator is appended to HKDF info. Each output is 32 bytes.

## ANPD v2

The 34-byte header remains: `ANPD | version=2:u8 | type:u8 | flags=0:u16 |
sessionId:u64 | sequence:u64 | sampleFrameIndex:u64 | payloadLength:u16`.
Types remain FORMAT=1 and AUDIO=2. The entire header is AES-256-GCM AAD;
the tag is 16 bytes. Nonce is `direction:u32 | sequence:u64`, with existing
ANPH/ANPR direction domains. Datagram limit remains 1200, payload limit 1150.
FORMAT contains exactly the selected 16-byte descriptor and frame index zero.
AUDIO must contain a nonempty whole number of selected sample frames.
No old session/key may be reused with a reset sequence. A decoder is explicitly
v1 or v2 and never accepts the other version. Replay state advances only after
authentication and format/payload validation succeed.
