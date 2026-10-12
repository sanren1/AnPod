# P3 UDP prototype protocol

This document fixes the P3 wire format used by the localhost prototype and the
P4-A Android parser/adapter. The P3 mock sender remains loopback-only. The
separate P5 authenticated session can use a caller-selected private IPv4
interface and authenticated literal peer. An explicit Tauri backend command
now wires bootstrap/discovery, binding selection, playback, and recovery; no UI
caller or default connection flow starts this path.

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
handshake API remains in-memory; the explicit P5-F backend obtains binding
material through USB provisioning or a selected saved binding, then uses the
existing handshake over the selected private IPv4 connection.

The 69-byte binding record is `ANPB` (4 bytes), version `1` (1 byte), host ID
(16 random bytes), receiver ID (16 random bytes), and shared secret (32 random
bytes). Windows stores only the current-user DPAPI-protected record, with UI
forbidden and no machine-wide protection. Android stores the record under the
explicit app-private files directory via `AtomicFile`; the Android file is
app-private storage, not an encrypted-at-rest record. Both stores are
create-only and never overwrite an existing binding. Product provisioning
creates the selected per-device/user parent directory before the first write.

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

The Rust `receiver::provision` entry point is not registered as a standalone
Tauri command; it is called only by the explicit `start_network_audio` backend
flow. It accepts a per-device/user host binding file path, verifies the
selected target as authorized USB, reads the active
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

The provider's read-only query on that fixed URI returns `empty`, `pending`,
`saved`, or `failed`, a fixed `bindingFailure` code, and
`SHA-256(complete 69-byte ANPB record)` when available. Failure codes are
limited to `cancelled`, `timeout`, `invalid_record`, `pipe_error`,
`storage_error`, and `conflict`; no exception text or record bytes are exposed.
The provider polls the pipe fd in bounded slices against one 15-second total
deadline, checks `CancellationSignal` while waiting, requires exactly 69 bytes
and EOF within that deadline, and releases the single-writer slot on every
worker exit. Cancellation, timeout, malformed input, and storage errors do not
write a record; an existing Android record is never replaced. The worker owns
and closes its read descriptor. This digest is a USB provisioning consistency
commitment; it does not independently authenticate a future network peer. The
original health query remains unchanged and read-only. Rust checks USB
transport, current Android user, and installed APK bytes again while waiting
for the saved state, and returns success only when the reported commitment
equals the locally persisted record. A `content write` process exit code or
its output alone is not completion evidence: AOSP `WriteCommand` copies stdin through
`ContentProvider.openFile(uri, "w")`, while its command wrapper can report an
exception without propagating it as a process failure. A missing confirmation,
conflict, identity change, ADB error, or cancellation leaves the Windows DPAPI
record available for retry. The stdin write, child wait, and both output
drains share one operation deadline and cancellation token; on failure Rust
terminates only its own child and aborts and joins both drain tasks. ADB output
is reduced to presence markers and is not copied to logs or returned as a
diagnostic.

Provisioning itself remains USB-only. The P5-F backend checks the Receiver
network protocol version before calling it and never copies a global binding
between devices or Android users. There is no unbind, clear, or silent re-pair
operation. Real USB, DUMP permission enforcement, and Android private storage
remain NOT_TESTED.

## P5-C authenticated session prototype

P5-C composes the existing P5-A handshake and P3 ANPD audio datagrams; it does
not change either format. The prototype is explicitly started by an internal
caller. Its legacy check path binds its temporary TCP listener and UDP
sockets to `127.0.0.1`; the explicit interface path binds each to the caller's
selected private IPv4 address. There is no default listener; optional discovery
is a separately started caller action. The host connects to the explicit literal IPv4 TCP address. The receiver accepts
loopback peers on the legacy check path and private IPv4 peers on the explicit
path. The host creates its UDP socket before authentication so
its local port is known; the receiver selects its UDP port after authentication.
The authenticated BIND exchange fixes the host UDP port, while the receiver
UDP port is returned in the authenticated ACK. The receiver's expected UDP
peer is exactly the accepted TCP peer address plus the authenticated host UDP
port. No UDP source or discovery packet can change either endpoint.

P5-A's four TCP handshake frames remain byte-for-byte unchanged and are read
with their existing fixed lengths: HELLO 85 bytes, CHALLENGE 86 bytes, FINISH
38 bytes, READY 38 bytes. Their `ANP5`, version 1, and message type bytes are
followed by the existing identity, nonce, session, parameter, and proof bytes.
The existing transcript authenticates host/receiver IDs, both nonces, session
ID, and PCM parameters. No binding is created or replaced during a handshake;
each side must receive an already-loaded binding record.

After READY, control is carried on the same TCP connection. Each control frame
has a two-byte big-endian length prefix, then a 30-byte header, payload, and
32-byte HMAC-SHA256 tag. The header is `ANPC` (4 bytes), version 1 (1), type
(1), zero flags (2), direction (4), session ID (8), sequence (8), and payload
length (2), all multi-byte integers big-endian. The HMAC covers the complete
header and payload; the length prefix is checked against the header and frame
size. Directions reuse `ANPH` host-to-receiver and `ANPR` receiver-to-host.
The selected P5-A directional control key authenticates every field, including
peer role, session, sequence, type, and length. Control frames are limited to
1200 bytes. The control sequence starts at zero independently in each
direction and must increase by exactly one. Repeating the exact immediately
previous command returns its cached ACK without applying it again; changing a
frame while reusing its sequence, skipping a sequence, or using an old session
is rejected. TCP provides transport retransmission, so the prototype does not
add application retries or UDP rate limiting.

Control types are BIND (1), SELECT_MODE (2), START (3), PAUSE (4), RESUME (5),
STOP (6), HEARTBEAT (7), and ACK (127). BIND carries the host UDP port as an
unsigned 16-bit integer. SELECT_MODE carries the one-byte P5-A mode and is
accepted only when it exactly matches the authenticated handshake parameters;
changing mode requires a fresh authenticated session. START/PAUSE/RESUME/STOP
and HEARTBEAT carry no payload. ACK carries the acknowledged command type
(1 byte), command sequence (8), result (1), receiver state (1), and receiver
UDP port (2). The port is nonzero only in the successful BIND ACK. Results are
OK (0), rejected (1), or failed (2). States are authenticated (0),
TransportConnected (1), ReceiverRunning (2), Streaming (3), paused (4), and
stopped (5). ACK has its own receiver-to-host sequence and HMAC.

Audio remains the existing ANPD v1 UDP datagram: 34-byte authenticated header,
PCM16LE payload, AES-GCM tag, and the negotiated P5-A session ID. The sender
uses the P5-A host-to-receiver audio key. The receiver uses that same derived
direction key. Authenticated P5-A sample rate and channel count initialize the
production receiver's PCM format and stream header, so playback does not depend
on receiving the sender's one-time FORMAT packet. If FORMAT arrives, it must
use sequence zero and match those authenticated parameters; a mismatch is
dropped without changing replay state or playback, and a matching late FORMAT
does not restart the queue or stream header. AUDIO sequence zero is invalid;
later PCM payloads must be aligned to the authenticated format.
Both balanced and low-latency modes use the existing 5 ms target for UDP
packetization, bounded further by the 1200-byte datagram limit. Mode selection
does not claim a transport packetization or performance difference; it passes
the authenticated mode to the existing AudioTrack path for buffer behavior.
START is the first point where the receiver opens the
authenticated UDP session for playback; the host starts its sender only after
the successful START ACK. STOP closes the sender, clears the queue, destroys
session keys/context, and does not reconnect.

TransportConnected means authentication and BIND completed. ReceiverRunning
means the controlled playback endpoint reports successful start. Streaming
means the endpoint recently received valid session audio, successfully wrote
PCM, and observed AudioTrack's playback head advance within its activity window.
This receiver activity state is not proof of audible output. Waiting for PCM or
a paused session stays on the live control path as ReceiverRunning or paused;
it is not an audio-session failure. Only an authenticated HEARTBEAT can refresh
receiver liveness or return a newer playback state. A heartbeat deadline is
checked from local monotonic time.
Unknown commands, invalid control state transitions, bad control identity or
authentication, replay, and sequence failures terminate or reject the control
operation and never claim that the Receiver is running. Invalid UDP datagrams
are dropped by the existing parser; they cannot commit replay state or enter
the PCM queue.

P5-C provides the authenticated session used by the explicit P5-F backend. The
local cross-language test uses synthetic PCM and a fake playback endpoint;
real USB/LAN/hotspot transport, Android runtime, WASAPI/AudioTrack output, and
device behavior remain NOT_TESTED.

## P5-D local single-session recovery owner

`p5c::SessionOwner::start` is the explicit localhost check path. The
`start_on_interface` path receives a caller-selected private IPv4 interface,
literal receiver endpoint, trusted `BindingMaterial`, PCM parameters, and a
caller `CancellationToken`. Both start one dedicated standard thread to own
all synchronous session I/O; neither blocks Tauri or PCM callback threads.
Tokio's cancellable connect is converted to a standard TCP socket and switched
to blocking mode before synchronous I/O. A 50 ms socket timeout bounds each
read/write poll while the absolute operation deadline and cancellation checks
remain in force.
The owner reuses the supplied binding by making and wiping one private copy per
fresh P5-A handshake. Every recovery derives a new session ID and directional
keys and creates a new PCM queue; it never provisions, overwrites, or clears
the caller's binding.

While active, the owner sends authenticated heartbeats every second. A failed
heartbeat closes the old sender, UDP/TCP sockets, queue, and session keys,
publishes `Reconnecting`, and waits on the bounded 1/2/5/10 second schedule
(10 seconds maximum) before trying the currently supplied endpoint again.
`update_endpoint` is the loopback test path; `update_network_endpoint` explicitly
replaces both selected interface and literal receiver endpoint. Either change
closes the old session and requires a fresh P5-A handshake. Address guessing is
not implemented. The latest requested pause state is tracked
separately from the last ACK-confirmed receiver state and is reapplied after
the next authenticated START. A failed PAUSE/RESUME does not claim the requested
state was applied. `SessionSnapshot` distinguishes
authenticated, transport-connected, receiver-running, streaming, paused,
reconnecting, and stopped; streaming still comes only from the authenticated
receiver's recent receive/write/playback-head activity reflected by a heartbeat
ACK.

Explicit owner stop disables recovery and asks the owner thread to send the
authenticated STOP when it is between I/O operations. If that does not finish
within its short 100 ms stop window, it cancels the pending operation and
closes locally. Caller cancellation directly interrupts a pending
connect/read/write/backoff poll. Both paths clear the queue and close the
control socket. The Android service keeps its listener available for a bounded
30-second recovery window after disconnect, then stops when recovery expires.
The product owner reauthenticates with a fresh session and the selected binding
during that window. Local recovery checks use a rebuilt fake localhost endpoint;
they do not establish recovery after USB/bootstrap loss, start an unavailable
Android media service, or verify real audio. Device, LAN, WASAPI and AudioTrack
recovery remain NOT_TESTED.

## P4-B / P5-E explicit private IPv4 path and discovery candidate

Windows `network_interface::enumerate_private_ipv4` reads enabled, non-loopback
IPv4 adapters through the existing Windows API dependency. It returns private
unicast addresses with prefix, interface index/name, and IPv4 gateway values;
the caller must select one entry explicitly. Runtime interface names and
addresses are not persisted or logged. The direct session binds both its
cancellable TCP connect and P5 authenticated UDP socket to that selected local
address. Its literal IPv4 peer must be private; explicitly binding the local
socket keeps route selection on the caller-selected interface.
The authenticated BIND sends the exact host UDP port; the receiver binds its
UDP socket to its selected local address and fixes the expected peer to the
accepted TCP source address plus that port. The authenticated BIND ACK supplies
the receiver UDP port, and the host sends the existing P3 encrypted PCM packets
to the TCP peer address plus that port. Queue format, P5-A-derived audio key,
session ID, and UDP sender all come from the same authenticated session.

An explicit discovery call sends at most two UDP probes on that interface: the
first actual gateway reported for that interface, then its directed broadcast
(when the prefix supports one). There is no subnet scan, mDNS dependency, default
probe, listener, or automatic hotspot/firewall change. The Receiver's internal
`openOnInterface` accepts one explicitly supplied private IPv4 address and
prefix for its P5-C listener. Optional discovery binds only the fixed discovery
receive port to wildcard for broadcast reception, accepts probes only from the
selected address prefix, and sends each response through a socket bound to the
selected address. Control and audio sockets remain bound to that explicit
address. Closing or replacing the interface/session closes the responder and
old authenticated session; the caller must supply the replacement endpoint for
a fresh handshake.

Discovery uses one fixed 26-byte UDP record on port 49772:

| Offset | Bytes | Field |
| --- | ---: | --- |
| 0 | 4 | ASCII `ANPD` magic |
| 4 | 1 | Version, `1` |
| 5 | 1 | Type: probe `1`, candidate response `2` |
| 6 | 2 | Reserved flags, zero |
| 8 | 16 | Fresh random probe correlation token |
| 24 | 2 | TCP control port, big-endian; zero in a probe, nonzero in a response |

The receiver replies only to a correctly sized, versioned probe with zero flags
and a zero port. It copies the correlation token and advertises its already
bound temporary TCP port. Discovery receives on the fixed wildcard UDP port;
responses originate from a separate socket bound to the selected private IPv4
address. The host accepts a response only when the token, record shape, nonzero
UDP source port, and source IPv4 prefix match the current probe and selected
interface. A response is only an unauthenticated endpoint
candidate: it carries no key, serial, or persistent identity and cannot bind a
UDP peer or start playback. The caller must continue with the existing P5-A
mutual authentication and P5-C BIND/START. A caller can instead supply a
literal private IPv4 endpoint directly without discovery; it still must pass
the same authentication before playback.

Discovery is cancellable, polls at 50 ms, sends to no more than those two
targets, has a maximum two-second window, retains at most 16 unique candidates,
and stops receiving once that cap is reached. An empty result reports that no
Receiver answered and says it may be unreachable or stopped and require an
authorized USB/bootstrap path; the result does not infer ForceStop or firewall
state. Interface bind errors, unreachable endpoints, and P5-A identity/proof
failure have separate diagnostics. None of these paths claim `phoneReady`.
Discovery and the direct session are available through explicit Tauri backend
commands and the existing product session state. No UI caller, persisted
network choice, default listener, or automatic startup was added. Local checks exercise
the wire exchange and full authenticated PCM handoff only on loopback with a
fake sink; real LAN/hotspot broadcast, Android runtime, ADB, WASAPI, AudioTrack,
and audible output remain NOT_TESTED.

## P5-F explicit product backend

The desktop backend exposes explicit Tauri commands and routes their work
through the existing session state; there is no UI caller or default network
listener. The selected output and balanced/low-latency mode flow through the
existing session state. Computer output uses the existing Legacy path; phone
output requires an explicitly selected private IPv4 interface and receiver
binding reference. Native initialization exposes its negotiated stereo PCM
rate through the optional `AudioCapture_GetPcmFormat` C ABI before the
authenticated handshake. The handshake uses that rate, and the capture sink
compares the metadata again before starting capture or muting computer output;
it does not wait for a PCM packet. An older DLL without the metadata export
reports an explicit unsupported-capability error while its Legacy exports
remain usable. USB bootstrap
is allowed only after checking Receiver network protocol version 1. The host
then queries the Receiver's actual bound address and nonzero control port; it
does not guess an address or port. Discovery candidates remain unauthenticated
until P5-A mutual authentication succeeds.

Saved bindings are isolated by USB serial and Android user. The UI-facing list
contains opaque SHA-256 references only; binding bytes remain in the existing
DPAPI store and Android app-private store. A selected reference is rechecked
against the current USB identity/user before provisioning or connecting.
Changing playback mode or returning from computer output closes the old
authenticated session and establishes a fresh one with a fresh PCM queue while
preserving pause intent. Recovered frame zero therefore belongs to a new
session and queue. A pause received after START but before UDP queue creation is
recorded on its owned audio session and applied atomically when that queue is
attached. Live bitrate uses successful PCM payload byte deltas from a shared
sender counter that survives sender teardown, including the final unsampled
tail. Native capture replacement joins the previous capture before releasing
callback state. Capture or format failure follows local cleanup and restores
computer output.

An unexpected Receiver audio-worker failure and a rejected or timed-out START
begin a bounded cleanup/recovery window immediately; a stale owned playback
session fails its next heartbeat instead of being reported as merely idle. A
healthy idle stream with no PCM packets stays ReceiverRunning. The Receiver
keeps its listener for at most 30 seconds after disconnect so the host can
recover with a fresh authenticated session. This does not cover loss of USB
provisioning or an unavailable service. The only current verification for
this backend is local compilation and synthetic localhost coverage; real
USB bootstrap, Android runtime, LAN/hotspot, WASAPI capture, AudioTrack output,
audible output, and recovery on devices remain NOT_TESTED. No APK was assembled
or installed during this work.

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
latency bounded. The P3 mock sender remains loopback-only; the authenticated
P5 sender binds to the explicitly selected private IPv4 address and literal
peer. Android uses a separate receive thread to authenticate and enqueue
packets; a queue-backed `InputStream` supplies the negotiated format header and
PCM bytes to the existing `Pcm16FrameStream` / single AudioTrack playback
worker. Its eight-packet queue sorts by unsigned sample frame index and drops
old history on overflow or pause. The pause watermark advances through every
discarded packet, and the stream adapter drops any unread bytes from a packet
it had already taken when the pause boundary changes. No UDP control packet or
default listener is implemented. This does not claim one-way latency or
real-time performance.

The native capture handoff is bounded by frames and local monotonic age: at
most 20 ms in low-latency mode or 40 ms in balanced mode. If one capture block
is larger than that frame budget, it keeps the newest tail and advances its
sample frame index by the discarded prefix. Older queued blocks are evicted
first, and the sender checks age again before each datagram; discarded frames
are included in the queue counter. Android's network jitter reserve starts at
5 ms / 15 ms and may grow on AudioTrack underruns up to 20 ms / 40 ms. That
reserve is independent of AudioTrack's own buffer and startup pre-roll; these
values are tuning bounds, not an end-to-end latency budget.

AudioTrack is configured independently: the balanced mode requests a 50 ms
effective application buffer where `setBufferSizeInFrames` is available;
low-latency keeps its 15 ms request and 5 ms API 31+ start-threshold request.
Each request is bounded by the track's reported capacity. Diagnostics preserve
the requested, accepted, effective, capacity, start-threshold, and actual
pre-roll frame counts separately; on API 23 the system buffer remains in use.
The pending-frame diagnostic includes pre-roll writes and subtracts the
unsigned, wrap-aware playback head. It describes queued PCM frames, not acoustic
output latency.

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
