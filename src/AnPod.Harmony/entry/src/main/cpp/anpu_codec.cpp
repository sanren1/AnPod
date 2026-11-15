#include "anpu_codec.h"

#include "anpd_ingress.h"
#include "p5c_control.h"

namespace anpod {
namespace {

bool validHandshake(std::span<const uint8_t> b) {
    if (b.size() < 8 || b[0] != 'A' || b[1] != 'N' || b[2] != 'P' || b[3] != '5' || b[4] != 2) return false;
    size_t declared = (static_cast<size_t>(b[6]) << 8) | b[7];
    size_t expected;
    switch (b[5]) {
        case 1: expected = 96; break;
        case 2: expected = 104; break;
        case 3:
        case 4: expected = 40; break;
        default: return false;
    }
    return b.size() == expected && declared == expected;
}

bool validControl(std::span<const uint8_t> b) {
    if (b.size() < p5c::kHeaderBytes + p5c::kTagBytes || b[0] != 'A' || b[1] != 'N' || b[2] != 'P' || b[3] != 'C' ||
        b[4] != 1 || b[6] != 0 || b[7] != 0)
        return false;
    size_t payload = (static_cast<size_t>(b[28]) << 8) | b[29];
    return b.size() == p5c::kHeaderBytes + payload + p5c::kTagBytes;
}

bool validAudio(std::span<const uint8_t> b) {
    if (b.size() < kAnpdHeaderBytes + kAnpdTagBytes || b.size() > kAnpdMaxDatagramBytes || b[0] != 'A' ||
        b[1] != 'N' || b[2] != 'P' || b[3] != 'D' || b[4] != 2 ||
        (b[5] != kAnpdFormat && b[5] != kAnpdAudio && b[5] != kAnpdLossLedger) || b[6] != 0 || b[7] != 0)
        return false;
    size_t payload = (static_cast<size_t>(b[32]) << 8) | b[33];
    return payload <= kAnpdMaxPlaintextBytes && b.size() == kAnpdHeaderBytes + payload + kAnpdTagBytes &&
           !(b[5] == kAnpdFormat && payload != 16);
}

bool validBody(uint8_t type, std::span<const uint8_t> body) {
    if (body.empty() || body.size() > kAnpuMaxBodyBytes) return false;
    switch (type) {
        case 1: return validHandshake(body);
        case 2: return validControl(body);
        case 3: return validAudio(body);
        default: return false;
    }
}

}  // namespace

Bytes encodeAnpu(AnpuType type, std::span<const uint8_t> body) {
    if (!validBody(static_cast<uint8_t>(type), body)) return {};
    Bytes record = {'A', 'N', 'P', 'U', 1, static_cast<uint8_t>(type), static_cast<uint8_t>(body.size() >> 8),
                    static_cast<uint8_t>(body.size())};
    record.insert(record.end(), body.begin(), body.end());
    return record;
}

std::optional<AnpuRecord> readAnpu(ByteReader& in) {
    uint8_t header[kAnpuHeaderBytes];
    if (!in.readExact(header, sizeof header)) return std::nullopt;
    if (header[0] != 'A' || header[1] != 'N' || header[2] != 'P' || header[3] != 'U' || header[4] != 1 ||
        header[5] < 1 || header[5] > 3)
        return std::nullopt;
    size_t length = (static_cast<size_t>(header[6]) << 8) | header[7];
    if (length < 1 || length > kAnpuMaxBodyBytes) return std::nullopt;
    Bytes body(length);
    if (!in.readExact(body.data(), length) || !validBody(header[5], body)) return std::nullopt;
    return AnpuRecord{static_cast<AnpuType>(header[5]), std::move(body)};
}

}  // namespace anpod
