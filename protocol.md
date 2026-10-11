# P3 UDP prototype protocol

This document fixes the P3 wire format used by the localhost prototype and the
P4-A Android parser/adapter. No product connection flow selects the Android
adapter: it requires an in-memory session context from a future authenticated
coordinator, and the Windows prototype still sends only to `127.0.0.1`.

## Datagram

Every datagram is one 34-byte big-endian header, ciphertext, and a 16-byte
AES-GCM tag. The header is authenticated as associated data (AAD); only the
payload is encrypted.

| Offset | Size | Field | Value / encoding |
|---:|---:|---|---|
| 0 | 4 | Magic | ASCII `ANPD` |
| 4 | 1 | Version | `1` |
| 5 | 1 | Type | `1` Format, `2` Audio |
| 6 | 2 | Flags | zero, unsigned big-endian |
| 8 | 8 | Session ID | unsigned big-endian |
| 16 | 8 | Sequence | unsigned big-endian, starts at zero |
| 24 | 8 | Sample frame index | unsigned big-endian |
| 32 | 2 | Payload length | plaintext bytes, unsigned big-endian |

The maximum UDP payload is 1200 bytes, including header and tag; therefore
plaintext is at most 1150 bytes. Any version, type, flags, declared length, or
datagram size outside these values is rejected before decryption. Audio payload
is interleaved signed PCM16 little-endian. A format payload is exactly seven
bytes: encoding `1` (PCM16LE), channel count (`u16` BE), and sample rate
(`u32` BE). The first Format packet has sequence and sample frame index zero.
A 48 kHz stereo 5 ms audio packet contains 240 frames / 960 bytes; its UDP
payload is 1010 bytes.

## Authentication, nonce, and replay

Cipher suite: AES-256-GCM with a 256-bit per-session key, 96-bit nonce, and
128-bit tag. The nonce is `direction-domain (u32 BE) || sequence (u64 BE)`;
host-to-receiver is `ANPH` (`0x414e5048`) and receiver-to-host is `ANPR`
(`0x414e5052`). A key must be fresh for every session and must never be reused
with a sequence in the same direction. The session ID binds the expected
session identity in the authenticated header; it is not a key exchange. The
Rust loopback sender creates a fresh random 64-bit session ID and 256-bit key
with the existing `getrandom` dependency. The non-cloneable session material is
moved into the sender and kept only in memory; rebuilding a sender requires a
new material value. The loopback mock receives the same material in memory.
Persistent device binding, HKDF, USB pairing, and remote key transport remain
for P5. The fixed cross-language vector below remains deterministic test data
only and is not used as a session key.

The receiver checks bounds, version, type, flags, and session identity. The
Android parser authenticates before checking replay eligibility; the Rust P3
mock checks replay eligibility before AEAD. Both commit a sequence only after
authentication and packet-specific format validation, so invalid auth/format
does not mutate the replay window or audio queue. The replay window holds 64
unsigned sequence positions; duplicates and older packets outside the window
are rejected. Sequence exhaustion stops the sender instead of wrapping and
reusing a nonce.

## Queue and prototype scope

The Native PCM callback copies into eight preallocated stereo slots, each sized
for at most 4096 frames. It never waits for the queue lock or a socket; a busy
lock drops the incoming block, and a full queue drops its oldest block to keep
latency bounded. Rust packetizes on a dedicated worker and sends only to IPv4
loopback. Android uses a separate receive thread to authenticate and enqueue
packets; a queue-backed `InputStream` supplies the negotiated format header and
PCM bytes to the existing `Pcm16FrameStream` / single AudioTrack playback
worker. Its eight-packet queue sorts by unsigned sample frame index and drops
old history on overflow or pause. The pause watermark advances through every
discarded packet, and the stream adapter drops any unread bytes from a packet
it had already taken when the pause boundary changes. No UDP control packet or
default listener is implemented. This does not claim one-way latency or
real-time performance.

## Cross-language vector

The Rust test `protocol::tests::java_jca_aes_gcm_vector` and
`tests/AnPodUdpVectorCheck.java` use the same key, nonce, 34-byte AAD, and
payload. The Java entry also sends the fixed packet through the real Android
parser, then runs the merged parser/queue/loopback checks. Both vector paths
must produce this complete packet:

```text
414e50440101000001020304050607081112131415161718212223242526272800073bd32562c2e2cdfe093c22223a9705d1ae457629baad04
```

The vector is test data only and must not be used as a session key.
