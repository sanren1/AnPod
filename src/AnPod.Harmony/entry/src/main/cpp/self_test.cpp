// On-device self-tests, run by ohosTest CoreSelfTest through the NAPI selfTest() export.
// Expected vectors come from Python hmac/hashlib and from PairingProtocol.java's own *ForVector helpers
// (see .superpowers/sdd/2026-11-14-harmonyos-m1-wired/findings.md), never from this C++ code.
#include "aod_sampler.h"
#include "anhb_codec.h"
#include "anpd_ingress.h"
#include "anpu_codec.h"
#include "buffer_budget.h"
#include "crypto.h"
#include "float_ring.h"
#include "wireless_discovery.h"
#include "p5c_control.h"
#include "pairing_protocol.h"

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

namespace anpod {

void runSessionSelfTests(std::vector<std::string>& failures);  // self_test_session.cpp

namespace {

bool aodSamplesAreSessionScoped() {
    AodSampler session;
    const float stereo[]{-0.25f, 0.75f};
    if (session.snapshot(0, 0).bitsPerSecond || session.snapshot(0, 0).receiving) return false;
    session.audio(0, stereo, 2, true, 0);
    session.audio(2, stereo, 2, false, 10000000);
    session.settlement(0, 3, 48000, true);
    auto recent = session.snapshot(10000000, 16);
    if (!recent.receiving || recent.left[0] != .25f || recent.right[0] != .75f ||
        !recent.lossPercent || std::abs(*recent.lossPercent - 200.0 / 3) > .001) return false;
    auto idle = session.snapshot(500000000, 16);
    // Late delivery fills the gap. Only accepted frames in the trailing minute are eligible as good.
    session.audio(1, stereo, 2, true, 600000000);
    session.settlement(0, 3, 48000, true);
    auto reordered = session.snapshot(600000000, 24);
    if (!reordered.lossPercent || std::abs(*reordered.lossPercent - 100.0 / 3) > .001) return false;
    session.audio(62, stereo, 2, true, 700000000);
    session.settlement(0, 63, 1, true);
    auto minute = session.snapshot(700000000, 32);
    if (!minute.lossPercent || std::abs(*minute.lossPercent - 5900.0 / 60) > .001) return false;
    session.settlement(0, 63, 1, false);
    if (session.snapshot(700000000, 32).lossPercent) return false;
    AodSampler replacement;
    return idle.bitsPerSecond == 256 && idle.left[0] == 0 &&
           !session.snapshot(2700000001LL, 32).receiving &&
           !replacement.snapshot(500000000, 0).lossPercent;
}

Bytes fromHex(const char* h) {
    Bytes out;
    auto nib = [](char c) { return c <= '9' ? c - '0' : c - 'a' + 10; };
    for (size_t i = 0; h[i] && h[i + 1]; i += 2) out.push_back(static_cast<uint8_t>(nib(h[i]) << 4 | nib(h[i + 1])));
    return out;
}

bool eqHex(std::span<const uint8_t> got, const char* hex) {
    Bytes want = fromHex(hex);
    return got.size() == want.size() && std::memcmp(got.data(), want.data(), want.size()) == 0;
}

template <size_t N> std::array<uint8_t, N> filled(uint8_t v) {
    std::array<uint8_t, N> a;
    a.fill(v);
    return a;
}

std::span<const uint8_t> str(const char* s) { return {reinterpret_cast<const uint8_t*>(s), std::strlen(s)}; }

bool discoveryMatchesAndroid() {
    Key32 secret{};
    for (size_t i = 0; i < secret.size(); ++i) secret[i] = static_cast<uint8_t>(i);
    BindingMaterial material({}, {}, secret);
    auto probe = fromHex("414e504403060000000102030405060708090a0b0c0d0e0f000026414e36e4dc63e683ec229f58b0088dd60051cb736e6db8");
    auto reply = authenticatedCandidate(material, probe, 0x1234);
    BindingMaterial unrelated({}, {}, Key32{});
    if (authenticatedCandidate(unrelated, probe, 0x1234)) return false;
    probe.back() ^= 1;
    return reply && eqHex(*reply, "414e504403070000000102030405060708090a0b0c0d0e0f123426414e36e4dc63e6cc22b8c876869dc9aabbd5b84d90c466") &&
           !authenticatedCandidate(material, probe, 0x1234);
}

bool offerMatchesDesktop() {
    Nonce16 intent{};
    for (size_t i = 0; i < intent.size(); ++i) intent[i] = static_cast<uint8_t>(i);
    auto candidate = fromHex("414e504402040000000102030405060708090a0b0c0d0e0fc26c");
    auto offer = connectionOffer(candidate, intent, 0x1234);
    if (!offer || !eqHex(*offer, "414e504402050000000102030405060708090a0b0c0d0e0f1234")) return false;
    candidate[8] ^= 1;
    if (connectionOffer(candidate, intent, 0x1234)) return false;
    candidate[8] ^= 1;
    candidate.pop_back();
    return !connectionOffer(candidate, intent, 0x1234);
}

bool hmacRfc4231Case2() {
    return eqHex(hmacSha256(str("Jefe"), str("what do ya want for nothing?")),
                 "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
}

bool aes256GcmRoundtripAndTamper() {
    // NIST GCM test case 15 (AES-256, 96-bit IV, no AAD): pins the ct||tag layout.
    Bytes key = fromHex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
    Bytes iv = fromHex("cafebabefacedbaddecaf888");
    Bytes pt = fromHex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72"
                       "1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255");
    std::span<const uint8_t, 32> k(key.data(), 32);
    std::span<const uint8_t, 12> n(iv.data(), 12);
    Bytes sealed, opened;
    if (!aes256GcmSeal(k, n, {}, pt, sealed) ||
        !eqHex(sealed, "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa"
                       "8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015ad"
                       "b094dac5d93471bdec1a502270e3cc6c"))
        return false;
    if (!aes256GcmOpen(k, n, {}, sealed, opened) || opened != pt) return false;

    Bytes aad = {'A', 'N', 'P', 'U'};
    if (!aes256GcmSeal(k, n, aad, pt, sealed) || !aes256GcmOpen(k, n, aad, sealed, opened) || opened != pt)
        return false;
    Bytes tampered = sealed;
    tampered.back() ^= 1;
    Bytes wrongAad = {'A', 'N', 'P', 'V'};
    Bytes shortInput(15, 0);
    return !aes256GcmOpen(k, n, aad, tampered, opened) && opened.empty() &&
           !aes256GcmOpen(k, n, wrongAad, sealed, opened) && !aes256GcmOpen(k, n, aad, shortInput, opened);
}

BindingMaterial vectorMaterial() { return {filled<16>(1), filled<16>(2), filled<32>(3)}; }

bool deriveAdbStreamVector() {
    BindingMaterial derived = vectorMaterial().deriveAdbStream();
    auto record = derived.encode();
    bool ok = derived.hostId() == filled<16>(1) && derived.receiverId() == filled<16>(2) &&
              eqHex(std::span<const uint8_t>(record).subspan(37),
                    "cb76cb157e4c5446d03232534f61486a750cb61ff29ccf8882b09c3ffe1d0c37");
    secureZero(record.data(), record.size());
    return ok;
}

bool bindingEncodeDecodeRoundtrip() {
    auto record = vectorMaterial().encode();
    auto decoded = BindingMaterial::decode(record);
    bool ok = decoded && decoded->encode() == record && record[0] == 'A' && record[3] == 'B' && record[4] == 1 &&
              eqHex(vectorMaterial().commitment(),
                    "952198dd3179477780e017fe59b0ade9a12096497671b22829aba2fb4cc4bb7b");
    auto badMagic = record;
    badMagic[0] = 'X';
    auto badVersion = record;
    badVersion[4] = 2;
    ok = ok && !BindingMaterial::decode(badMagic) && !BindingMaterial::decode(badVersion);
    secureZero(record.data(), record.size());
    secureZero(badMagic.data(), badMagic.size());
    secureZero(badVersion.data(), badVersion.size());
    return ok;
}

bool pairingCodeVector() { return vectorMaterial().pairingCode(filled<16>(4), filled<16>(5)) == "017647"; }

// Fixture printed by PairingProtocol.V2.{hello,challenge,proofMessage,ready,derive}ForVector with
// hostId 0x11*16, receiverId 0x22*16, secret 0x33*32, hostNonce 0x44*32, receiverNonce 0x55*32,
// sessionId 0x0102030405060708, AudioFormatV2.forProfile(5, 1); cross-checked with Python.
constexpr const char* kHello =
    "414e50350201006011111111111111111111111111111111222222222222222222222222222222224444444444444444"
    "444444444444444444444444444444444444444444444444010203040506070802050302200401000001770000080000";
constexpr const char* kChallenge =
    "414e503502020068222222222222222222222222222222225555555555555555555555555555555555555555555555555555"
    "555555555555020503022004010000017700000800007dcab95426a901b2481236f237f0dc7cf72a806404a64d10f85ce413ff"
    "299c4b";
constexpr const char* kFinish = "414e503502030028486ac4767bbeadb6f7ec87966172680ae4e9ce8eb752b1994ac450741dc391da";
constexpr const char* kReady = "414e503502040028ad41b1aa31f0d1dc21afdd4d56325ac9dccdcbc5f72061cde80d78978b41e324";

bool v2HandshakeAgainstJavaTranscript() {
    BindingMaterial material(filled<16>(0x11), filled<16>(0x22), filled<32>(0x33));
    Bytes hello = fromHex(kHello);
    auto started = v2::acceptHelloWithNonce(material, hello, filled<32>(0x55));
    auto* await = std::get_if<v2::ReceiverAwaitFinish>(&started);
    if (!await || !eqHex(await->challenge(), kChallenge)) return false;

    auto finished = await->acceptFinish(fromHex(kFinish));
    auto* result = std::get_if<v2::FinishResult>(&finished);
    if (!result || !eqHex(result->ready, kReady) || result->sessionId != 0x0102030405060708ULL ||
        result->format.encode() != AudioFormatV2::forProfile(5, 1)->encode() ||
        !eqHex(result->keys.hostToReceiverAudio, "1ca4c71d6bed7d41f1ac1c9beb67aa5421716ccd94eb6cf9ec920ca47af10022") ||
        !eqHex(result->keys.receiverToHostAudio, "bc267babe05ce114bff67a2d0456b87db690cd7fdc493e39de866585b1251381") ||
        !eqHex(result->keys.hostToReceiverControl, "e6d7cb8f4a0025cc3ccf8e35e2edba7dd27c5f4efcaaff6c6c7a208de301e85b") ||
        !eqHex(result->keys.receiverToHostControl, "b88971d2a43d135d4df815b93d2847e1f12c586427a8a11e0b35befdb74a1f36"))
        return false;
    // One shot: a replayed finish on the same state is refused.
    auto again = await->acceptFinish(fromHex(kFinish));
    if (!std::holds_alternative<v2::Failure>(again)) return false;

    // Tampered host proof -> InvalidProof; wrong identity / bad length / bad descriptor -> refused.
    auto restart = v2::acceptHelloWithNonce(material, hello, filled<32>(0x55));
    Bytes badFinish = fromHex(kFinish);
    badFinish.back() ^= 1;
    auto bad = std::get<v2::ReceiverAwaitFinish>(restart).acceptFinish(badFinish);
    auto* badFailure = std::get_if<v2::Failure>(&bad);
    Bytes wrongHost = hello;
    wrongHost[8] ^= 1;
    Bytes wrongFormat = hello;
    wrongFormat[8 + 72 + 1] = 9;
    Bytes truncated(hello.begin(), hello.end() - 1);
    auto failureOf = [&](const Bytes& m) {
        auto r = v2::acceptHello(material, m);
        auto* f = std::get_if<v2::Failure>(&r);
        return f ? *f : v2::Failure::Consumed;
    };
    return badFailure && *badFailure == v2::Failure::InvalidProof &&
           failureOf(wrongHost) == v2::Failure::WrongIdentity &&
           failureOf(wrongFormat) == v2::Failure::InvalidParameters &&
           failureOf(truncated) == v2::Failure::Malformed &&
           std::holds_alternative<v2::ReceiverAwaitFinish>(v2::acceptHello(material, hello));
}


// ---- Task 3: ANPU / ANHB / P5-C / ANPD / ring --------------------------------------------------------------

Bytes handshakeBody40() {
    Bytes b = {'A', 'N', 'P', '5', 2, 3, 0, 40};
    b.resize(40, 0);
    return b;
}

// Smallest valid ANPD audio datagram body for the ANPU audio check: header + 1150 payload + tag = 1200.
Bytes audioBody1200() {
    Bytes b(1200, 0);
    const uint8_t head[8] = {'A', 'N', 'P', 'D', 2, 2, 0, 0};
    std::memcpy(b.data(), head, 8);
    b[32] = 1150 >> 8;
    b[33] = 1150 & 0xff;
    return b;
}

bool readsBack(const Bytes& record, AnpuType type, const Bytes& body) {
    SpanReader r(record);
    auto got = readAnpu(r);
    return got && got->type == type && got->body == body && r.remaining() == 0;
}

// Control record body: the host START frame pinned by p5cControlMatchesJavaVectors.
constexpr const char* kStartFrame =
    "414e504301030000414e504801020304050607080000000000000005000066b4a3aa2a0be2d91cf0ba882cca34c5b50a83a60a306ada093be55455d26e5f";

bool anpuRejectsBadMagicVersionAndOversize() {
    Bytes hs = handshakeBody40();
    Bytes record = encodeAnpu(AnpuType::Handshake, hs);
    const uint8_t head[8] = {'A', 'N', 'P', 'U', 1, 1, 0, 40};
    if (record.size() != 48 || std::memcmp(record.data(), head, 8) != 0 || !readsBack(record, AnpuType::Handshake, hs))
        return false;

    Bytes control = fromHex(kStartFrame);
    Bytes audio = audioBody1200();
    Bytes controlRecord = encodeAnpu(AnpuType::Control, control);
    Bytes audioRecord = encodeAnpu(AnpuType::Audio, audio);  // body 1200 is the largest accepted
    if (controlRecord.size() != 8 + control.size() || audioRecord.size() != 1208 ||
        !readsBack(controlRecord, AnpuType::Control, control) || !readsBack(audioRecord, AnpuType::Audio, audio))
        return false;

    Bytes badMagic = record;
    badMagic[0] = 'X';
    Bytes badVersion = record;
    badVersion[4] = 2;
    Bytes unknownType0 = record;
    unknownType0[5] = 0;
    Bytes unknownType4 = record;
    unknownType4[5] = 4;
    Bytes zeroLength = record;
    zeroLength[7] = 0;
    zeroLength.resize(8);
    Bytes oversize = {'A', 'N', 'P', 'U', 1, 3, 1200 >> 8, 1200 & 0xff};
    oversize[6] = 1201 >> 8;
    oversize[7] = 1201 & 0xff;
    oversize.resize(8 + 1201, 0);
    Bytes truncated(record.begin(), record.end() - 1);
    Bytes wrongBody = record;  // handshake body whose declared P5-A length disagrees
    wrongBody[8 + 7] = 41;
    Bytes shortHeader(record.begin(), record.begin() + 5);
    for (const Bytes* bad : {&badMagic, &badVersion, &unknownType0, &unknownType4, &zeroLength, &oversize,
                             &truncated, &wrongBody, &shortHeader}) {
        SpanReader r(*bad);
        if (readAnpu(r)) return false;
    }
    Bytes tooBig(1201, 0);
    return encodeAnpu(AnpuType::Audio, tooBig).empty() && encodeAnpu(AnpuType::Handshake, {}).empty() &&
           encodeAnpu(AnpuType::Handshake, control).empty();
}

bool anhbRoundtripAllKinds() {
    const struct {
        AnhbKind kind;
        size_t size;
    } kinds[] = {{AnhbKind::TabletHello, 17}, {AnhbKind::PairOffer, 85}, {AnhbKind::PairNonce, 16},
                 {AnhbKind::PairConfirmed, 0}, {AnhbKind::PairSaved, 32},  {AnhbKind::PairCancel, 1},
                 {AnhbKind::Proceed, 0}};
    for (const auto& k : kinds) {
        Bytes body(k.size);
        for (size_t i = 0; i < body.size(); i++) body[i] = static_cast<uint8_t>(i * 7 + static_cast<uint8_t>(k.kind));
        Bytes frame = encodeAnhb(k.kind, body);
        if (frame.size() != 8 + k.size || frame[4] != 1 || frame[5] != static_cast<uint8_t>(k.kind) ||
            frame[6] != 0 || frame[7] != k.size)
            return false;
        SpanReader r(frame);
        auto got = readAnhb(r);
        if (!got || got->kind != k.kind || got->body != body || r.remaining() != 0) return false;
    }
    // Byte-identical to src-tauri/src/hdc_bootstrap.rs: encode_anhb(PROCEED, &[]) == b"ANHB\x01\x07\x00\x00".
    const uint8_t proceed[8] = {'A', 'N', 'H', 'B', 1, 7, 0, 0};
    Bytes frame = encodeAnhb(AnhbKind::Proceed, {});
    return frame.size() == 8 && std::memcmp(frame.data(), proceed, 8) == 0;
}

bool anhbRejectsUnknownKindAndOversize() {
    Bytes proceed = encodeAnhb(AnhbKind::Proceed, {});
    if (proceed.size() != 8) return false;
    auto rejects = [](Bytes frame) {
        SpanReader r(frame);
        return !readAnhb(r);
    };
    Bytes kind0 = proceed;
    kind0[5] = 0;
    Bytes kind8 = proceed;
    kind8[5] = 8;
    Bytes badMagic = proceed;
    badMagic[3] = 'X';
    Bytes badVersion = proceed;
    badVersion[4] = 2;
    Bytes oversize = {'A', 'N', 'H', 'B', 1, 2, 1, 1};  // declares 257 bytes
    oversize.resize(8 + 257, 0);
    Bytes wrongSize = {'A', 'N', 'H', 'B', 1, 3, 0, 15};  // PairNonce needs 16
    wrongSize.resize(8 + 15, 0);
    Bytes shortBody = encodeAnhb(AnhbKind::PairNonce, Bytes(16, 1));
    shortBody.pop_back();
    Bytes unknownBody(1, 0);
    return rejects(kind0) && rejects(kind8) && rejects(badMagic) && rejects(badVersion) && rejects(oversize) &&
           rejects(wrongSize) && rejects(shortBody) && rejects({}) && rejects(Bytes(proceed.begin(), proceed.begin() + 7)) &&
           encodeAnhb(static_cast<AnhbKind>(0), {}).empty() && encodeAnhb(static_cast<AnhbKind>(8), {}).empty() &&
           encodeAnhb(AnhbKind::PairOffer, Bytes(257, 0)).empty() && encodeAnhb(AnhbKind::Proceed, unknownBody).empty();
}

uint64_t be64(const Bytes& b, size_t at) {
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) v = (v << 8) | b[at + i];
    return v;
}
uint32_t be32(const Bytes& b, size_t at) {
    return (uint32_t{b[at]} << 24) | (uint32_t{b[at + 1]} << 16) | (uint32_t{b[at + 2]} << 8) | b[at + 3];
}

// Offsets and lengths copied from P5cProgressExtensionTest.java.
bool progressSchemasMatchJava() {
    std::array<int64_t, 12> counters;
    for (size_t i = 0; i < counters.size(); i++) counters[i] = static_cast<int64_t>(i + 1);
    Bytes legacy(70, 0);
    Bytes ext = p5c::playbackProgressPayloadExtended(legacy, 101, 202, 3, 2, counters, 17, 23);
    if (ext.size() != 196 || ext[70] != 2 || be64(ext, 71) != 101 || be64(ext, 79) != 202 || be32(ext, 87) != 3 ||
        ext[91] != 2 || be32(ext, 188) != 17 || be32(ext, 192) != 23)
        return false;
    for (size_t i = 0; i < counters.size(); i++)
        if (be64(ext, 92 + i * 8) != i + 1) return false;
    if (p5c::playbackProgressPayloadExtended(Bytes(78, 0), 101, 202, 3, 2, counters, 17, 23).size() != 204 ||
        !p5c::playbackProgressPayloadExtended(legacy, 1, 2, 3, 3, counters, 17, 23).empty())
        return false;

    Bytes schema2 = p5c::playbackProgressPayloadExtended(Bytes(78, 0), 101, 202, 3, 2, counters, 17, 23);
    Bytes schema3 = p5c::playbackProgressPayloadSchema3(schema2, 1920, 1955, 35, 420, 4);
    if (schema3.size() != 221 || schema3[78] != 3 || be32(schema3, 204) != 1920 || be32(schema3, 208) != 1955 ||
        be32(schema3, 212) != 35 || be32(schema3, 216) != 420 || schema3[220] != 4)
        return false;
    Bytes schema3V1 = p5c::playbackProgressPayloadSchema3(p5c::playbackProgressPayloadExtended(legacy, 101, 202, 3, 2, counters, 17, 23),
                                                          -1, -1, INT32_MIN, INT32_MIN, 0);
    if (schema3V1.size() != 213 || be32(schema3V1, 196) != 0xffffffffu || be32(schema3V1, 204) != 0x80000000u) return false;

    Bytes schema4 = p5c::playbackProgressPayloadSchema4(schema3, 42);
    if (schema4.size() != 229 || schema4[78] != 4 || be64(schema4, 221) != 42) return false;
    Bytes schema5 = p5c::playbackProgressPayloadSchema5(schema4, p5c::LossWindow{10, 100}, p5c::LossWindow{20, 200},
                                                        p5c::LossWindow{30, 300});
    if (schema5.size() != 277 || schema5[78] != 5 || be64(schema5, 221) != 42) return false;
    for (size_t i = 0; i < 6; i++)
        if (be64(schema5, 229 + i * 8) != (i % 2 == 0 ? 10 * (i / 2 + 1) : 100 * (i / 2 + 1))) return false;
    Bytes unknown5 = p5c::playbackProgressPayloadSchema5(schema4, std::nullopt, std::nullopt, std::nullopt);
    if (unknown5.size() != 277) return false;
    for (size_t at = 229; at < 277; at += 8)
        if (be64(unknown5, at) != UINT64_MAX) return false;
    Bytes schema6 = p5c::playbackProgressPayloadSchema6(schema5, 1, 2, 3, 4, 5, 6);
    if (schema6.size() != 325 || schema6[78] != 6) return false;
    for (size_t i = 0; i < 6; i++)
        if (be64(schema6, 277 + i * 8) != i + 1) return false;
    // Wrong stage -> refused, as the Java IllegalArgumentException.
    return p5c::playbackProgressPayloadSchema4(schema2, 1).empty() && p5c::playbackProgressPayloadSchema6(schema4, 1, 2, 3, 4, 5, 6).empty();
}

// Java's default progress (P5cReceiverCoordinator) sends -1 / null for every unknown field: all ones on the wire,
// which the PC reads as unknown (src-tauri p5c/wire.rs metric_u32 / metric_u64).
bool progressUnknownFieldsAreAllOnes() {
    Bytes unknown = p5c::playbackProgressPayload(p5c::ProgressBase{});
    if (unknown.size() != 70 || std::any_of(unknown.begin(), unknown.end(), [](uint8_t b) { return b != 0xff; }))
        return false;
    p5c::ProgressBase known;
    known.sampleRate = 48000;
    known.channels = 2;
    known.pendingFrames = 7;
    Bytes b = p5c::playbackProgressPayload(known);
    return be32(b, 0) == 48000 && b[4] == 2 && be64(b, 25) == 7 && be32(b, 5) == 0xffffffffu &&
           be64(b, 9) == UINT64_MAX && be32(b, 46) == 0xffffffffu && be32(b, 66) == 0xffffffffu;
}

// Fixed frames: HMAC computed by Python hmac/hashlib over the layout in P5cControlProtocol.java.
bool p5cControlMatchesJavaVectors() {
    Key32 key = filled<32>(0x0a);
    constexpr uint64_t sid = 0x0102030405060708ULL;
    Bytes start = p5c::encode(key, p5c::kHostToReceiver, sid, 5, p5c::kStart, {});
    if (!eqHex(start, kStartFrame)) return false;
    auto decoded = p5c::decodeRaw(start, key, p5c::kHostToReceiver, sid);
    if (!decoded || decoded->type != p5c::kStart || decoded->sequence != 5 || !decoded->payload.empty()) return false;

    uint32_t caps = p5c::kCapAudioV2 | p5c::kCapAudioFloat32 | p5c::kCapProgressExt | p5c::kCapAudioAdbStream |
                    p5c::kCapBufferMetrics | p5c::kCapSourceFrameLoss;
    Bytes info = p5c::capabilitiesInfoPayload(7, 3, 1, 0, caps, 0x1234);
    if (caps != 0x7c80 || !eqHex(info, "00000000000000070003010000007c8000001234")) return false;
    Bytes infoFrame = p5c::encode(key, p5c::kReceiverToHost, sid, 9, p5c::kCapabilitiesInfo, info);
    if (!eqHex(infoFrame, "414e504301090000414e505201020304050607080000000000000009001400000000000000070003010000007c8"
                          "00000123464ad1b3665409e23f24bb8a6a1a64e00887ace1022c13c6dbaaa099e0198dd95"))
        return false;
    auto got = p5c::decodeRaw(infoFrame, key, p5c::kReceiverToHost, sid);
    if (!got || got->type != 9 || got->payload != info || info[11] != 0) return false;

    // Wrong key / direction / session / flipped payload bit / truncated frame are all refused.
    Bytes flipped = infoFrame;
    flipped[30] ^= 1;
    Bytes shortFrame(infoFrame.begin(), infoFrame.end() - 1);
    if (p5c::decodeRaw(infoFrame, filled<32>(0x0b), p5c::kReceiverToHost, sid) ||
        p5c::decodeRaw(infoFrame, key, p5c::kHostToReceiver, sid) || p5c::decodeRaw(infoFrame, key, p5c::kReceiverToHost, sid + 1) ||
        p5c::decodeRaw(flipped, key, p5c::kReceiverToHost, sid) || p5c::decodeRaw(shortFrame, key, p5c::kReceiverToHost, sid))
        return false;

    // Constants and small payloads (P5cControlProtocol.java / AoaReceiverCoordinatorTest.java).
    if (p5c::kCapabilitiesInfo != 9 || p5c::kPlaybackProgressQuery != 16 || p5c::kAck != 127 ||
        p5c::kResultUserStopped != 3 || p5c::kCapSourceSegmentStart != 0x10000 || p5c::kCapSourceLossDetails != 0x8000)
        return false;
    if (!eqHex(p5c::ackPayload(p5c::kStart, 3, 0, 2, 0), "03000000000000000300020000") ||
        !eqHex(p5c::capabilitiesQueryPayload(3, 1, 0x7c80), "0003010000007c80"))
        return false;
    Bytes big(p5c::kMaxFrameBytes - p5c::kHeaderBytes - p5c::kTagBytes + 1, 0);
    if (!p5c::encode(key, p5c::kHostToReceiver, sid, 0, p5c::kBind, big).empty()) return false;
    Bytes commitment(32, 9);
    if (p5c::backgroundTrustAckExtra(commitment, 2, 1, 1000).size() != 42 ||
        !p5c::backgroundTrustAckExtra(commitment, 2, 1, 8LL * 3600 * 1000 + 1).empty() ||
        p5c::backgroundAuthorizationAckExtra(Bytes(16, 1), 6).size() != 17 ||
        !p5c::backgroundAuthorizationAckExtra(Bytes(16, 1), 7).empty() || p5c::validBackgroundTrust(0, 0, 1))
        return false;
    return progressSchemasMatchJava();
}

Bytes seal(uint64_t sid, const Key32& key, uint64_t seq, uint8_t type, uint64_t frameIndex, const Bytes& plain) {
    Bytes head = {'A', 'N', 'P', 'D', 2, type, 0, 0};
    auto put64 = [&](uint64_t v) {
        for (int i = 7; i >= 0; i--) head.push_back(static_cast<uint8_t>(v >> (i * 8)));
    };
    put64(sid);
    put64(seq);
    put64(frameIndex);
    head.push_back(static_cast<uint8_t>(plain.size() >> 8));
    head.push_back(static_cast<uint8_t>(plain.size()));
    std::array<uint8_t, 12> nonce = {0x41, 0x4e, 0x50, 0x48};
    for (int i = 0; i < 8; i++) nonce[4 + i] = static_cast<uint8_t>(seq >> (56 - i * 8));
    Bytes sealed;
    aes256GcmSeal(key, nonce, head, plain, sealed);
    head.insert(head.end(), sealed.begin(), sealed.end());
    return head;
}

// Java-sealed (javax.crypto, 32 x 0x0b key, session 0x1122334455667788) so the C++ nonce/AAD layout is
// checked against the Android implementation rather than against itself.
constexpr const char* kJavaFormatPacket =
    "414e5044020100001122334455667788000000000000000000000000000000000010a5ea2ff5dbb4e4620279bac81bcedbb31dd8e6206e7"
    "9737025cddffb941f96f9";
constexpr const char* kJavaFloatPacket =
    "414e50440202000011223344556677880000000000000001000000000000000000209ed510a269a9f4e4a2f62c25f7e8cbc290c752025d4"
    "6ad30b9a7cf038ce21b1152dea704d499b4c6db9d58793b32559d";
constexpr const char* kJavaNonFinitePacket =
    "414e504402020000112233445566778800000000000000020000000000000004000827d2db3b29b4436c678e934126594189268f3aef5462c"
    "ae6";
constexpr uint64_t kSid = 0x1122334455667788ULL;
// FLOAT_BITS from src-tauri/src/p5c_tests.rs.
constexpr uint32_t kFloatBits[8] = {0x3f800000, 0xbf000000, 0x00000001, 0x80000000,
                                    0x7f7fffff, 0x80000001, 0x3eaaaaab, 0x00800000};

struct Capture {
    std::vector<uint32_t> bits;
    uint64_t firstFrame = 0;
    uint32_t frames = 0;
    bool accept = true;
    AnpdIngress::FloatSink sink() {
        return [this](uint64_t index, std::span<const float> samples, uint32_t n) {
            firstFrame = index;
            frames += n;
            for (float f : samples) {
                uint32_t b;
                std::memcpy(&b, &f, sizeof b);
                bits.push_back(b);
            }
            return accept;
        };
    }
};

std::unique_ptr<AnpdIngress> makeIngress(Capture& c, const Key32& key = filled<32>(0x0b)) {
    return AnpdIngress::create(kSid, key, *AudioFormatV2::forProfile(5, 1), c.sink());
}

bool anpdFloat32BitsExact() {
    Capture cap;
    auto ingress = makeIngress(cap);
    if (!ingress || AnpdIngress::create(kSid, filled<32>(0x0b), *AudioFormatV2::forProfile(1, 1), cap.sink()) ||
        AnpdIngress::create(kSid, filled<32>(0x0b), *AudioFormatV2::forProfile(3, 1), cap.sink()))
        return false;  // only Float32 profiles are accepted
    Bytes format = fromHex(kJavaFormatPacket);
    Bytes audio = fromHex(kJavaFloatPacket);
    if (ingress->accept(format) != AnpdIngress::Result::Format || ingress->accept(audio) != AnpdIngress::Result::Audio)
        return false;
    if (!ingress->aodSnapshot(ingress->lastAuthenticatedAudioAtNanos()).receiving) return false;
    if (cap.frames != 4 || cap.bits.size() != 8 || ingress->authenticatedAudioFrames() != 4 ||
        ingress->authenticatedAudioPayloadBytes() != 32 || !ingress->hasAuthenticatedAudio())
        return false;
    for (size_t i = 0; i < 8; i++)
        if (cap.bits[i] != kFloatBits[i]) return false;  // -0.0, denormals and max-finite survive bit for bit
    // An Inf-carrying packet is authenticated but rejected, and no frames are counted.
    if (ingress->accept(fromHex(kJavaNonFinitePacket)) != AnpdIngress::Result::FormatRejected ||
        ingress->authenticatedAudioFrames() != 4)
        return false;
    // A full sink is a counted drop, not an error.
    cap.accept = false;
    Bytes more = seal(kSid, filled<32>(0x0b), 3, kAnpdAudio, 4, Bytes(32, 0));
    return ingress->accept(more) == AnpdIngress::Result::Audio && ingress->droppedFrames() == 4 &&
           ingress->authenticatedAudioFrames() == 8;
}

bool anpdRejectsReplayAndWrongKey() {
    Key32 key = filled<32>(0x0b);
    Capture cap;
    auto ingress = makeIngress(cap);
    if (!ingress) return false;
    Bytes format = fromHex(kJavaFormatPacket);
    Bytes audio = fromHex(kJavaFloatPacket);
    // Audio before the format descriptor is refused and not committed.
    if (ingress->accept(audio) != AnpdIngress::Result::FormatRejected) return false;
    if (ingress->accept(format) != AnpdIngress::Result::Format || ingress->accept(format) != AnpdIngress::Result::Replay)
        return false;
    if (ingress->accept(audio) != AnpdIngress::Result::Audio || ingress->accept(audio) != AnpdIngress::Result::Replay)
        return false;
    // Out-of-order inside the 64-packet window is accepted once.
    Bytes pcm(16, 0);
    if (ingress->accept(seal(kSid, key, 5, kAnpdAudio, 100, pcm)) != AnpdIngress::Result::Audio ||
        ingress->accept(seal(kSid, key, 3, kAnpdAudio, 50, pcm)) != AnpdIngress::Result::Audio ||
        ingress->accept(seal(kSid, key, 3, kAnpdAudio, 50, pcm)) != AnpdIngress::Result::Replay ||
        ingress->accept(seal(kSid, key, 200, kAnpdAudio, 300, pcm)) != AnpdIngress::Result::Audio ||
        ingress->accept(seal(kSid, key, 5, kAnpdAudio, 100, pcm)) != AnpdIngress::Result::Replay ||  // now > 64 behind
        ingress->accept(seal(kSid, key, 4, kAnpdAudio, 100, pcm)) != AnpdIngress::Result::Replay)
        return false;

    // Wrong key, wrong session, tampered ciphertext / AAD / header, bad lengths and payload shape.
    Capture other;
    auto wrongKey = makeIngress(other, filled<32>(0x0c));
    Bytes tamperedBody = audio;
    tamperedBody[40] ^= 1;
    Bytes tamperedTag = audio;
    tamperedTag.back() ^= 1;
    Bytes tamperedSeq = audio;  // sequence is in the AAD and the nonce
    tamperedSeq[23] ^= 1;
    Bytes wrongSession = seal(kSid + 1, key, 6, kAnpdAudio, 0, pcm);
    Bytes badVersion = audio;
    badVersion[4] = 1;
    Bytes badType = audio;
    badType[5] = 9;
    Bytes badFlags = audio;
    badFlags[6] = 1;
    Bytes badMagic = audio;
    badMagic[0] = 'X';
    Bytes shortPacket(audio.begin(), audio.begin() + 49);
    Bytes oversize(1201, 0);
    Capture third;
    auto fresh = makeIngress(third);
    if (!wrongKey || !fresh) return false;
    fresh->accept(format);
    return wrongKey->accept(format) == AnpdIngress::Result::Authentication &&
           fresh->accept(tamperedBody) == AnpdIngress::Result::Authentication &&
           fresh->accept(tamperedTag) == AnpdIngress::Result::Authentication &&
           fresh->accept(tamperedSeq) == AnpdIngress::Result::Authentication &&
           fresh->accept(wrongSession) == AnpdIngress::Result::Identity &&
           fresh->accept(badVersion) == AnpdIngress::Result::HeaderRejected &&
           fresh->accept(badType) == AnpdIngress::Result::HeaderRejected &&
           fresh->accept(badFlags) == AnpdIngress::Result::HeaderRejected &&
           fresh->accept(badMagic) == AnpdIngress::Result::HeaderRejected &&
           fresh->accept(shortPacket) == AnpdIngress::Result::HeaderRejected &&
           fresh->accept(oversize) == AnpdIngress::Result::HeaderRejected &&
           fresh->accept(seal(kSid, key, 7, kAnpdAudio, 0, Bytes(12, 0))) == AnpdIngress::Result::FormatRejected &&  // not whole frames
           fresh->accept(seal(kSid, key, 8, kAnpdAudio, 0, {})) == AnpdIngress::Result::FormatRejected &&
           fresh->accept(seal(kSid, key, 9, kAnpdFormat, 0, Bytes(16, 0))) == AnpdIngress::Result::FormatRejected;  // wrong descriptor
}

bool lossLedgerAppliesInOrder() {
    Key32 key = filled<32>(0x0b);
    Capture cap;
    auto ingress = makeIngress(cap);
    if (!ingress) return false;
    LossLedger l;
    l.periodEpoch = 1;
    l.revision = 1;
    l.captureTail = 1000;
    l.settledThrough = 900;
    l.sourceSegmentStart = 10;
    l.hasSourceSegmentStart = true;
    l.complete = true;
    l.intervals = {{100, 20}, {300, 5}};
    auto roundtrip = LossLedger::decode(l.encode());
    if (!roundtrip || roundtrip->intervals.size() != 2 || roundtrip->intervals[1].start != 300 || l.encode().size() != 52 + 32)
        return false;
    // The ledger needs the format descriptor first.
    if (ingress->accept(seal(kSid, key, 1, kAnpdLossLedger, 0, l.encode())) != AnpdIngress::Result::FormatRejected) return false;
    ingress->accept(fromHex(kJavaFormatPacket));
    if (ingress->accept(seal(kSid, key, 1, kAnpdLossLedger, 0, l.encode())) != AnpdIngress::Result::LossLedger ||
        ingress->lossLedger().appliedRevision() != 1u || ingress->lossLedger().captureTail() != 1000u ||
        ingress->lossLedger().sourceSegmentStart() != 10u || !ingress->lossLedger().complete())
        return false;
    // Same revision with different content is a protocol violation; a newer one must keep earlier intervals.
    LossLedger changed = l;
    changed.captureTail = 1001;
    LossLedger dropsInterval = l;
    dropsInterval.revision = 2;
    dropsInterval.intervals = {{300, 5}};
    LossLedger next = l;
    next.revision = 2;
    next.captureTail = 1100;
    next.complete = false;
    next.intervals = {{100, 25}, {300, 5}};
    LossLedger staleEpoch = l;
    staleEpoch.periodEpoch = 0;
    staleEpoch.revision = 9;
    Bytes badFrameIndex = seal(kSid, key, 8, kAnpdLossLedger, 1, l.encode());
    return ingress->accept(seal(kSid, key, 2, kAnpdLossLedger, 0, changed.encode())) == AnpdIngress::Result::FormatRejected &&
           ingress->accept(seal(kSid, key, 3, kAnpdLossLedger, 0, dropsInterval.encode())) == AnpdIngress::Result::FormatRejected &&
           ingress->accept(seal(kSid, key, 4, kAnpdLossLedger, 0, next.encode())) == AnpdIngress::Result::LossLedger &&
           ingress->lossLedger().appliedRevision() == 2u && !ingress->lossLedger().complete() &&
           ingress->accept(seal(kSid, key, 5, kAnpdLossLedger, 0, staleEpoch.encode())) == AnpdIngress::Result::LossLedger &&
           ingress->lossLedger().appliedRevision() == 2u &&  // an older epoch is ignored
           ingress->accept(badFrameIndex) == AnpdIngress::Result::FormatRejected &&
           !LossLedger::decode(Bytes(35, 0)) && ingress->lossLedger().intervals().size() == 2;
}

bool floatRingWrapsWithoutLoss() {
    SpscFloatRing ring;
    if (ring.initialize(0, 2) || ring.initialize(8, 0) || !ring.initialize(8, 2)) return false;
    uint32_t nextWrite = 0, nextRead = 0;
    float in[2 * 5], out[2 * 5];
    // 1-5 frame chunks over an 8-frame ring cross the physical end many times.
    for (int round = 0; round < 200; round++) {
        uint32_t want = 1 + static_cast<uint32_t>(round) % 5;
        for (uint32_t f = 0; f < want; f++) {
            in[f * 2] = static_cast<float>(nextWrite + f);
            in[f * 2 + 1] = -static_cast<float>(nextWrite + f);
        }
        uint32_t wrote = ring.write(in, want);
        nextWrite += wrote;
        uint32_t take = 1 + static_cast<uint32_t>(round * 3) % 5;
        uint32_t got = ring.render(out, take, false, 1.0f);
        for (uint32_t f = 0; f < got; f++) {
            if (out[f * 2] != static_cast<float>(nextRead) || out[f * 2 + 1] != -static_cast<float>(nextRead)) return false;
            nextRead++;
        }
        for (uint32_t f = got; f < take; f++)
            if (out[f * 2] != 0.0f || out[f * 2 + 1] != 0.0f) return false;  // underrun is silence
        if (ring.pendingFrames() != nextWrite - nextRead) return false;
    }
    // Full ring accepts only what fits; muted render yields silence and keeps the queued audio.
    ring.resetAfterQuiescence();
    float eight[16];
    for (float& f : eight) f = 1.0f;
    if (ring.write(eight, 8) != 8 || ring.write(eight, 1) != 0 || ring.render(out, 2, true, 1.0f) != 0 ||
        out[0] != 0.0f || ring.pendingFrames() != 8 || ring.render(out, 2, false, 0.5f) != 2 || out[3] != 0.5f)
        return false;
    // Buffer budget (copied verbatim from the Android receiver).
    return requestedStreamBufferFrames(4096, 8192, 2048, 0) == 4096 &&
           requestedStreamBufferFrames(4096, 8192, 8192, 0) == -1 && actualBridgeRingFrames(8192, 2048, 0, 4096, 480) == 2048 &&
           actualBridgeRingFrames(8192, 2048, 0, 8192, 480) == -1;
}

}  // namespace

std::vector<std::string> runSelfTests() {
    struct Case {
        const char* name;
        bool (*fn)();
    };
    static constexpr Case cases[] = {
        {"aod_samples_are_session_scoped", aodSamplesAreSessionScoped},
        {"discovery_offer_matches_desktop", offerMatchesDesktop},
        {"discovery_matches_android", discoveryMatchesAndroid},
        {"hmac_rfc4231_case2", hmacRfc4231Case2},
        {"aes256gcm_roundtrip_and_tamper", aes256GcmRoundtripAndTamper},
        {"derive_adb_stream_vector", deriveAdbStreamVector},
        {"binding_encode_decode_roundtrip", bindingEncodeDecodeRoundtrip},
        {"pairing_code_vector", pairingCodeVector},
        {"v2_handshake_against_java_transcript", v2HandshakeAgainstJavaTranscript},
        {"anpu_rejects_bad_magic_version_and_oversize", anpuRejectsBadMagicVersionAndOversize},
        {"anhb_roundtrip_all_kinds", anhbRoundtripAllKinds},
        {"anhb_rejects_unknown_kind_and_oversize", anhbRejectsUnknownKindAndOversize},
        {"p5c_control_matches_java_vectors", p5cControlMatchesJavaVectors},
        {"progress_unknown_fields_are_all_ones", progressUnknownFieldsAreAllOnes},
        {"anpd_float32_bits_exact", anpdFloat32BitsExact},
        {"anpd_rejects_replay_and_wrong_key", anpdRejectsReplayAndWrongKey},
        {"anpd_loss_ledger_applies_in_order", lossLedgerAppliesInOrder},
        {"float_ring_wraps_without_loss", floatRingWrapsWithoutLoss},
    };
    std::vector<std::string> failures;
    for (const Case& c : cases)
        if (!c.fn()) failures.emplace_back(c.name);
    runSessionSelfTests(failures);
    return failures;
}

}  // namespace anpod
