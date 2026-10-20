# AOA direct audio transport (Phase 6a protocol and 6b1 I/O foundation)

This specification defines the accepted accessory-mode byte-stream transport
for an AnPod host and Receiver pair. Phase 6a implements its offline record
codec, transport-specific binding derivation, and shared-vector checks. It
reuses the existing ANPB binding,
P5-A v2 handshake, P5-C authenticated control messages and ANPD v2 encrypted
audio packets. It does not reuse the AOA 4096-byte echo probe, ADB TCP, or the
LAN UDP transport context. Phase 6a protocol codecs and the 6b1 platform I/O
owners are implemented as offline foundations. They do not yet integrate a
complete AOA session into the product audio path, and no physical accessory
transfer has been tested.

The Receiver must have the matching accessory-mode application installed. The
first product stage connects only to a target already in AOA accessory mode;
the Android user explicitly selects USB Direct and the current accessory,
grants `UsbManager` permission, and starts playback from the foreground UI.
That action authorizes only the current accessory connection/session. AOA
vendor requests 51/52/53 for mode switching remain in the separate manual
probe workflow and are not sent by product code. A detach, permission
revocation, or user cancellation ends the current session.

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
pipe. Product code does not replace drivers, modify registry entries, or send
accessory-switch control requests. The existing diagnostic probe retains its
`AnPod Lab` / `AOA Bulk Probe` accessory strings and echo protocol. A separate
future manual mode-switch tool may identify the product as manufacturer
`AnPod`, model `AnPod USB Direct`, but still requires explicit SWITCH
confirmation; the diagnostic probe identity is not product trust.

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
`AnPod Lab` / `AOA Bulk Probe` identity. A future manual mode-switch workflow
uses the product identity and still requires explicit SWITCH confirmation;
the diagnostic probe identity is not production trust. `UsbManager.requestPermission`,
`openAccessory`, accessory detach, and descriptor close remain explicit
lifecycle events. AOA does not create an ADB-shell audio path or bypass ANPB
mutual authentication and the Receiver's single-owner policy. LAN background
authorization and trust behavior remain unchanged.

## Offline verification and limits

`tests/aoa-v1-vectors.txt` is fixed synthetic data only. It contains a sample
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
:app:testDebugUnitTest --tests app.anpod.receiver.AoaAccessoryIoTest --offline --no-daemon -PreceiverBuildDir=F:\Program\AnPod\artifacts\phase6b1-android-build
```

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
