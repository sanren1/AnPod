// ByteStream over the loopback TCP socket that `hdc rport` forwards to the PC.
#pragma once

#include "byte_reader.h"

#include <atomic>
#include <memory>

namespace anpod {

class SocketStream final : public ByteStream {
public:
    // Connects to 127.0.0.1:port; nullptr on failure.
    static std::unique_ptr<SocketStream> connectLoopback(uint16_t port);
    explicit SocketStream(int fd) : fd_(fd) {}  // takes ownership
    ~SocketStream() override;
    SocketStream(const SocketStream&) = delete;
    SocketStream& operator=(const SocketStream&) = delete;

    bool readExact(uint8_t* out, size_t count) override;
    bool writeAll(std::span<const uint8_t> data) override;
    // shutdown(SHUT_RDWR) unblocks every pending read / write at once. The descriptor itself is released in
    // the destructor, so a thread still inside recv / send can never hit a reused descriptor number.
    void close() override;

private:
    const int fd_;
    std::atomic<bool> closed_{false};
};

}  // namespace anpod
