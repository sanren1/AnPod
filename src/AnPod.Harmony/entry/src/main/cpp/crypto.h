// Thin wrappers over the HarmonyOS Crypto NDK. Every failure of the NDK aborts: callers never get
// zero-filled "keys" or a predictable random value.
#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

namespace anpod {

using Bytes = std::vector<uint8_t>;
using Digest = std::array<uint8_t, 32>;

Digest sha256(std::span<const uint8_t> data);
Digest hmacSha256(std::span<const uint8_t> key, std::span<const uint8_t> data);

// out = ciphertext || 16-byte tag.
bool aes256GcmSeal(std::span<const uint8_t, 32> key, std::span<const uint8_t, 12> nonce,
                   std::span<const uint8_t> aad, std::span<const uint8_t> plaintext, Bytes& out);
// Returns false (and clears out) on a short input, a wrong key/nonce/AAD or a tampered tag.
bool aes256GcmOpen(std::span<const uint8_t, 32> key, std::span<const uint8_t, 12> nonce,
                   std::span<const uint8_t> aad, std::span<const uint8_t> ciphertextAndTag, Bytes& out);

void secureRandom(std::span<uint8_t> out);
void secureZero(void* data, size_t size);
bool constantTimeEqual(std::span<const uint8_t> a, std::span<const uint8_t> b);

}  // namespace anpod
