# Audio and USB measurements

This page separates measured historical results from source/offline checks.
Phase 6 product State, permission, and UI integration has offline acceptance,
and a release-signed Receiver candidate has been built. No current AOA path has
been exercised on a phone or PC. Physical playback, acoustic latency,
sustained USB throughput, and long-run stability are `NOT_TESTED`.

## Historical AnPod acoustic latency

The only repository-recorded AnPod end-to-end acoustic short test is PC 1.2.1
with Receiver 0.9.1/code 12. Each route/mode ran for about 24 seconds with ten
prompt sounds; 10/10 were matched and independently reviewed.

| Route and mode | Median | P95 |
| --- | ---: | ---: |
| USB / ADB · Balanced | 156.720 ms | 157.111 ms |
| USB / ADB · Low Latency | 123.269 ms | 125.299 ms |
| LAN / UDP · Balanced | 310.645 ms | 314.294 ms |
| LAN / UDP · Low Latency | 187.394 ms | 200.240 ms |
| PC local reference | 71.699 ms | 71.778 ms |

This is an end-to-end acoustic/microphone measurement, not transport-only
latency. The PC local path is an independent reference and is not subtracted
from the other values. The UDP Low Latency group had two low match scores and
is only indicative. These results predate the current audio implementation.
The raw test procedure is linked from the [README latency record](../../README.md#延迟测试记录).

## Historical Android output diagnostics

An exploratory iQOO 11 run recorded OboeTester at 48 kHz PCM16 stereo, 192
frames per burst and a 384-frame buffer. OboeTester reported a timestamp-based
latency estimate around 30–31 ms and zero XRuns in the captured screenshot.
That timestamp estimate is not a speaker/microphone acoustic measurement.

A separate historical dual-engine CSV reported:

| Engine | P50 | P95 | Counter recorded |
| --- | ---: | ---: | --- |
| AudioTrack | 30.10 ms | 31.86 ms | 4 underruns |
| Oboe | 30.33 ms | 32.19 ms | 0 XRuns |

This was one non-equivalent-counter comparison. It does not establish an Oboe
latency or sound-quality advantage, and it is not a Phase 5 validation of the
current backend. Current Oboe routing, focus behavior, output format, XRun
rates, and acoustic quality on hardware remain `NOT_TESTED`.

## Historical AOA echo probe

The separate AOA probe on an iQOO 11 / Android 16 completed 16/16 and 64/64
4096-byte echo blocks. The recorded application round-trip values were P50
3.29 ms / P95 10.31 ms for the 16-round run and P50 0.69 ms / P95 0.92 ms for
the 64-round run. One zero-byte USB IN read occurred per round; its cause was
not established. This is bidirectional request/response echo RTT, not audio
latency, sustained unidirectional throughput, or evidence that the current
ANPU transport works on hardware. The probe uses the separate `AnPod Lab` /
`AOA Bulk Probe` identity.

## Current source verification is not a performance benchmark

The phase documents record codec vectors, bounded queues, packetization, fake
stream I/O, owner lifecycle tests, offline builds, and UI checks. Those verify
contracts and failure handling; they do not report measured audio throughput,
real capture rate, physical output precision, acoustic latency, or extended
stability. In particular, an ANPD timestamp and an echo RTT must not be
presented as acoustic latency.

For reproducible commands and per-phase test counts, see:

- [Phase 2 Float capture](02-float-capture.md)
- [Phase 5 Oboe output](05-oboe-output.md)
- [Phase 6 AOA transport](06-aoa-transport.md)
- [Compatibility and offline gate record](COMPATIBILITY.md)

The current debug APK hash and packaging checks are in the Phase 5 record;
they are not release-signature upgrade or device-playback evidence.
