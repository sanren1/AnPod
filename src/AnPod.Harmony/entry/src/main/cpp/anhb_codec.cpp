#include "anhb_codec.h"

namespace anpod {
namespace {

constexpr uint8_t kVersion = 1;

// Exact body length per kind; the Rust host sends and expects these sizes.
std::optional<size_t> bodyLength(uint8_t kind) {
    switch (kind) {
        case 1: return 17;
        case 2: return 85;
        case 3: return 16;
        case 4: return 0;
        case 5: return 32;
        case 6: return 1;
        case 7: return 0;
        default: return std::nullopt;
    }
}

}  // namespace

Bytes encodeAnhb(AnhbKind kind, std::span<const uint8_t> body) {
    auto want = bodyLength(static_cast<uint8_t>(kind));
    if (!want || *want != body.size() || body.size() > kAnhbMaxBody) return {};
    Bytes frame = {'A', 'N', 'H', 'B', kVersion, static_cast<uint8_t>(kind),
                   static_cast<uint8_t>(body.size() >> 8), static_cast<uint8_t>(body.size())};
    frame.insert(frame.end(), body.begin(), body.end());
    return frame;
}

std::optional<AnhbFrame> readAnhb(ByteReader& in) {
    uint8_t header[kAnhbHeaderBytes];
    if (!in.readExact(header, sizeof header)) return std::nullopt;
    if (header[0] != 'A' || header[1] != 'N' || header[2] != 'H' || header[3] != 'B' || header[4] != kVersion)
        return std::nullopt;
    size_t length = (static_cast<size_t>(header[6]) << 8) | header[7];
    auto want = bodyLength(header[5]);
    if (length > kAnhbMaxBody || !want || *want != length) return std::nullopt;
    AnhbFrame frame{static_cast<AnhbKind>(header[5]), Bytes(length)};
    if (length != 0 && !in.readExact(frame.body.data(), length)) return std::nullopt;
    return frame;
}

}  // namespace anpod
