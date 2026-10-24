# AOA direct audio transport

This specification defines the accessory-mode byte-stream transport for an
AnPod host and Receiver pair. Phase 6a–6c source integration and offline gates
are complete: AOA uses an isolated binding secret, the existing P5-A v2
handshake, authenticated P5-C control, ANPD v2 audio, ANPU record framing,
platform I/O owners, and the product USB permission/state entrypoints. It does
not reuse the AOA 4096-byte echo probe, ADB TCP, or the LAN UDP context. The
current candidate also supports an explicit, confirmed USB mode switch from
the PC app for the supported iQOO 11 V2243A; the switch does not start audio.
The 1.10.1 crash-fix package and PC 1.10.2 control flow are historical
baselines. PC 1.10.2 added owner suppression around USB Direct configuration,
a separately confirmed option to stop the local ADB server before one fresh,
identity-checked WinUSB attempt, and latency-mode availability for connected
USB/ADB sessions. The 1.10.3 candidate flow and verification are recorded as a
previous candidate below. PC 1.10.4 review corrections and final offline
results are recorded at the end. No physical accessory transfer or audio
playback has been tested for the current candidate.

The Receiver must have the matching accessory-mode application installed. The
PC app can send AOA vendor requests 51/52/53 to the explicitly selected and
confirmed supported pre-switch interface. The user then grants
`UsbManager` permission in the foreground Receiver and explicitly starts the
PC connection with a selected accessory and saved binding. USB mode switching
does not grant accessory permission or start audio. A detach, permission
revocation, or user cancellation ends the current session.

## Historical PC 1.10.2 USB Direct control-flow baseline

Opening the USB Direct configuration flow establishes a temporary owner
suppression before showing the window. Closing the window releases this
temporary suppression. Before a switch attempt, the user must explicitly opt
in to stopping the local ADB server;
this affects every ADB debugging connection on the PC. After consent, the PC
rereads and validates the selected `VID_2D95&PID_6013&MI_02` interface, runs
`adb kill-server`, and makes one WinUSB switch attempt. At that point
suppression becomes sticky and remains after a failed attempt and window
closure. Failure is surfaced as an error; there is no fallback or automatic
retry. The window's temporary suppression and sticky suppression have separate
lifetimes. Closing the window always releases the temporary owner; once a
switch attempt begins, sticky suppression survives an AOA connection and its
later disconnection. Only an explicit normal USB/LAN connection or re-enabling
automatic connection clears sticky suppression. During the flow, the wireless
offer listener stays running but pauses accepting offers; the saved wireless
preference is retained.

## Binding and transport separation

The saved ANPB v1 record remains exactly 69 bytes. USB serial numbers,
VID/PID, manufacturer/model strings, and Windows PnP paths select a physical
connection only; none is pairing identity or trusted session input.

Before the handshake, each side derives an ephemeral AOA-only binding secret
from the selected ANPB record:

```text
salt = SHA256(
    ASCII("AnPod AOA Direct binding salt v1") || 0x00 ||
    hostId[16] || receiverId[16]
)
aoaBindingSecret = HKDF-SHA256(
    salt=salt,
    IKM=ANPB.sharedSecret[32],
    info=ASCII("AnPod AOA Direct P5-A binding secret v1"),
    length=32
)
```

The derived bytes exist only for this transport session and are wiped with its
other key material. They are passed to the existing P5-A v2 handshake without
changing its message layout, transcript, labels, or the stable LAN handshake.
Because LAN uses the original ANPB secret and AOA uses the derived secret, a
proof or packet key from one transport is invalid on the other. The selected
ANPB host and Receiver IDs remain the identities checked by the handshake.

Each AOA connection creates a fresh handshake nonce pair, session ID, keys,
packet sequence, frame cursor, queue, and cancellation generation. A normal
PAUSE/RESUME stays within that session and keeps monotonic P5-C and ANPD
sequences. STOP, detach, permission revocation, failed/cancelled connect, or a
format change ends the session; reconnecting creates fresh keys and a new
session ID rather than resetting a sequence under the old key.

## ANPU stream records

USB bulk pipes carry a byte stream. Each record has an eight-byte outer header
followed by one complete inner message:

| Offset | Size | Field | Value |
|---:|---:|---|---|
| 0 | 4 | Magic | ASCII `ANPU` |
| 4 | 1 | Version | `1` |
| 5 | 1 | Type | `1` handshake, `2` control, `3` audio |
| 6 | 2 | Body length | Unsigned big-endian, 1–1200 |

The body length excludes the outer header. A receiver rejects an unknown type,
version, zero/oversize length, invalid inner header, or message type that is
not valid in the current session state. It closes the session on malformed or
truncated input; there is no byte-scanning or resynchronization after an error.
The outer type and length are framing only. P5-C HMAC and ANPD AES-GCM remain
the authentication boundaries.

Type 1 contains one unchanged P5-A v2 message. The required order and body
lengths are HELLO 96, CHALLENGE 104, FINISH 40, READY 40. A side accepts only
the next expected handshake message and aborts on duplicates or a wrong phase.

Type 2 contains a raw P5-C v1 control frame, without the two-byte TCP length
prefix. The existing 30-byte ANPC header, payload, and 32-byte HMAC tag are
unchanged; the complete body must be 62–1200 bytes and its internal payload
length must exactly match. The AOA adapter may prepend the two-byte body length
in memory when calling an existing TCP-frame validator. No prefix is added to
the USB body itself.

Type 3 contains one complete ANPD v2 datagram, at most 1200 bytes. It retains
the 34-byte ANPD header, authenticated header fields, AES-GCM tag, session ID,
direction, sequence, sample-frame index, and 1150-byte maximum plaintext. The
first audio record is the existing 16-byte FORMAT payload with frame index 0.
ANPD v1 is not accepted on this transport.

The Rust `read_record`/`write_record` and Java `AnpuRecordCodec.read`/`write`
operations consume or produce exactly one bounded record. An error returned by
either codec is fatal to that stream session; after partial record input/output,
the caller must close the session and must not retry at the next byte or scan
for another `ANPU` header. Transport adapters absorb only transient no-data
conditions before returning bytes to the codec: EAGAIN/EINTR and USB zero-length
packets are handled inside bounded platform I/O/deadline handling, not treated
as record boundaries. Windows may use its bounded pipe timeout/abort policy;
Android uses interruptible channel close rather than polling.
Rust's `read_exact`/`write_all` retry `Interrupted`; therefore an adapter must
not use `ErrorKind::Interrupted` to report cancellation, and must return a fatal
error kind when the session has been cancelled.
Any non-transient I/O failure, cancellation, zero-progress result from the
codec's stream API, or EOF during a record terminates the session. On writes,
the adapter may continue a partial transfer at its current offset, but a failure
after any bytes have been emitted closes the stream; it never re-encodes or
restarts that record.

## Control and playback state

After P5-A v2 authentication, the host sends the existing P5-C capability
query on the new session and requires the existing `CAP_AUDIO_V2` bit. The
query does not claim connection intent. The Android foreground user action
authorizes one playback start for the current accessory session; this first
integration does not request or persist background-start trust, auto-start in
the background, or reuse the LAN background-authorization flow. The endpoint
advertises only capability bits that its AOA implementation actually
supports. It reuses the existing SELECT_MODE, START, PAUSE, RESUME, STOP,
heartbeat, progress query, and ACK encodings. A management-only session cannot
send audio.

P5-C BIND has a transport-specific payload only in this AOA context. LAN
continues to send its nonzero UDP port unchanged. AOA sends exactly two zero
bytes to mean “bind this authenticated stream”; the Receiver returns a zero
port in the successful BIND ACK. Zero is rejected on LAN, and a nonzero port is
rejected on AOA. The session transport kind is established by the opened
Windows/Android accessory handles, never inferred from the control payload.

The supported product progression is:

| Profile | Format | Initial AOA support |
|---|---|---|
| 1 | 48 kHz, stereo PCM16LE | First implementation stage; existing Standard capture/conversion contract |
| 2 | 48 kHz, stereo packed PCM24 | After profile 1 passes |
| 3 | 96 kHz, stereo packed PCM24 | After profile 2; exact-rate source required |

Each profile uses the authenticated P5-A v2 descriptor and ANPD v2 encoder and
decoder. Profile 1 retains the existing Standard 48 kHz PCM16 capture and
conversion contract; its descriptor describes the emitted stream. Profiles 2
and 3 require the actual Windows mix/capture rate to match 48 kHz or 96 kHz
respectively. A 48 kHz source is never relabeled or resampled as 96 kHz.
Unsupported high-precision rate or representation fails the requested profile
explicitly; it does not silently fall back to another profile or to legacy
USB/ADB PCM16.

ANPD payloads contain whole interleaved frames. Packetization stays within 1150
plaintext bytes: PCM16 stereo is at most 287 frames per record; packed PCM24
stereo is at most 191 frames. A 5 ms 48 kHz PCM24 block (240 frames) is split
into 191 + 49 frames; a 5 ms 96 kHz block (480 frames) is split into 191 + 191
+ 98 frames. Every part receives its own ANPD sequence and begins at the exact
next sample-frame index. The Receiver validates a complete packet before
advancing replay state or queueing samples.

## Bounded byte-stream I/O and teardown

There is one reader for USB IN and one writer for USB OUT. Android's AOA
accessory endpoint is a blocking character device: the AOSP `android_f_accessory`
file operations do not provide `poll`, and its read path waits for USB data
without honoring `O_NONBLOCK`. Therefore `fcntl(O_NONBLOCK)` plus `poll` is not
a cancellation contract for `/dev/usb_accessory`.

The Android AOA product path requires API 24 or later. It uses one standard
`FileChannel` for USB IN and one for USB OUT, each backed by a non-owning
`FileInputStream`/`FileOutputStream` over the session's
`ParcelFileDescriptor`. Closing a channel from the cleanup worker signals and
waits for its active native I/O threads; the supplied streams do not own or
close the PFD's descriptor. The I/O owner closes both channels on its cleanup
worker, waits for its in-flight record operations to quiesce, and only then
closes the PFD exactly once. The coordinator separately joins its reader and
writer workers before discarding or reusing session state. API 23 does not
provide this channel-level native-thread signaling for externally owned
descriptors, so USB Direct is unavailable there; do not silently switch that
request to ADB. The app's existing UDP/ADB paths and minimum SDK remain
unchanged.

Waiting for the first byte of a new record may wait for the PC to send; the
three-second absolute AOA record-completion deadline starts only after its
first byte. Startup/owner and authenticated heartbeat deadlines remain
separate.

When a record deadline expires, schedule channel close on the cleanup worker;
never close a channel or join a worker on the main thread. Use a fixed 16 KiB
transfer buffer for record I/O; it is protocol buffering only and does not add
audio pre-roll.

The version boundary is based on the tagged AOSP implementations:
[Android 6.0.1 `FileChannelImpl`](https://android.googlesource.com/platform/libcore/+/refs/tags/android-6.0.1_r81/luni/src/main/java/java/nio/FileChannelImpl.java)
only closes its parent stream, while [Android 7.0 `FileChannelImpl`](https://android.googlesource.com/platform/libcore/+/refs/tags/android-7.0.0_r1/ojluni/src/main/java/sun/nio/ch/FileChannelImpl.java)
signals registered native threads before closing its parent. The signal path is
implemented by [Android 7.0 `NativeThreadSet`](https://android.googlesource.com/platform/libcore/+/refs/tags/android-7.0.0_r1/ojluni/src/main/java/sun/nio/ch/NativeThreadSet.java)
and [`NativeThread.c`](https://android.googlesource.com/platform/libcore/+/refs/tags/android-7.0.0_r1/ojluni/src/main/native/NativeThread.c);
the channel's read path stops retrying once closed. The [Android 15 AOA driver](https://android.googlesource.com/kernel/common/+/refs/heads/android15-6.6/drivers/usb/gadget/function/android_f_accessory.c)
confirms that accessory reads wait on USB request completion and that the file
operations have no `poll` callback.

For a PC-requested stop, use the existing authenticated ANPC STOP/ACK exchange
and then close the transport. For a local Android user stop, invalidate the
session generation and stop PCM immediately, then close both channels on the
cleanup worker; the Host observes the resulting I/O failure or heartbeat
timeout. There is no new receiver-to-host STOP event in this protocol. Channel
close may wait for blocked reads/writes to return, so it must never run on the
main thread. The PFD is not considered closed until record I/O is quiescent and
its owner has closed it; the coordinator also joins its stream workers before
discarding session state.

The reader reads exactly the eight-byte ANPU header, validates it before
allocation, then reads exactly the bounded body. Short transfers accumulate
until the record is complete. USB zero-length packets are not ANPU records and
do not delimit messages; the transport adapter continues after a live-pipe
zero-length transfer, with a bounded no-progress/deadline policy. EOF,
cancellation, a detached device, or a pipe error ends the session. The adapter
does not spin indefinitely on zero progress.

One writer owns the USB OUT pipe and serializes complete ANPU records so that
control and audio bytes cannot interleave. It pulls audio online from the
existing bounded PCM source; it does not add a second audio staging queue. A
bounded control queue is serviced before the next audio record. A partial
write advances an offset within the same record. A zero-byte write waits only
within the bounded/cancellable I/O policy and checks the current generation.
No packet is re-encoded or assigned a new sequence to continue a partial write.
STOP invalidates the generation and discards audio records not yet started. If
a record is cancelled or fails after a partial transfer, the whole stream is
closed and the session is discarded; the receiver never attempts to find a
boundary in the remaining old bytes. The maximum body remains 1200 bytes, and
no additional audio queue is introduced by this envelope.

On Windows, WinUSB operations must also be bounded and cancellable. The
implementation may use overlapped operations or the existing pipe timeout
policy with `WinUsb_AbortPipe`; it does not block indefinitely waiting for a
reader before it can cancel. The read and write sides have separate workers.
Cancellation signals and joins each worker before `WinUsb_Free` or closing its
safe handle. The audio writer does not use periodic 250 ms reads or waits as a
polling loop.

Shutdown first invalidates the session generation, cancels pending WinUSB
operations, and stops new control/audio writes. It then joins the stream reader
and writer before freeing WinUSB handles, queue state, or key material. Android
closes the accessory descriptors only after its worker exits. A new session
gets a fresh ANPB-derived AOA secret, handshake, keys, and queue.

## Windows and Android device selection

Windows uses the existing `windows` crate and SetupAPI/WinUSB. It enumerates
present device interfaces for a user-selected Android device, reads the
registered device-interface class GUID and properties from that installed
PnP interface, and opens only the accessory interface. AOA accessory IDs
18D1:2D00 and 18D1:2D01 may identify candidate accessory devices, but a matching
USB string or ID is not authenticated identity. On composite 2D01 devices,
only MI_00 is eligible; MI_01 is the ADB interface and is never opened for this
audio transport. The chosen interface must expose one bulk IN and one bulk OUT
pipe. The mode-switch path separately filters the supported iQOO 11
V2243A `VID_2D95&PID_6013&MI_02` interface and requires an explicit UI
confirmation before sending the three accessory requests. Product code does
not replace drivers or modify registry entries. The existing diagnostic probe
retains its `AnPod Lab` / `AOA Bulk Probe` accessory strings and echo protocol;
that identity is not product trust.

The existing Windows `windows` crate can provide SetupAPI, device installation,
WinUSB, registry-property, and overlapped-I/O declarations by adding only the
required feature flags (`Win32_Devices_DeviceAndDriverInstallation`,
`Win32_Devices_Usb`, `Win32_System_IO`, `Win32_System_Registry`). No USB
dependency or cryptographic dependency is needed. Persisted device selection
must not reuse an ADB serial as a trust key or as a substitute for the ANPB
identity.

The Android playback entry accepts only the product accessory identity
(manufacturer `AnPod`, model `AnPod USB Direct`) after the foreground user
grants access. The existing diagnostic echo probe keeps its separate
`AnPod Lab` / `AOA Bulk Probe` identity. `UsbManager.requestPermission`,
`openAccessory`, accessory detach, and descriptor close remain explicit
lifecycle events. AOA does not create an ADB-shell audio path or bypass ANPB
mutual authentication and the Receiver's single-owner policy. LAN background
authorization and trust behavior remain unchanged.

## Offline verification and limits

The local test suite uses fixed synthetic data only. It contains a sample
ANPB record, derived AOA binding secret, profile descriptors, P5-A v2 handshake
proofs and keys, P5-C control and ANPD v2 FORMAT/AUDIO records, ANPU wrappers,
and negative wrong-domain expectations. It never contains a real binding or
session secret. Rust and Java checks consume the same file through their
production security, control, audio packet, and ANPU codecs.

Offline implementation checks completed on 2026-10-20:

- JDK 17 `packaging/check-java-offline.ps1` passed with the production
  `AnpuRecordCodec.read`/`write` methods. Synthetic streams cover short and
  coalesced records, EOF/truncation, zero-progress failure, cancellation, and
  complete record writing. Android Receiver JVM tests passed 55/55 after the
  codec change.
- Rust `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`, normal
  strict Clippy, and the focused AOA/security/P5-C/ANPD tests passed. The focused
  set passed 7/7 after adding fatal cancellation-after-partial-read/write
  checks. The full locked offline single-thread suite had 120 passed, 0 failed,
  1 ignored, and 2 filtered in 126.92 seconds before that final test-only
  addition; it was not rerun afterward. Formatting, strict Clippy, and focused
  tests were rerun after the addition.

The first full Rust attempt ran its Java fixture with an unintended JDK 16 and
failed 16 fixture checks; the same suite passed after selecting the approved
JDK 17. No source assertion was changed to make that environment failure pass.

## 6b1 platform I/O foundation status

The Android `AoaAccessoryIo` owner and Windows `aoa_winusb` transport owner
have offline implementations. Android's focused `AoaAccessoryIoTest` passed
5/5 using fake channels with the production ANPU record codec. It ran with
JDK 17 and SDK 35 using:

```powershell
gradlew.bat -p src/AnPod.Receiver :app:testDebugUnitTest --tests app.anpod.receiver.AoaAccessoryIoTest --offline --no-daemon -PreceiverBuildDir=<absolute-repo>\artifacts\phase6b1-android-build
```

Replace `<absolute-repo>` with the local checkout path; this is equivalent to
the recorded invocation and keeps machine-specific paths out of the document.

The XML result is
`artifacts/phase6b1-android-build/test-results/testDebugUnitTest/TEST-app.anpod.receiver.AoaAccessoryIoTest.xml`.
Windows focused AOA I/O checks passed 7/7 (124 filtered); Rust formatting,
strict Clippy, and the Windows MSVC-target check also passed. Exact Windows
commands and raw logs:

```powershell
cargo test --manifest-path src-tauri\Cargo.toml --locked --offline aoa_winusb::tests -- --nocapture
cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
cargo clippy --manifest-path src-tauri\Cargo.toml --locked --offline -- -D warnings
cargo check --manifest-path src-tauri\Cargo.toml --locked --offline --target x86_64-pc-windows-msvc
```

Logs: `artifacts/phase6b-focused-tests.log`, `artifacts/phase6b-fmt-check.log`,
`artifacts/phase6b-clippy.log`, and `artifacts/phase6b-windows-check.log`;
each reports exit code 0. The focused test log contains unrelated test-build
warnings, while strict Clippy passed. These checks cover record-stream
cancellation, deadlines, short transfers, interface discovery logic and
handle ownership; they do not prove behavior on OEM Android accessory drivers
or a physical Windows/USB connection.

This milestone supplies only platform I/O owners. It does not establish the
authenticated session/ingress wiring in 6b2 or the UI/state product wiring in
6c. Real AOA mode switching, Android permission behavior on OEM devices,
Windows PnP/WinUSB access with hardware, USB throughput, 96 kHz compatibility,
latency, and audible quality remain `NOT_TESTED`.

## 6b2a shared ingress and Receiver audio-worker foundation

The Receiver now has a shared authenticated ANPD ingress for existing UDP and
AOA contexts, plus an AOA session audio context that creates the profile-matched
bounded queue before playback starts. Existing v1 UDP still uses its PCM queue
accept path; locked v2 PCM16 uses the PCM16 queue worker, while packed-PCM24
profiles use the Float queue worker. AOA requires a valid FORMAT record before
accepting AUDIO. The Service can accept an already-authenticated AOA audio
request, but its coordinator still owns AOA context lifetime and must stop I/O,
join its reader/writer, and then destroy that context. The Service only stops
the queue and does not join an AOA reader. This is shared ingress and worker
plumbing, not complete authenticated-session product integration; the 6b2
Coordinator and 6c UI/state wiring remain pending.

Offline verification on 2026-10-20:

- `packaging/check-java-offline.ps1` passed all 7 offline Java checks.
- Android Receiver JVM tests passed 61/61 (9 XML suites; 0 failures, errors, or
  skipped tests). Logs: `artifacts/phase6b2-java-offline.log` and
  `artifacts/phase6b2-android-jvm.log`.

These tests verify shared software ingress and Service worker behavior with
synthetic/fake inputs. They do not establish a complete AOA connection or test
USB permission, detach, Windows device I/O, physical playback, throughput,
latency, or acoustics; those remain `NOT_TESTED`.

## 6b2b authenticated-session core status

The host has an authenticated AOA owner that consumes the shared PCM16/Float32
queues, validates each queue against its locked descriptor, runs P5-C control,
and emits ANPD v2 records over the ANPU writer. Pause clears pending source
audio; a stopped source queue requires a fresh session. The Receiver has an
isolated coordinator that authenticates P5-A v2, requires the AOA capability
query first, validates raw P5-C frames, registers the descriptor-matched
ingress before acknowledging START, and owns transport/context teardown. Its
Service endpoint reuses the existing audio workers and playback controls. The
6b2 coordinator tests use fake channels and an in-memory endpoint; production
`UsbManager.openAccessory`, permission handling, Android Coordinator
ownership, and the host State/command route are recorded under 6c below.

Offline verification on 2026-10-20:

- Receiver `AoaReceiverCoordinatorTest` focused checks passed 3/3. The full
  Receiver JVM suite passed 64/64 (10 XML suites; 0 failures, errors, or
  skipped tests), and `packaging/check-java-offline.ps1` passed all 7 checks.
  Logs: `artifacts/phase6b2b-focused.log`,
  `artifacts/phase6b2b-android-jvm.log`, and
  `artifacts/phase6b2b-java-offline.log`.
- Rust focused owner checks passed 6/6, covering three profiles, wrong binding
  domain, queue/descriptor mismatch, owner drop/join, active queue stop, and
  expired pending frames after a fake write stall. WinUSB focused checks passed
  7/7; P5-C raw control vector passed 1/1. Rust formatting and normal strict
  Clippy passed. The final elevated locked/offline Rust suite passed 135 tests,
  with 0 failures, 0 ignored, and 2 filtered in 95.81 seconds. The filters were
  `adb_stdin_child_helper` and
  `p1_real_usb_installed_receiver_health_start_stop_when_explicitly_requested`.
  Logs: `artifacts/phase6b2-final-owner.log`,
  `artifacts/phase6b2-final-winusb.log`, `artifacts/phase6b2-p5c-vector.log`,
  `artifacts/phase6b2-final-fmt.log`, `artifacts/phase6b2-final-clippy.log`,
  and `artifacts/phase6b2-full-rust-elevated.log`.

The full-suite command was the locked, offline, single-thread Cargo test run
with `adb_stdin_child_helper` and the explicitly opted-in real-USB receiver
health test filtered. `reads_the_artifact_written_by_windows_powershell_when_requested`
was not filtered, but `ANPOD_RECEIVER_ARTIFACT_DIR` was unset, so that test took
its no-artifact early-return path; Windows artifact consumption is not verified
by this run.

These checks exercise synthetic handshakes, fake byte-stream I/O, queue
lifecycles, and the existing audio worker. They do not exercise the Android
permission dialog, accessory attachment/detachment, Windows PnP/WinUSB hardware,
complete product session recovery, physical playback, throughput, latency, or
acoustics. Those remain `NOT_TESTED`.

## 6c product entrypoint and offline status

The Receiver foreground UI exposes USB Direct and requests Android accessory
permission for the selected device. Its coordinator owns the PFD, transport
workers, authenticated context, and teardown; the Service owns audio queue
playback but does not join USB I/O workers. The host exposes target listing and
`start_aoa_audio`, requires an explicit saved binding and target selection, and
reports the session as `aoa` / `usbDirect`. USB Direct does not alter the
persistent USB/ADB or LAN preference and does not silently fall back to ADB.
The app also lists only the supported iQOO 11 V2243A /
`VID_2D95&PID_6013&MI_02` pre-switch interface and sends mode-switch requests
only after explicit user confirmation. This request does not start audio; the
user must grant Android accessory permission, wait for re-enumeration, then
select an accessory and saved binding and start the connection. On Windows the
product transport requires an available WinUSB accessory interface.

Already connected standard USB/ADB and LAN sessions permit switching between
balanced and low-latency playback modes; AOA playback mode remains locked.
Audio quality profile and format changes remain locked to a new session. USB
Direct setup and ADB-server consent behavior are described at the start of this
document.

The final Phase 6c offline source gates completed on 2026-10-20:

- Host Rust full locked/offline suite: 137 passed, 0 failed, 0 ignored, 2
  filtered (96.81 seconds), before the final opaque-device-label-only change.
  After that change, formatting, AOA focused tests (18/18), and the Windows
  target check passed. The labels are the fixed product name; the opaque PnP
  instance ID is retained only as an internal target identifier.
- Android Receiver JVM suite: 70 tests, 0 failures/errors/skips across 12 XML
  suites. The final callback-only test then passed focused (1/1); Java offline
  checks passed 7/7. Logs: `artifacts/phase6c-full-jvm.log`,
  `artifacts/phase6c-focused-final.log`, and `artifacts/phase6c-java-offline.log`.
- Frontend production build and persistent AOA smoke check passed for
  explicit target/binding selection, cancellation, actual transport display,
  and active-session mode/backend locks.

The signed Receiver APK build is verified. Portable package checks and its final SHA-256 are recorded in the development plan after package generation; they are not embedded in this document, which ships in the package. USB permission UX on hardware, accessory attach/detach recovery,
Windows PnP/WinUSB hardware access, real playback, throughput, acoustic
latency, and long-duration stability remain `NOT_TESTED`. Source and synthetic
gates do not establish those results.

The following PC 1.10.2 artifact records describe previous candidates. The
current Receiver release artifact and verification status are recorded below.

The previous Receiver candidate was `0.17.0-v3-flow-candidate` / code 34, built from
source commit `8ca9638db128f99e36769f42de18ed9244f457c2` with
`sourceModified=false`. Its APK SHA-256 is
`711c20c4348b1865d0475244271271f1fc32d70888f1f77054481bba138fed83`; the
verified release signer is
`f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`. Rust
loaded and read this exact APK through the artifact test. Signature and APK
inspection do not verify device installation, in-place upgrade, permission
behavior, or playback.

The previous Receiver crash-fix candidate was `0.17.1-v3-flow-candidate` / code
35, built from source `2bd998d65fe650bb225c09d7353c5c2ba39febfc` with
`sourceModified=false`. Its release-signed APK SHA-256 is
`8f8e76e4b9e2dc1e3bb8d4575f5e21e77838056b2b55536c5736eedcccf4f15f`; the
verified signer remains
`f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`. The
exact artifact passed the Rust artifact-read test. Package inspection does not
verify installation, upgrade, permission UX, or playback on a device.

## Previous PC 1.10.3 / Receiver 0.17.2 code 36 behavior and verification

Every initial connection requires a manual PC click, including when there is
only one LAN offer. Remembered authorization does not automatically connect.
Only an unexpected drop from an established session starts bounded reconnect,
with the same transport, target, binding, profile, mode, and output. A manual
disconnect from the PC or phone cancels reconnect. An initial failure is not
retried in the background, and AOA does not fall back to ADB.

Balanced and low-latency switching is supported on USB/ADB, LAN, and AOA. AOA
requires Receiver capability `0x200`; a mode change creates a newly
authenticated session with fresh keys and queue. An older Receiver without the
capability prompts for an update. Hi-Res requires actual 96 kHz Windows output;
at 48 kHz the app gives a clear prompt without resampling or silent downgrade.
AOD identifies USB Direct accurately and shows receive statistics only from
real PCM ingress, shared by Oboe and AudioTrack.

The signed release APK is `artifacts/connection-v3-receiver-release/receiver.apk`:
version `0.17.2-v3-flow-candidate` / code 36, minSdk 23, targetSdk 35, SHA-256
`0f34c03b5618182fcd90f5a33883bd7a6de5f5a4696d9c1a64603f3f4983194e`, signer
SHA-256 `f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573`.
Its metadata records source commit `ce81f03b54285691b3ce602461180516b60a1d4b`,
`sourceModified=false`, and source-input SHA-256
`2599b5dcc07c92303fc4bf1f082dc17ef01113c28d897276253cd41fd9199f6e`.

Offline verification on the signed source snapshot: `ReceiverServiceRobolectricTest`
passed 33/33 and `AoaReceiverCoordinatorTest` passed 6/6 in the same run (their
XML reports show 0 failures, errors, or skipped tests). Earlier 32/32 and 8/8
runs are separate and are not added to these counts. Frontend TypeScript and
Vite checks passed, as did the Playwright matrix covering four transports,
modes, manual offer handling, and quality selection. The older broad UI harness
remains partial because some assertions target the previous UI. Rust formatting
and checks, five focused test groups, APK artifact reading, and the serial
locked/offline suite passed (145 passed, 0 failed, 1 ignored). Real-device AOA
permissions and attach/detach behavior, Windows hardware access, installation
or upgrade, playback, acoustic quality, and long-duration stability remain
`NOT_TESTED`.

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
