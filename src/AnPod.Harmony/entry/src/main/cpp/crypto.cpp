#include "crypto.h"

#include <CryptoArchitectureKit/crypto_architecture_kit.h>

#include <cstdlib>
#include <cstring>

namespace anpod {
namespace {

Crypto_DataBlob blob(std::span<const uint8_t> b) {
    return Crypto_DataBlob{const_cast<uint8_t*>(b.data()), b.size()};
}

// The NDK only fails here on allocation or a broken system service: never continue with garbage output.
void require(OH_Crypto_ErrCode rc) {
    if (rc != CRYPTO_SUCCESS) std::abort();
}

void appendAndFree(Crypto_DataBlob& b, Bytes& out) {
    if (b.data) {
        out.insert(out.end(), b.data, b.data + b.len);
        secureZero(b.data, b.len);
    }
    OH_Crypto_FreeDataBlob(&b);
}

Digest takeDigest(Crypto_DataBlob& b) {
    if (b.len != 32) std::abort();
    Digest d;
    std::memcpy(d.data(), b.data, 32);
    secureZero(b.data, b.len);
    OH_Crypto_FreeDataBlob(&b);
    return d;
}

// Runs AES-256-GCM. For decrypt, `tag` is the received tag; encrypt needs a dummy tag param or Init fails.
bool gcm(Crypto_CipherMode mode, std::span<const uint8_t, 32> key, std::span<const uint8_t, 12> nonce,
         std::span<const uint8_t> aad, std::span<const uint8_t> in, std::span<const uint8_t, 16> tag, Bytes& out) {
    OH_CryptoSymKeyGenerator* gen = nullptr;
    OH_CryptoSymKey* k = nullptr;
    OH_CryptoSymCipher* cipher = nullptr;
    OH_CryptoSymCipherParams* params = nullptr;
    Crypto_DataBlob kb = blob(key), ivb = blob(nonce), aadb = blob(aad), tagb = blob(tag), inb = blob(in);
    Crypto_DataBlob update{}, final{};
    require(OH_CryptoSymKeyGenerator_Create("AES256", &gen));
    require(OH_CryptoSymKeyGenerator_Convert(gen, &kb, &k));
    require(OH_CryptoSymCipher_Create("AES256|GCM|NoPadding", &cipher));
    require(OH_CryptoSymCipherParams_Create(&params));
    require(OH_CryptoSymCipherParams_SetParam(params, CRYPTO_IV_DATABLOB, &ivb));
    if (!aad.empty()) require(OH_CryptoSymCipherParams_SetParam(params, CRYPTO_AAD_DATABLOB, &aadb));
    require(OH_CryptoSymCipherParams_SetParam(params, CRYPTO_TAG_DATABLOB, &tagb));
    require(OH_CryptoSymCipher_Init(cipher, mode, k, params));
    out.clear();
    bool ok = in.empty() || OH_CryptoSymCipher_Update(cipher, &inb, &update) == CRYPTO_SUCCESS;
    // Decrypt reports a bad tag / AAD from Final (CRYPTO_OPERTION_ERROR); that is a normal "reject".
    ok = ok && OH_CryptoSymCipher_Final(cipher, nullptr, &final) == CRYPTO_SUCCESS;
    appendAndFree(update, out);
    appendAndFree(final, out);
    OH_CryptoSymCipherParams_Destroy(params);
    OH_CryptoSymCipher_Destroy(cipher);
    OH_CryptoSymKey_Destroy(k);
    OH_CryptoSymKeyGenerator_Destroy(gen);
    if (!ok) {
        secureZero(out.data(), out.size());
        out.clear();
    }
    return ok;
}

}  // namespace

Digest sha256(std::span<const uint8_t> data) {
    OH_CryptoDigest* d = nullptr;
    Crypto_DataBlob in = blob(data), out{};
    require(OH_CryptoDigest_Create("SHA256", &d));
    if (!data.empty()) require(OH_CryptoDigest_Update(d, &in));
    require(OH_CryptoDigest_Final(d, &out));
    OH_DigestCrypto_Destroy(d);
    return takeDigest(out);
}

Digest hmacSha256(std::span<const uint8_t> key, std::span<const uint8_t> data) {
    OH_CryptoSymKeyGenerator* gen = nullptr;
    OH_CryptoSymKey* k = nullptr;
    OH_CryptoMac* mac = nullptr;
    static const uint8_t kSha256[] = {'S', 'H', 'A', '2', '5', '6'};
    Crypto_DataBlob kb = blob(key), name = blob(kSha256), in = blob(data), out{};
    require(OH_CryptoSymKeyGenerator_Create("HMAC", &gen));
    require(OH_CryptoSymKeyGenerator_Convert(gen, &kb, &k));
    require(OH_CryptoMac_Create("HMAC", &mac));
    require(OH_CryptoMac_SetParam(mac, CRYPTO_MAC_DIGEST_NAME_STR, &name));
    require(OH_CryptoMac_Init(mac, k));
    if (!data.empty()) require(OH_CryptoMac_Update(mac, &in));
    require(OH_CryptoMac_Final(mac, &out));
    OH_CryptoMac_Destroy(mac);
    OH_CryptoSymKey_Destroy(k);
    OH_CryptoSymKeyGenerator_Destroy(gen);
    return takeDigest(out);
}

bool aes256GcmSeal(std::span<const uint8_t, 32> key, std::span<const uint8_t, 12> nonce,
                   std::span<const uint8_t> aad, std::span<const uint8_t> plaintext, Bytes& out) {
    static const uint8_t kDummyTag[16] = {};
    return gcm(CRYPTO_ENCRYPT_MODE, key, nonce, aad, plaintext, kDummyTag, out) && out.size() == plaintext.size() + 16;
}

bool aes256GcmOpen(std::span<const uint8_t, 32> key, std::span<const uint8_t, 12> nonce,
                   std::span<const uint8_t> aad, std::span<const uint8_t> ciphertextAndTag, Bytes& out) {
    out.clear();
    if (ciphertextAndTag.size() < 16) return false;
    size_t n = ciphertextAndTag.size() - 16;
    return gcm(CRYPTO_DECRYPT_MODE, key, nonce, aad, ciphertextAndTag.first(n),
               ciphertextAndTag.subspan(n).first<16>(), out);
}

void secureRandom(std::span<uint8_t> out) {
    if (out.empty()) return;
    OH_CryptoRand* rnd = nullptr;
    Crypto_DataBlob b{};
    require(OH_CryptoRand_Create(&rnd));
    require(OH_CryptoRand_GenerateRandom(rnd, static_cast<int>(out.size()), &b));
    if (b.len != out.size()) std::abort();
    std::memcpy(out.data(), b.data, out.size());
    secureZero(b.data, b.len);
    OH_Crypto_FreeDataBlob(&b);
    OH_CryptoRand_Destroy(rnd);
}

void secureZero(void* data, size_t size) {
    volatile uint8_t* p = static_cast<volatile uint8_t*>(data);
    while (size--) *p++ = 0;
}

bool constantTimeEqual(std::span<const uint8_t> a, std::span<const uint8_t> b) {
    if (a.size() != b.size()) return false;
    uint8_t diff = 0;
    for (size_t i = 0; i < a.size(); i++) diff |= a[i] ^ b[i];
    return diff == 0;
}

}  // namespace anpod
