// On-device self-tests, run by ohosTest CoreSelfTest through the NAPI selfTest() export.
// Expected vectors come from Python hmac/hashlib and from PairingProtocol.java's own *ForVector helpers
// (see .superpowers/sdd/2026-11-14-harmonyos-m1-wired/findings.md), never from this C++ code.
#include "crypto.h"
#include "pairing_protocol.h"

#include <cstring>
#include <string>
#include <vector>

namespace anpod {
namespace {

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

}  // namespace

std::vector<std::string> runSelfTests() {
    struct Case {
        const char* name;
        bool (*fn)();
    };
    static constexpr Case cases[] = {
        {"hmac_rfc4231_case2", hmacRfc4231Case2},
        {"aes256gcm_roundtrip_and_tamper", aes256GcmRoundtripAndTamper},
        {"derive_adb_stream_vector", deriveAdbStreamVector},
        {"binding_encode_decode_roundtrip", bindingEncodeDecodeRoundtrip},
        {"pairing_code_vector", pairingCodeVector},
        {"v2_handshake_against_java_transcript", v2HandshakeAgainstJavaTranscript},
    };
    std::vector<std::string> failures;
    for (const Case& c : cases)
        if (!c.fn()) failures.emplace_back(c.name);
    return failures;
}

}  // namespace anpod
