// Port of the receiver side of src/AnPod.Receiver/.../PairingProtocol.java (ANPB binding, ADB-stream
// derivation, P5-A v2 handshake). Byte layouts match PairingProtocol.java and src-tauri/src/security.rs.
#pragma once

#include <array>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <variant>

namespace anpod {

using Id16 = std::array<uint8_t, 16>;
using Nonce16 = std::array<uint8_t, 16>;
using Nonce32 = std::array<uint8_t, 32>;
using Key32 = std::array<uint8_t, 32>;

namespace v2 {
class ReceiverAwaitFinish;
}

class BindingMaterial {
public:
    static constexpr size_t kBytes = 69;

    BindingMaterial(const Id16& hostId, const Id16& receiverId, const Key32& secret);
    BindingMaterial(const BindingMaterial&) = default;
    BindingMaterial(BindingMaterial&&) = default;
    BindingMaterial& operator=(const BindingMaterial&) = default;
    BindingMaterial& operator=(BindingMaterial&&) = default;
    ~BindingMaterial();

    // "ANPB" | 1 | hostId | receiverId | secret. The encoded record holds the secret: zero it after use.
    static std::optional<BindingMaterial> decode(std::span<const uint8_t, kBytes> bytes);
    std::array<uint8_t, kBytes> encode() const;

    const Id16& hostId() const { return hostId_; }
    const Id16& receiverId() const { return receiverId_; }
    std::array<uint8_t, 32> commitment() const;
    BindingMaterial deriveAdbStream() const;
    // HMAC(secret, "AnPod Harmony pairing code v1" | pc | tablet), first 4 bytes BE u32 % 1e6, 6 digits.
    std::string pairingCode(const Nonce16& pc, const Nonce16& tablet) const;

private:
    friend class v2::ReceiverAwaitFinish;
    Id16 hostId_;
    Id16 receiverId_;
    Key32 secret_;
};

struct AudioFormatV2 {
    uint8_t profile = 0, sampleFormat = 0, channels = 0, validBits = 0, bytesPerSample = 0, mode = 0;
    uint32_t sampleRate = 0;
    uint16_t frameBytes = 0;

    static std::optional<AudioFormatV2> forProfile(int profile, int mode);
    static std::optional<AudioFormatV2> decode(std::span<const uint8_t, 16> bytes);
    std::array<uint8_t, 16> encode() const;
};

struct SessionKeys {
    Key32 hostToReceiverAudio{};
    Key32 receiverToHostAudio{};
    Key32 hostToReceiverControl{};
    Key32 receiverToHostControl{};

    SessionKeys() = default;
    SessionKeys(const SessionKeys&) = default;
    SessionKeys(SessionKeys&&) = default;
    SessionKeys& operator=(const SessionKeys&) = default;
    SessionKeys& operator=(SessionKeys&&) = default;
    ~SessionKeys();
};

namespace v2 {

enum class Failure { Malformed, WrongIdentity, InvalidParameters, InvalidProof, Consumed };

struct FinishResult {
    std::array<uint8_t, 40> ready;
    SessionKeys keys;
    AudioFormatV2 format;
    uint64_t sessionId;
};

class ReceiverAwaitFinish {
public:
    ReceiverAwaitFinish(ReceiverAwaitFinish&&) = default;
    ReceiverAwaitFinish& operator=(ReceiverAwaitFinish&&) = default;
    ~ReceiverAwaitFinish();

    const std::array<uint8_t, 104>& challenge() const { return challenge_; }
    // One shot: the binding secret is wiped after the first call, whether it succeeds or not.
    std::variant<FinishResult, Failure> acceptFinish(std::span<const uint8_t> finish);

private:
    friend std::variant<ReceiverAwaitFinish, Failure> acceptHelloWithNonce(const BindingMaterial&,
                                                                           std::span<const uint8_t>,
                                                                           const Nonce32&);
    ReceiverAwaitFinish(const BindingMaterial& material, const AudioFormatV2& format, uint64_t sessionId,
                        std::span<const uint8_t, 32> hostNonce, const Nonce32& receiverNonce);

    BindingMaterial material_;
    AudioFormatV2 format_;
    uint64_t sessionId_;
    std::array<uint8_t, 143> transcript_{};
    Key32 receiverProof_{};
    std::array<uint8_t, 104> challenge_{};
    bool consumed_ = false;
};

std::variant<ReceiverAwaitFinish, Failure> acceptHello(const BindingMaterial& material,
                                                       std::span<const uint8_t> hello);
// Test seam: same as acceptHello with a fixed receiver nonce (fixture replay).
std::variant<ReceiverAwaitFinish, Failure> acceptHelloWithNonce(const BindingMaterial& material,
                                                                std::span<const uint8_t> hello,
                                                                const Nonce32& receiverNonce);

}  // namespace v2

}  // namespace anpod
