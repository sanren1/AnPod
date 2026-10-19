# Phase 1 audio format and authentication v2

This specification describes an offline negotiation simulator. Product playback,
discovery, P5-C capabilities, native capture and ReceiverService remain v1 PCM16.
All integer fields are unsigned big endian; PCM sample bytes are little endian.

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
