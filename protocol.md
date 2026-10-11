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
The fixed cross-language vector below remains deterministic test data only and
is not used as a session key.

## P5-A offline binding and authenticated session derivation

P5-A defines the offline binding record, mutually authenticated handshake, and
session key derivation used by the Rust and Android implementations. The
handshake is currently in-memory only: no USB transport, discovery, listener,
control path, or product connection flow invokes it.

The 69-byte binding record is `ANPB` (4 bytes), version `1` (1 byte), host ID
(16 random bytes), receiver ID (16 random bytes), and shared secret (32 random
bytes). Windows stores only the current-user DPAPI-protected record, with UI
forbidden and no machine-wide protection. Android stores the record under the
explicit app-private files directory via `AtomicFile`; the Android file is
app-private storage, not an encrypted-at-rest record. Both stores are
create-only, and neither creates its parent directory or overwrites an
existing binding.

Handshake messages use `ANP5`, version `1`, and a one-byte type. All integers
are big-endian. The caller exchanges complete messages but transport framing
is not defined here:

| Message | Type | Length | Body |
|---|---:|---:|---|
| Hello | 1 | 85 | host ID (16), receiver ID (16), host nonce (32), session ID (8), audio parameters (7) |
| Challenge | 2 | 86 | receiver ID (16), receiver nonce (32), receiver proof (32) |
| Finish | 3 | 38 | host proof (32) |
| Ready | 4 | 38 | ready proof (32) |

Audio parameters are encoding `1` (PCM16LE, 1 byte), channels (1 byte; 1 or
2), sample rate (u32, 8,000–192,000), and mode (1 byte; `0` balanced, `1` low
latency). The 118-byte authenticated transcript is `ANP5`, version `1`, host
role `1`, receiver role `2`, host ID (16), receiver ID (16), host nonce (32),
receiver nonce (32), session ID (u64), and the seven audio-parameter bytes.
Both peers verify the paired identities and role-specific proof before
advancing. A receiver emits Ready and returns session keys only after
validating Finish; a host returns keys only after validating Ready. A failed
message does not establish a session.

Proofs are HMAC-SHA256 over `ASCII(label) || 0x00 || transcript`, keyed by the
shared secret. Labels are `AnPod P5-A proof receiver v1`,
`AnPod P5-A proof host v1`, and `AnPod P5-A ready v1`; the ready proof appends
host proof then receiver proof to the transcript input. Session keys use
HKDF-SHA256 (RFC 5869): `salt = SHA256(ASCII("AnPod P5-A HKDF-SHA256 salt v1")
|| 0x00 || transcript)`, IKM is the shared secret, and each 32-byte output uses
one distinct ASCII `info` label:

* `AnPod P5-A key host-to-receiver audio AES-256-GCM v1`
* `AnPod P5-A key receiver-to-host audio AES-256-GCM v1`
* `AnPod P5-A key host-to-receiver control HMAC-SHA256 v1`
* `AnPod P5-A key receiver-to-host control HMAC-SHA256 v1`

The two audio keys are directional keys for the P3 AES-GCM nonce domains
`ANPH` and `ANPR`. Control keys are reserved for a future control protocol and
are not used by P5-A. Session and key objects are one-use/non-cloneable at the
API boundary and wipe their owned key bytes when closed or dropped. Fixed
cross-language test values verify the transcript, proofs, ready value, and all
four derived keys.

## P5-B USB binding provision path

The explicit Rust `receiver::provision` entry point is not registered with
Tauri IPC and has no product UI caller. It accepts an explicit host binding
file path, verifies the selected target as authorized USB, reads the active
Android user, and rechecks the installed APK identity against the local
trusted artifact before sending anything. If the DPAPI file is absent, Rust
creates and durably publishes the current-user protected record first. If it
already exists, provisioning loads and reuses that exact record, so a failed
or cancelled attempt can retry without generating a replacement secret. A
conflicting or unreadable local record fails closed.

The only write command is
`adb -s <USB serial> shell -T content write --user <current user> --uri content://app.anpod.receiver.state/binding`.
The 69-byte ANPB v1 record is sent only on the child process stdin pipe; it is
not passed in command arguments, URI parameters, shell extras, a temporary
plaintext file, or logs. Android reuses the existing DUMP-protected
`ReceiverStateProvider`. Only this exact URI and exact `w` mode opens the
single in-progress pipe worker. It accepts exactly 69 bytes followed by EOF,
checks the ANPB magic/version, and stores only under the Receiver's fixed
app-private files directory. Short, oversized, malformed, unknown-version,
other-path, or other-mode writes are rejected before storage. Existing equal
records are idempotently confirmed; a different saved record is a conflict
and is never replaced. No clear, unlink, or re-pair operation is defined.

The provider's read-only query on that fixed URI returns only `empty`,
`pending`, or `saved`, plus `SHA-256(complete 69-byte ANPB record)` when
available. This digest is a USB provisioning consistency commitment; it does
not independently authenticate a future network peer. The original health
query remains unchanged and read-only. Rust checks USB transport, current
Android user, and installed APK bytes again while waiting for the saved state,
and returns success only when the reported commitment equals the locally
persisted record. A `content write` process exit code or its output alone is
not completion evidence: AOSP `WriteCommand` copies stdin through
`ContentProvider.openFile(uri, "w")`, while its command wrapper can report an
exception without propagating it as a process failure. A missing confirmation,
conflict, identity change, ADB error, or cancellation leaves the Windows DPAPI
record available for retry. No ADB output is copied to logs or returned as a
diagnostic.

This remains a USB provisioning path only. It does not add LAN discovery,
listeners, automatic reconnect, audio/control startup, product IPC/UI,
unbinding, or reconfiguration. Real USB, system DUMP permission enforcement,
and Android private storage remain NOT_TESTED.

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
