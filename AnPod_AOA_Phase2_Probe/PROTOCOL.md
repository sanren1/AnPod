# AOA 4096-byte fixed-block Echo Protocol v1

This is a diagnostic protocol only, not AnPod transport or an audio protocol.

Transport: WinUSB bulk OUT (PC → Android) and bulk IN (Android → PC). Both sides treat this as a byte stream and transfer exactly 4096-byte frames. The Android endpoint echoes only after byte-for-byte validation.

| Offset | Size | Encoding | Contents |
|---|---:|---|---|
| 0 | 4 | ASCII | `APAO` |
| 4 | 1 | u8 | `1`, protocol version |
| 5 | 1 | u8 | `1`, echo request |
| 6 | 2 | u8 | reserved 0 |
| 8 | 4 | little endian u32 | sequence, 1..512 |
| 12 | 4 | little endian u32 | total frame length, `4096` |
| 16 | 4080 | raw bytes | byte[i] = `(seq * 131 + i * 17 + 7) & 255` |

Windows verifies 4096 bytes of echoed data exactly and counts p50, p95 of synchronous roundtrip per frame. 4096 bytes / transaction is **not** an audio packetization recommendation. No encryption, authentication or link-level retry exists in this lab format. It must not be exposed to networks or promoted to production protocol.

To minimize interoperability ambiguity, the test **requires an Android-side app and matching Windows source in this same package version**, not unrelated third-party AOA accessories. Android USB stream may split frames across multiple `read()` results, so its `readExactly()` loop is mandatory.
