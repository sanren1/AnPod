// ANHB bootstrap frame: "ANHB" | version 1 | kind u8 | body length u16 BE | body (<= 256).
// Byte-identical to encode_anhb / read_anhb in src-tauri/src/hdc_bootstrap.rs.
#pragma once

#include "byte_reader.h"
#include "crypto.h"

#include <optional>

namespace anpod {

inline constexpr size_t kAnhbHeaderBytes = 8;
inline constexpr size_t kAnhbMaxBody = 256;

enum class AnhbKind : uint8_t {
    TabletHello = 1,    // launchToken[16] | hasBinding u8
    PairOffer = 2,      // binding[69] | pcNonce[16]
    PairNonce = 3,      // tabletNonce[16]
    PairConfirmed = 4,  // empty
    PairSaved = 5,      // commitment[32]
    PairCancel = 6,     // reason u8: 1 user, 2 timeout
    Proceed = 7,        // empty
};

struct AnhbFrame {
    AnhbKind kind;
    Bytes body;
};

// Empty result when the kind is unknown or the body length is wrong for it.
Bytes encodeAnhb(AnhbKind kind, std::span<const uint8_t> body);
// nullopt on a short read, bad magic/version, oversize body, unknown kind or a body length wrong for its kind.
std::optional<AnhbFrame> readAnhb(ByteReader& in);

}  // namespace anpod
