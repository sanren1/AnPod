// All-or-nothing byte source for the framed codecs (ANPU records, ANHB frames). The socket-backed reader
// of the session code implements it; false means EOF, timeout or error, and the stream is then unusable.
#pragma once

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <span>

namespace anpod {

class ByteReader {
public:
    virtual ~ByteReader() = default;
    virtual bool readExact(uint8_t* out, size_t count) = 0;
};

// Full-duplex byte transport of the wired session (the rport TCP socket; an in-memory pipe in the self-tests).
// Timeouts are enforced by the owner calling close(), which must be thread-safe, idempotent and make every
// pending and later readExact / writeAll on any thread return false.
class ByteStream : public ByteReader {
public:
    virtual bool writeAll(std::span<const uint8_t> data) = 0;
    virtual void close() = 0;
};

// Reads from a fixed buffer; used by the self-tests.
class SpanReader final : public ByteReader {
public:
    explicit SpanReader(std::span<const uint8_t> data) : data_(data) {}
    bool readExact(uint8_t* out, size_t count) override {
        if (count > data_.size()) return false;
        if (count != 0) std::memcpy(out, data_.data(), count);
        data_ = data_.subspan(count);
        return true;
    }
    size_t remaining() const { return data_.size(); }

private:
    std::span<const uint8_t> data_;
};

}  // namespace anpod
