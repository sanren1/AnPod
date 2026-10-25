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
and is never replaced. Reset is a separate, explicitly confirmed USB operation
described below; provisioning never silently clears or replaces a record.

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
between devices or Android users. It does not silently unbind or re-pair.
Real USB, DUMP permission enforcement, and Android private storage remain
NOT_TESTED for the new reset path.

### USB conditional binding reset

`receiver_binding_reset_preview` accepts an authorized USB serial and returns
the current Android user, its derived PC binding reference, each side's binding
state and commitment, and whether the Receiver is busy. It verifies the installed
Receiver identity and queries the exact one-column `bindingResetVersion`
projection on the existing `/binding` URI; version 1 advertises this operation.
An older Receiver is rejected before session teardown or deletion. The existing
three-column binding query is unchanged.

Only a separate user confirmation calls `receiver_reset_binding` with that
preview. The backend rechecks USB identity, Android user, binding states and
commitments before stopping a matching active session. Pending/failed binding
writes and a different active target are rejected before teardown. Android
deletion requires the fixed binding URI, DUMP permission, the adb shell UID,
null selection arguments, and either `expectedEmpty` or
`expectedCommitment=<64 lowercase hexadecimal digits>` as its exact selection.
It rejects active reception/playback and pending writes. The idle check and
conditional deletion share the binding storage lock with reception startup's
state change and binding load, so startup cannot retain a record deleted by
a competing reset.

The phone is conditionally cleared and read back as empty before the PC removes
only the selected serial/user reference's DPAPI file with the expected
commitment. The operation does not promise an atomic transaction across USB.
Its structured result distinguishes complete reset/already-empty from retained,
changed, or unknown states on either side and includes a reason and next action.
Unless both sides still report their original saved commitments, the PC closes
an offer listener whose automatic binding matches this reset reference. Closing
it cancels its pending connection attempt and invalidates its queued offers;
listeners for another binding and preflight failures before deletion are retained.
Cancellation or an uncertain phone result preserves the PC copy where possible;
a shell process exit code alone never proves deletion. Reset never provisions
a replacement binding or starts playback, and no LAN reset command is defined.
Fake ADB and local storage tests cover backend outcomes; Android permission,
AtomicFile, and real-device reset behavior remain NOT_TESTED.

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

The Receiver bounds HELLO through READY by one two-second monotonic deadline,
including blocked reads and writes. Pre-authentication admits at most two
connections globally and one per source address; admission may close an older
pending connection to keep these bounds. Discovery remains available during
unauthenticated attempts. Only a successfully authenticated connection can
atomically claim the single control/playback owner and stop discovery; another
pending authentication cannot replace an active owner.

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

## Receiver management negotiation (V3)

The P5-C authenticated outer frame remains version 1: a two-byte length prefix,
the fixed 30-byte header, payload, and 32-byte HMAC. Existing ACK type 127 keeps
its exact 13-byte payload. The V3 capability response is a separate fixed
frame; no implementation may append fields to an old ACK.

After the P5-A binding handshake has authenticated the session, but before
BIND/START, PCM, or any new management action, the host may send
`CAPABILITIES_QUERY` (type 8) with this fixed 8-byte payload:

| Offset | Width | Field |
|---:|---:|---|
| 0 | 2 | requested management control major, unsigned big-endian (`3` for V3) |
| 2 | 1 | requested audio protocol major (`1` for PCM16 v1) |
| 3 | 1 | reserved, must be zero |
| 4 | 4 | required capability bit mask, unsigned big-endian |

A V3 receiver replies to an authenticated query with `CAPABILITIES_INFO`
(type 9), whose fixed 20-byte payload reports whether the requested versions
and capabilities are compatible:

| Offset | Width | Field |
|---:|---:|---|
| 0 | 8 | echoed host query sequence, unsigned big-endian |
| 8 | 2 | supported management control major, unsigned big-endian |
| 10 | 1 | supported audio protocol major |
| 11 | 1 | result: `0` compatible, `1` incompatible |
| 12 | 4 | implemented capability bit mask, unsigned big-endian |
| 16 | 4 | `versionCode`, unsigned big-endian; informational only |

Both frames use the existing direction, session ID, sequence, and HMAC checks.
Capability bits are:

| Bit mask | Capability |
|---:|---|
| `0x00000001` | `receiver-ui` |
| `0x00000002` | `authenticated-status` |
| `0x00000008` | `playback-progress` |
| `0x00000020` | `background-start-authorization`, only for explicitly enabled standby endpoints |

The former capability bit `0x00000004` and command types 10–15 for network APK
updates are retired. They are not advertised, accepted, or sent, and these
numeric identifiers remain reserved rather than being reassigned to other
features.

A receiver reports only implemented bits; unknown bits do not satisfy a
required bit. `versionCode` and the capability response are not
APK identity evidence. The host must separately establish identity from its
existing binding authentication or an exact locally audited APK digest. It
must not trust a remote package name, certificate string, or version claim as
identity.

The host proceeds with audio only after an authenticated response with result
`0`, matching required control/audio majors, and every required capability bit.
Network APK upload and self-update are not part of P5-C. The host keeps APK identity and installation on the explicitly selected USB/ADB path; network sessions are limited to authenticated management and audio operations.

An old receiver that does not recognize type 8 may reject or close the session.
The V3 host ends that session and does not reinterpret an old ACK or issue V3
commands. A documented legacy path is allowed only when the installed APK has
already been proven against the exact local target or an explicit audited
historical digest/version/source record. Otherwise the user must use the
existing authorized ADB or manual same-signature installation path. The first
v2-to-v3 installation is not a network update.

The current V3 source configuration uses Android `versionCode` 9 and the
existing `0.7.0-v3-flow-candidate` version name; no final APK 9 artifact has
been generated. APK identity still comes from locally verified artifact
metadata, not the capability response.

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

If START fails before playback ownership is established, the Receiver clears
the pending UDP context and mode selection and returns a failed ACK in
authenticated state (0). A later attempt must bind and select the mode again;
the host recovery path creates a fresh authenticated session.

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

While active, the owner sends authenticated heartbeats every second and checks
whether its UDP sender worker has exited unexpectedly, including while paused.
A failed heartbeat or sender closes the old sender, UDP/TCP sockets, queue, and session keys,
publishes `Reconnecting`, and waits on the bounded 1/2/5/10 second schedule
(10 seconds maximum) before trying the currently supplied endpoint again.
For a previously active session, the 30-second recovery window starts when the
connection is lost, rather than when playback originally started. The desktop
snapshot exposes `networkRecoverySecondsRemaining` only while that recovery
window is active; initial connection attempts and healthy sessions expose null.
Transient UDP sends allow at most three attempts, each with a fresh packet
sequence and GCM nonce; a fatal or exhausted send is reported to the owner.
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

## P5-C authenticated discovery (V3 records)

Discovery v1 (`PROBE`/`CANDIDATE`) and the V3 records above stay
unauthenticated: their replies carry only an endpoint, so a host cannot tell
which saved binding answered, and any host in the prefix can fabricate one.
Two additional records bind a reply to the USB-provisioned binding secret:

| Record | Version/type | Bytes | Payload |
| --- | --- | ---: | --- |
| `AUTH_PROBE` | 3 / 6 | 50 | zero flags, fresh 16-byte nonce, 8-byte binding tag, 16-byte MAC |
| `AUTH_CANDIDATE` | 3 / 7 | 50 | zero flags, echoed nonce, Receiver TCP control port, tag, MAC |

All integers are big-endian. The tag is `HMAC-SHA256(secret,
"anpod-discovery-tag-v3")[0..8]`; it identifies the binding without revealing
its identifiers. The probe MAC is `HMAC-SHA256(secret, "anpod-probe-v3" ||
nonce || tag)[0..16]`; the reply MAC is `HMAC-SHA256(secret,
"anpod-candidate-v3" || nonce || tag || controlPort)[0..16]`. The distinct
labels keep these values separate from every other use of the binding secret.

The host sends one `AUTH_PROBE` per saved binding alongside each v1 probe and
records which binding produced a verified `AUTH_CANDIDATE`. A Receiver replies
only when it holds a binding whose tag and probe MAC verify, at most once per
200 ms, and after a random delay of up to 20 ms. Replies are dropped rather
than queued, so a flood cannot delay audio work. A host that receives no
authenticated reply still sees the v1 candidate, which the interface marks as
unverified; authenticated candidates are the only ones a future presence
tracker may trust. Verification never replaces P5-A: a verified candidate is
still an unauthenticated endpoint hint until the session handshake succeeds.

The shared vector (`secret = 00..1f`, `nonce = 00..0f`, control port `41234`)
is asserted by both `src-tauri/src/discovery.rs` and the local Java check: tag
`26414e36e4dc63e6`, probe MAC
`83ec229f58b0088dd60051cb736e6db8`, reply MAC
`a82f6568b5e977d37c46b22155edcba4`.



## P5-C V3 bidirectional discovery and host offers

Discovery v1 remains unchanged for host-initiated discovery. A second,
unauthenticated offer channel adds three records on the same UDP port
(`49772`, version 2), sent only over the currently selected private IPv4
interface:

| Record | Version/type | Bytes | Payload |
| --- | --- | ---: | --- |
| `HOST_PROBE` | 2 / 3 | 24 | zero flags, fresh 16-byte nonce |
| `HOST_CANDIDATE` | 2 / 4 | 26 | zero flags, echoed nonce, PC UDP offer port (`49772`) |
| `CONNECT_OFFER` | 2 / 5 | 26 | zero flags, echoed nonce, Receiver TCP control port |

All integers are big-endian; the 2-byte flags field is zero. A Receiver search
is user-started, expires after ten seconds, and can be cancelled. Its discovery
socket binds port 49772 on the selected Android `Network` and receives on the
wildcard address so directed broadcasts reach it; the advertised TCP control
listener remains bound to the selected local address. The Receiver only sends
`HOST_PROBE` and accepts PC `HOST_CANDIDATE` replies; it does not answer
`HOST_PROBE` as though it were a PC. The Receiver accepts at
most 16 unique candidates whose source is in its selected interface prefix,
whose record advertises UDP offer port 49772, and whose nonce matches the active
search. The PC's explicit wireless-UDP offer listener binds port 49772 on the
wildcard address to receive LAN broadcasts, filters packets to the selected
private IPv4 prefix and source port, and sends candidate replies from a socket
bound to the selected local address. A selected candidate can send one
`CONNECT_OFFER` to the PC's fixed discovery port. The PC accepts an offer only
from that interface prefix and source port 49772, correlated to a fresh probe
and unexpired nonce. Offers expire thirty seconds after receipt of their
`CONNECT_OFFER`; expiry is enforced when snapshots are read and when an offer is
accepted. The automatic phone intent still expires ten seconds after search
starts, so a later offer cannot start automatic playback. Loss of the selected Android interface closes its discovery socket
and clears actionable candidates.

An offer is only a pending, unauthenticated endpoint hint. It cannot change
the current binding or establish identity. An explicitly enabled PC listener
may use the sole saved binding, or a binding explicitly selected when enabling
the listener, to authenticate one unambiguous pending target automatically.
Several pending targets or unresolved binding choices retain explicit selection.
Manual acceptance reserves the same non-preempting operation slot as automatic
acceptance before consuming an offer, so neither can replace an in-progress USB
management operation. A failed preflight releases its reservation.
An automatic caller requires the connection-intent capability described below;
an explicit accept can still connect to older Receivers. Both perform P5-A plus
the authenticated capability query before BIND/START. A spoofed, replayed, expired, or otherwise
uncorrelated offer is ignored. The offer listener requires saved transport
`Wifi` and wireless protocol `udp`; changing either setting or the selected
interface closes it. Activity rebinds reuse the existing Receiver service and
discovery socket. V3 search uses that same socket and creates no pairing or
audio endpoint.

### Authenticated discovery (V3, types 6–7)

Version 3 adds two records on the same UDP port so a host can recognise its own
bindings without a TCP handshake. Both are 50 bytes: `ANPD` (4), version 3 (1),
type 6 `AUTH_PROBE` or 7 `AUTH_CANDIDATE` (1), zero flags (2), a fresh 16-byte
nonce, the Receiver control port (2, big-endian), an 8-byte binding tag, and a
16-byte MAC.

The tag is `HMAC-SHA256(binding secret, "anpod-discovery-tag-v3")` truncated to
eight bytes, so it is stable for one binding. The probe MAC covers
`"anpod-probe-v3" || nonce || tag`; the candidate MAC covers
`"anpod-candidate-v3" || nonce || tag || port`. Both are truncated to sixteen
bytes. A Receiver compares the tag and probe MAC in constant time before it
answers.

The host sends one `AUTH_PROBE` per saved binding alongside the v1 probe on
every scheduled step. A Receiver answers with `AUTH_CANDIDATE` only when both
checks pass, at most once per 200 ms, with 0–19 ms of random delay before
sending. The host accepts a reply only when the nonce matches its current probe
and the candidate MAC verifies. A verified reply carries the binding reference
and upgrades an endpoint that was already seen through the unauthenticated v1
record; it is also the only input to binding presence, so an unauthenticated
answer can never make a binding look online.

Two consequences are deliberate:

- The v1 and v2 records stay unauthenticated for compatibility. Any device can
  still answer v1 and appear as an unpaired candidate, but it cannot impersonate
  a binding because the candidate MAC cannot be produced without the secret.
- The tag is a stable per-binding identifier on the local link. A passive
  observer can correlate probes from the same host across sessions; no key
  material is exposed.

### One-click reception intent (U5)

The phone's explicit start action uses the same ten-second search and sends
one CONNECT_OFFER automatically to each bounded valid PC candidate. None of
those candidates is trusted until the saved P5-A binding authenticates it.
Automatic mode advertises capability `connection-intent` (`0x00000010`).
After CAPABILITIES_INFO and before BIND, the host sends HMAC-protected
CONNECTION_INTENT (type 17) containing exactly the 16 discovery-token bytes.
An OK ACK retains Authenticated state. Wrong, absent, expired or cancelled
intents reject BIND/START. Types 10–15 and the retired network-update bit stay
reserved; P5-A and encrypted audio layouts are unchanged.

Cancellation clears the pending intent and closes any pending TCP connection.
START rechecks the intent in the service's actual main-thread handoff as well
as the coordinator. Interface loss invalidates it. Once playback is established,
the same intent identifier can identify that operation during the existing
bounded recovery window; it is not a cryptographic nonce. Every recovery still
performs a new P5-A handshake with fresh cryptographic nonces, session and keys.
Explicit stop closes the operation; Activity rebind never starts a new search.
PC listener stop, replacement, mode change or interface loss cancels its
pending operation; automatic attempts never preempt an active session or a
foreground operation. The listener remains alive through failed authentication
so consumed discovery tokens stay single-use. Failed initial automatic attempts
do not enter indefinite reconnect loops. No binding is created or changed.

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
worker. Its 32-packet queue sorts by unsigned sample frame index and drops
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
20 ms / 40 ms and may grow to 60 ms / 150 ms in low-latency / balanced mode.
Accepted, forward-moving audio packets provide relative arrival/frame-interval
jitter estimates; significant jitter or AudioTrack underruns can grow the
target quickly, while ten seconds of stability permits a 5 ms reduction.
The physical queue and age limits remain separate: 70 ms / 160 ms, with a
32-packet cap. These wireless tuning proposals are independent of the PC
capture queue, AudioTrack's own buffer and startup pre-roll; they are not
measured end-to-end latency or evidence of improved sound quality.

After the existing gap deadline, UDP loss concealment fills a sample-frame
gap only when it fits within the current jitter target. It repeats at most
one previously completed real PCM payload with a fade to zero, then fills
the remaining bounded gap with silence. The next real payload uses the
existing fade-in. Synthetic PCM never becomes the reference for another
concealment. A larger unsigned frame gap retains the discontinuity/skip
path rather than allocating or playing an unbounded silence buffer. With
no following packet, the queue waits instead of generating endless PCM.
Pause, discontinuity, expiry, stop, and failure invalidate the old reference.
Internal counters distinguish concealment events, repeated/silent frames,
and oversized gaps; these counters do not change the control wire format.
Synthetic mono/stereo checks do not establish audible quality improvements.

USB TCP backlog trimming uses a separate target and trigger: balanced mode
retains 10 ms after exceeding 30 ms; low-latency mode retains 5 ms after
exceeding 15 ms. Both the installed Receiver and Legacy AndroidServer use
these shared `Pcm16FrameStream` limits. Crossing the trigger causes one
frame-aligned discard of the oldest available PCM to the target, followed by
the existing fade-in. Below the trigger, PCM is retained. These network
backlog limits do not change AudioTrack's application buffer or pre-roll.

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

The Rust test `protocol::tests::java_jca_aes_gcm_vector` and the local Java
check use the same key, nonce, 34-byte AAD, and
payload. The Java entry also sends the fixed packet through the real Android
parser, then runs the merged parser/queue/loopback checks. Both vector paths
must produce this complete packet:

```text
414e50440101000001020304050607081112131415161718212223242526272800073bd32562c2e2cdfe093c22223a9705d1ae457629baad04
```

The vector is test data only and must not be used as a session key.

## Desktop transport selection (V2 backend contract)

The canonical selection is `transport = usb | wireless`. For a wireless
selection, persist `wirelessProtocol = udp | adb`; a new setting defaults to
`udp`. A saved `preferredTransport = Wifi` with no protocol field migrates to
`adb`, preserving the former Wi-Fi ADB behavior; explicit `udp` and `adb`
values are retained. A missing audio-backend setting also preserves the old
`ANPOD_AUDIO_BACKEND=installed-receiver` opt-in. The stored
`preferredTransport = Usb | Wifi` remains as a compatibility representation
for the current frontend; snapshots expose the canonical value as
`connectionTransport` and continue returning the old `transport = usb | wifi`
alias until the UI migration.

Use `update_setting` for an idle preference. For an active session, call
`reconfigure_session` with `kind = transport`, `wirelessProtocol`,
`audioBackend`, or `playback`; the backend validates the requested value before starting an
operation or cleaning up the current owner. It stops the old route, restores
computer output, removes owned resources, then reconnects only when the same
device and selected route are known. Missing wireless ADB devices or UDP
interface/binding/peer data produce a ready state with computer output and a
selection prompt. UDP never guesses a peer and never falls back to ADB. Device
monitoring and auto-reconnect follow the saved wireless protocol, while the
explicit `connect_device*` commands remain explicit ADB operations.

`connect_device`, `connect_device_only`, and `switch_device` are explicit
ADB-device operations. A wireless ADB serial always stays on ADB even when the
stored wireless protocol is UDP; UDP is selected explicitly through the
existing `start_network_audio` command. Its active session reports
`sessionConnectionTransport = wireless` and
`sessionWirelessProtocol = udp`; a Wi-Fi ADB session reports
`wireless` / `adb`. USB reports `usb` with no wireless protocol.

The Receiver audio backend is selected separately with
`audioBackend = receiver | legacy`; Legacy is not a wireless protocol.
New settings and settings without an audio backend default to Receiver.
An explicitly saved Receiver or Legacy choice remains unchanged. Receiver
routes both USB and wireless ADB through the installed APK's existing ADB
reverse and LocalSocket scheduling path.
Failures stop and clean that route; the host does not retry by silently
starting Legacy.

The protected `BootstrapActivity` STOP action sends an explicit STOP command
to the private `ReceiverService`. The service routes it through the current
owner/generation termination path, which releases AudioTrack, network
coordinator, and foreground state even while one or more UI clients remain
bound. A duplicate STOP waits for the existing cleanup. Device-side Binder and
foreground-service runtime behavior remains unverified.

## P5-C authenticated playback progress (V5)

After capability negotiation, a host that received `playback-progress`
(`0x00000008`) may send `PLAYBACK_PROGRESS_QUERY` (type 16) with an empty
payload. It is sent alongside the existing authenticated heartbeat, at the
host's one-second status-query cadence. The Receiver replies using ACK type
127 and the existing 13-byte ACK payload followed by exactly 70 bytes of
playback diagnostics (83 bytes total). The 13-byte ACK itself is unchanged.
An older Receiver that does not advertise the capability receives no type 16
query.

The 70-byte extension uses unsigned big-endian integers. `0xff`, `0xffffffff`,
and `0xffffffffffffffff` mean unknown for 1-, 4-, and 8-byte fields
respectively. Queue, pending, buffer, and drop values are PCM frames; sample
rate is Hz, channels are a count, and underruns are a cumulative AudioTrack
counter. The host computes `underrunIncrement` from successive cumulative
counts within the same authenticated session; the first sample or a counter
reset has an unknown increment.

| Offset | Width | Field |
|---:|---:|---|
| 0 | 4 | Receiver sample rate, Hz |
| 4 | 1 | Receiver channel count |
| 5 | 4 | UDP jitter target, frames |
| 9 | 8 | Expired UDP frames, cumulative |
| 17 | 8 | UDP queue depth, frames |
| 25 | 8 | AudioTrack pending frames |
| 33 | 4 | AudioTrack underrun count, cumulative |
| 37 | 8 | Receiver TCP backlog frames dropped, cumulative |
| 45 | 1 | Playback head advanced recently: `0` no, `1` yes |
| 46 | 4 | Requested AudioTrack buffer, frames |
| 50 | 4 | Accepted AudioTrack buffer, frames |
| 54 | 4 | Effective AudioTrack buffer, frames |
| 58 | 4 | AudioTrack capacity, frames |
| 62 | 4 | AudioTrack start threshold, frames |
| 66 | 4 | Startup pre-roll, frames |

The PC supplies its own capture sample rate/channels, PCM queue depth, and
dropped frames from its local session state. Neither frame counts nor recent
playback-head advancement are an acoustic-latency measurement, and this query
does not compare remote and local clocks.

## Background-start authorization (backend integration)

An explicitly enabled standby endpoint advertises `background-start-authorization`
(`0x00000020`). Existing binding authentication alone does not authorize background
playback. After P5-A and capability negotiation, before BIND/START, the host sends
BACKGROUND_START_REQUEST (18), polls BACKGROUND_START_STATUS (19), or sends
BACKGROUND_START_CANCEL (20). Each carries exactly one 16-byte request ID. Their
authenticated ACK payload is exactly 30 bytes: the existing 13-byte ACK, echoed
request ID (16), and authorization status (1). Existing ACK formats remain unchanged.
Status values are pending (0), once (1), eight hours (2), until revoked (3), denied
(4), expired (5), and cancelled (6). The host verifies the echoed ID and status.

The host waits at most 30 seconds, polls at 500ms intervals, and continues one-second
heartbeats. Each request/status exchange has a three-second deadline bounded by
the overall deadline, covering both reads and writes. The Receiver checks authority
at BIND and START and again at playback handoff. Missing capability or authorization
fails closed for background-required endpoints; the existing explicit manual path
keeps its behavior. The P5-A two-second handshake deadline is unchanged.

Once authorization belongs to the current owner generation, request, session, peer,
and binding. Recovery retains a fixed 30-second episode deadline; STOP clears once
authorization. Eight-hour trust uses elapsed time and lasts only in the current
process. Until-revoked trust is stored separately against the binding commitment;
old bindings do not acquire it automatically. Notification decisions carry the full
64-bit session ID and generation-bound request token. Permanent trust requires an
explicit UI action. In background mode PC STOP ends playback and leaves standby
available for a new authenticated session; phone stop, revocation, expiry, or selected
interface loss ends standby. Standby uses connectedDevice foreground service type;
active background playback also uses mediaPlayback.

Standby must be explicitly enabled from the Receiver's foreground UI. A stored
until-revoked grant survives restart, but does not itself restart the listener or
authorize a background-service launch at boot. The current candidate does not
implement automatic boot/process-restart listener restoration.

The type 18/19/20 ACK reports the request's authorization status, not its remaining
lifetime. Device notification, foreground service, persistence, and playback
behavior require device acceptance; offline state/loopback checks do not establish
that these platform paths work on a phone.

### Background trust management

An endpoint implementing binding-level authorization management advertises
`background-trust-management` (`0x00000040`). After P5-A and capability negotiation,
the host may query with `BACKGROUND_TRUST_QUERY` (21), or revoke with
`BACKGROUND_TRUST_REVOKE` (22). Neither operation starts audio. They are valid in
Authenticated, TransportConnected, ReceiverRunning, Streaming, and Paused states.
The request contains exactly the authenticated binding's full 32-byte commitment;
discovery data or possession of an endpoint address is not authorization.

The ACK contains the usual 13 bytes followed by exactly 42 bytes (55 total):

| Extension offset | Width | Field |
|---:|---:|---|
| 0 | 32 | Authenticated binding commitment, echoed and verified |
| 32 | 1 | Grant kind: none (0), once (1), eight hours (2), until revoked (3) |
| 33 | 1 | Persistence: none (0), process (1), binding (2) |
| 34 | 8 | Remaining milliseconds, unsigned big-endian |

Valid combinations are `(0, 0, 0)`, `(1, 1, 0..30000 or MAX)`,
`(2, 1, 0..28800000)`, and `(3, 2, MAX)`. `MAX` is `0xffffffffffffffff`
(Java `-1`), meaning no determined expiry, not a numeric duration for the UI.
For once, a finite duration describes only the fixed recovery window; active
once authorization has no predetermined expiry. The UI represents MAX as null
and does not derive an authoritative grant from a cached PC countdown.

A successful query preserves playback state. Successful revocation clears this
binding's playback authorization without deleting the pairing, stops its audio
and UDP context, and returns a Stopped ACK with `(0, 0, 0)`. The Receiver writes
the ACK before closing the control listener. Failed persistence changes must
not be reported as successful revocation. Type 20 still cancels one playback
request; it does not revoke binding-level trust.

## Explicit mobile receive intent

The normal mobile Start Receiving action keeps a service-owned receive intent on
the explicitly selected interface. It uses the existing manual connection-intent
authentication; it does not enable background-start authorization or grant lasting
trust. Discovery rounds remain bounded to ten seconds. A one-second maintenance
check starts a fresh idle round or renews an expired round while the intent is
enabled; it does not restart a search explicitly stopped or invalidated by interface
loss, and it does not replace an authenticated connection.

The UI preparation deadline and the previous session's recovery deadline do not
end this explicit receive intent. PC STOP ends the audio session and returns the
phone to waiting. Phone Stop cancels the intent and its owner generation; selected
interface loss or unavailable notification permission ends it with an error. The
waiting foreground service uses connectedDevice, and active playback adds
mediaPlayback. This intent does not survive process termination or imply boot start.

Playback-head advancement is a nullable observation in the local UI snapshot.
Unknown, waiting for advancement, and recent advancement are distinct; no PCM
arrival is not itself a control-session disconnect. Advancement is not acoustic
proof. Long real-device silence and subsequent playback still require acceptance.
# ADB Receiver Float32 audio stream

ADB Standard and Legacy continue to use their existing PCM16 path. HiFi/Hi-Res
over USB or wireless ADB use the authenticated loopback stream below; the PC
binds only `127.0.0.1`, and `adb reverse localabstract:<socketName> tcp:<port>`
maps the Receiver's abstract LocalSocket to that listener. The Receiver starts
through the DUMP-protected `START_ADB_STREAM_AUDIO` bootstrap action with only
`socketName`, the public 32-character lowercase-hex `hostId`, and
`connectionTransport` (`usb` or `wifi`). Format is selected only after P5-A
authentication.

The stream reuses ANPU records, ANP5 v2, P5-C and ANPD v2. ADB derives a
transport-specific binding secret: `salt = SHA256("AnPod ADB Stream binding
salt v1" || NUL || hostId16 || receiverId16)`, then
`HKDF-SHA256(savedSecret32, salt, "AnPod ADB Stream P5-A binding secret v1")`
to 32 bytes. This domain is distinct from the AOA Direct derivation. The host
requires capability mask `0x1582` (authenticated status, terminal cause,
ANPD v2, Float32 and ADB stream); `0x200` remains reserved for AOA session
restart. HiFi is profile 4 (48 kHz stereo Float32LE); Hi-Res is
profile 5 (96 kHz stereo Float32LE). ANPU record bodies remain at most 1200
bytes and each Float32 stereo ANPD payload carries at most 143 frames.

**Test-only independent binding vector**: host ID `01` repeated 16 times,
receiver ID `02` repeated 16 times, and saved secret `03` repeated 32 times
produce salt `d896877f373dc06ee5de7f1040620ec859e526d3c0d218c22ee0f8f0380e89d9`
and ADB binding secret
`cb76cb157e4c5446d03232534f61486a750cb61ff29ccf8882b09c3ffe1d0c37`.

# Playback progress schema 3

P5-C capability `0x2000` advertises buffer metrics. A host sends playback
progress query payload `[3]` only when this capability is present; otherwise it
uses the existing `[2]` extension capability `0x800`, or an empty payload. The
schema 3 tail is 143 bytes: bytes 0–125 keep schema 2 layout with byte 0 set to
3; offset 126 is `u32 driftTargetFrames`, 130 is `u32 driftObservedFrames`, 134
is signed `i32 driftErrorFrames`, 138 is signed `i32 driftCorrectionPpm`, and
142 is `u8 driftResetReason`. Unsigned unknown is `0xffffffff`; signed unknown
is `INT32_MIN`; reset reason 0 is unknown, and bit flags 1/2/4/8 mean initial,
not-playing or pause, source/output discontinuity, and observation/source gap.
Positive ppm means delete frames; negative ppm means insert frames. Target and
observed queue frames exclude platform output pending frames.
