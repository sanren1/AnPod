// Port of AnpuRecordCodec.java: one complete ANPU record = "ANPU" | 1 | type | body length u16 BE | body.
// Pure codec: transport reads stay with the ByteReader. After any failure that consumed part of a record the
// stream is session-fatal; callers must not resynchronize it.
#pragma once

#include "byte_reader.h"
#include "crypto.h"

#include <optional>

namespace anpod {

inline constexpr size_t kAnpuHeaderBytes = 8;
inline constexpr size_t kAnpuMaxBodyBytes = 1200;

enum class AnpuType : uint8_t { Handshake = 1, Control = 2, Audio = 3 };

struct AnpuRecord {
    AnpuType type;
    Bytes body;
};

// Empty result when the body is empty, over 1200 bytes or malformed for its type.
Bytes encodeAnpu(AnpuType type, std::span<const uint8_t> body);
// nullopt on a short read, bad magic/version/type/length, or a body malformed for its type.
std::optional<AnpuRecord> readAnpu(ByteReader& in);

}  // namespace anpod
