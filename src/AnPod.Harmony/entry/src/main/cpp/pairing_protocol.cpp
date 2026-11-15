#include "pairing_protocol.h"

#include "crypto.h"

#include <cstdio>
#include <cstring>

namespace anpod {
namespace {

constexpr uint8_t kMagic[4] = {'A', 'N', 'P', '5'};
constexpr uint8_t kBindingMagic[4] = {'A', 'N', 'P', 'B'};
constexpr uint8_t kV2 = 2, kHostRole = 1, kReceiverRole = 2;
constexpr uint8_t kHello = 1, kChallenge = 2, kFinish = 3, kReady = 4;

std::span<const uint8_t> ascii(const char* s) { return {reinterpret_cast<const uint8_t*>(s), std::strlen(s)}; }

void append(Bytes& out, std::span<const uint8_t> part) { out.insert(out.end(), part.begin(), part.end()); }

template <size_t N> void wipe(std::array<uint8_t, N>& a) { secureZero(a.data(), N); }
void wipe(Bytes& b) { secureZero(b.data(), b.size()); }

// RFC 5869 HKDF with SHA-256 for one 32-byte block: PRK = HMAC(salt, ikm); OKM = HMAC(PRK, info | 0x01).
Key32 hkdf32(const Digest& salt, std::span<const uint8_t> ikm, const char* info) {
    Digest prk = hmacSha256(salt, ikm);
    Bytes expand;
    append(expand, ascii(info));
    expand.push_back(1);
    Key32 okm = hmacSha256(prk, expand);
    wipe(prk);
    return okm;
}

// PairingProtocol.mac: HMAC(secret, label | 0x00 | transcript | extra).
Digest labeledMac(const Key32& secret, const char* label, std::span<const uint8_t> transcript,
                  std::span<const uint8_t> extra) {
    Bytes in;
    append(in, ascii(label));
    in.push_back(0);
    append(in, transcript);
    append(in, extra);
    Digest mac = hmacSha256(secret, in);
    wipe(in);
    return mac;
}

template <size_t N> std::array<uint8_t, N> message(uint8_t type, std::initializer_list<std::span<const uint8_t>> parts) {
    std::array<uint8_t, N> out{};
    std::memcpy(out.data(), kMagic, 4);
    out[4] = kV2;
    out[5] = type;
    out[6] = static_cast<uint8_t>(N >> 8);
    out[7] = static_cast<uint8_t>(N & 0xff);
    size_t at = 8;
    for (auto part : parts) {
        std::memcpy(out.data() + at, part.data(), part.size());
        at += part.size();
    }
    return out;
}

// Returns the body (after the 8-byte header) or an empty span when the header/length is wrong.
std::span<const uint8_t> parse(std::span<const uint8_t> message, uint8_t type, size_t length) {
    if (message.size() != length || std::memcmp(message.data(), kMagic, 4) != 0 || message[4] != kV2 ||
        message[5] != type || ((size_t{message[6]} << 8) | message[7]) != length)
        return {};
    return message.subspan(8);
}

void putU64(Bytes& out, uint64_t v) {
    for (int shift = 56; shift >= 0; shift -= 8) out.push_back(static_cast<uint8_t>(v >> shift));
}

}  // namespace

BindingMaterial::BindingMaterial(const Id16& hostId, const Id16& receiverId, const Key32& secret)
    : hostId_(hostId), receiverId_(receiverId), secret_(secret) {}

BindingMaterial::~BindingMaterial() {
    wipe(hostId_);
    wipe(receiverId_);
    wipe(secret_);
}

std::optional<BindingMaterial> BindingMaterial::decode(std::span<const uint8_t, kBytes> bytes) {
    if (std::memcmp(bytes.data(), kBindingMagic, 4) != 0 || bytes[4] != 1) return std::nullopt;
    Id16 host, receiver;
    Key32 secret;
    std::memcpy(host.data(), bytes.data() + 5, 16);
    std::memcpy(receiver.data(), bytes.data() + 21, 16);
    std::memcpy(secret.data(), bytes.data() + 37, 32);
    BindingMaterial material(host, receiver, secret);
    wipe(secret);
    return material;
}

std::array<uint8_t, BindingMaterial::kBytes> BindingMaterial::encode() const {
    std::array<uint8_t, kBytes> out;
    std::memcpy(out.data(), kBindingMagic, 4);
    out[4] = 1;
    std::memcpy(out.data() + 5, hostId_.data(), 16);
    std::memcpy(out.data() + 21, receiverId_.data(), 16);
    std::memcpy(out.data() + 37, secret_.data(), 32);
    return out;
}

std::array<uint8_t, 32> BindingMaterial::commitment() const {
    auto record = encode();
    Digest d = sha256(record);
    wipe(record);
    return d;
}

BindingMaterial BindingMaterial::deriveAdbStream() const {
    Bytes saltInput;
    append(saltInput, ascii("AnPod ADB Stream binding salt v1"));
    saltInput.push_back(0);
    append(saltInput, hostId_);
    append(saltInput, receiverId_);
    Digest salt = sha256(saltInput);
    Key32 derived = hkdf32(salt, secret_, "AnPod ADB Stream P5-A binding secret v1");
    BindingMaterial out(hostId_, receiverId_, derived);
    wipe(salt);
    wipe(derived);
    return out;
}

std::string BindingMaterial::pairingCode(const Nonce16& pc, const Nonce16& tablet) const {
    Bytes in;
    append(in, ascii("AnPod Harmony pairing code v1"));
    append(in, pc);
    append(in, tablet);
    Digest mac = hmacSha256(secret_, in);
    uint32_t value = uint32_t{mac[0]} << 24 | uint32_t{mac[1]} << 16 | uint32_t{mac[2]} << 8 | mac[3];
    wipe(mac);
    char code[7];
    std::snprintf(code, sizeof code, "%06u", value % 1000000u);
    return code;
}

std::optional<AudioFormatV2> AudioFormatV2::forProfile(int profile, int mode) {
    if (profile < 1 || profile > 5 || mode < 0 || mode > 1) return std::nullopt;
    AudioFormatV2 f;
    f.profile = static_cast<uint8_t>(profile);
    f.mode = static_cast<uint8_t>(mode);
    f.channels = 2;
    if (profile == 1) {
        f.sampleFormat = 1, f.validBits = 16, f.bytesPerSample = 2, f.sampleRate = 48000, f.frameBytes = 4;
    } else if (profile <= 3) {
        f.sampleFormat = 2, f.validBits = 24, f.bytesPerSample = 3, f.frameBytes = 6;
        f.sampleRate = profile == 3 ? 96000 : 48000;
    } else {
        f.sampleFormat = 3, f.validBits = 32, f.bytesPerSample = 4, f.frameBytes = 8;
        f.sampleRate = profile == 5 ? 96000 : 48000;
    }
    return f;
}

std::optional<AudioFormatV2> AudioFormatV2::decode(std::span<const uint8_t, 16> bytes) {
    if (bytes[0] != 2) return std::nullopt;
    auto f = forProfile(bytes[1], bytes[6]);
    // Like the Java decoder: only the canonical encoding of a known profile is accepted.
    if (!f || std::memcmp(f->encode().data(), bytes.data(), 16) != 0) return std::nullopt;
    return f;
}

std::array<uint8_t, 16> AudioFormatV2::encode() const {
    return {2,
            profile,
            sampleFormat,
            channels,
            validBits,
            bytesPerSample,
            mode,
            0,
            static_cast<uint8_t>(sampleRate >> 24),
            static_cast<uint8_t>(sampleRate >> 16),
            static_cast<uint8_t>(sampleRate >> 8),
            static_cast<uint8_t>(sampleRate),
            static_cast<uint8_t>(frameBytes >> 8),
            static_cast<uint8_t>(frameBytes),
            0,
            0};
}

SessionKeys::~SessionKeys() {
    wipe(hostToReceiverAudio);
    wipe(receiverToHostAudio);
    wipe(hostToReceiverControl);
    wipe(receiverToHostControl);
}

namespace v2 {

ReceiverAwaitFinish::ReceiverAwaitFinish(const BindingMaterial& material, const AudioFormatV2& format,
                                         uint64_t sessionId, std::span<const uint8_t, 32> hostNonce,
                                         const Nonce32& receiverNonce)
    : material_(material), format_(format), sessionId_(sessionId) {
    auto descriptor = format.encode();
    Bytes t;
    append(t, kMagic);
    t.insert(t.end(), {kV2, kHostRole, kReceiverRole});
    append(t, material_.hostId_);
    append(t, material_.receiverId_);
    append(t, hostNonce);
    append(t, receiverNonce);
    putU64(t, sessionId);
    append(t, descriptor);  // requested
    append(t, descriptor);  // selected (the receiver always selects the request)
    std::memcpy(transcript_.data(), t.data(), transcript_.size());
    receiverProof_ = labeledMac(material_.secret_, "AnPod P5-A proof receiver v2", transcript_, {});
    challenge_ = message<104>(kChallenge, {material_.receiverId_, receiverNonce, descriptor, receiverProof_});
}

ReceiverAwaitFinish::~ReceiverAwaitFinish() {
    wipe(transcript_);
    wipe(receiverProof_);
}

std::variant<FinishResult, Failure> ReceiverAwaitFinish::acceptFinish(std::span<const uint8_t> finish) {
    if (consumed_) return Failure::Consumed;
    consumed_ = true;
    std::span<const uint8_t> hostProof = parse(finish, kFinish, 40);
    std::variant<FinishResult, Failure> result = Failure::Malformed;
    if (!hostProof.empty()) {
        Digest expected = labeledMac(material_.secret_, "AnPod P5-A proof host v2", transcript_, {});
        if (!constantTimeEqual(expected, hostProof)) {
            result = Failure::InvalidProof;
        } else {
            Bytes proofs;
            append(proofs, hostProof);
            append(proofs, receiverProof_);
            Digest ready = labeledMac(material_.secret_, "AnPod P5-A ready v2", transcript_, proofs);

            Bytes saltInput;
            append(saltInput, ascii("AnPod P5-A HKDF-SHA256 salt v2"));
            saltInput.push_back(0);
            append(saltInput, transcript_);
            Digest salt = sha256(saltInput);
            FinishResult done{message<40>(kReady, {ready}), {}, format_, sessionId_};
            const Key32& s = material_.secret_;
            done.keys.hostToReceiverAudio = hkdf32(salt, s, "AnPod P5-A key host-to-receiver audio AES-256-GCM v2");
            done.keys.receiverToHostAudio = hkdf32(salt, s, "AnPod P5-A key receiver-to-host audio AES-256-GCM v2");
            done.keys.hostToReceiverControl =
                hkdf32(salt, s, "AnPod P5-A key host-to-receiver control HMAC-SHA256 v2");
            done.keys.receiverToHostControl =
                hkdf32(salt, s, "AnPod P5-A key receiver-to-host control HMAC-SHA256 v2");
            result = std::move(done);
            wipe(proofs);
            wipe(ready);
            wipe(saltInput);
            wipe(salt);
        }
        wipe(expected);
    }
    // Like the Java receiver: the binding secret is closed after the one finish attempt.
    material_ = BindingMaterial(Id16{}, Id16{}, Key32{});
    return result;
}

std::variant<ReceiverAwaitFinish, Failure> acceptHello(const BindingMaterial& material,
                                                       std::span<const uint8_t> hello) {
    Nonce32 nonce;
    secureRandom(nonce);
    return acceptHelloWithNonce(material, hello, nonce);
}

std::variant<ReceiverAwaitFinish, Failure> acceptHelloWithNonce(const BindingMaterial& material,
                                                                std::span<const uint8_t> hello,
                                                                const Nonce32& receiverNonce) {
    std::span<const uint8_t> body = parse(hello, kHello, 96);
    if (body.empty()) return Failure::Malformed;
    if (!constantTimeEqual(body.first(16), material.hostId()) ||
        !constantTimeEqual(body.subspan(16, 16), material.receiverId()))
        return Failure::WrongIdentity;
    uint64_t sessionId = 0;
    for (size_t i = 64; i < 72; i++) sessionId = sessionId << 8 | body[i];
    auto requested = AudioFormatV2::decode(body.subspan(72).first<16>());
    if (!requested) return Failure::InvalidParameters;
    return ReceiverAwaitFinish(material, *requested, sessionId, body.subspan(32).first<32>(), receiverNonce);
}

}  // namespace v2

}  // namespace anpod
