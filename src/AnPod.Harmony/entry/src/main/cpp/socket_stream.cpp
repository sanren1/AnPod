#include "socket_stream.h"

#include <arpa/inet.h>
#include <cerrno>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>

namespace anpod {

std::unique_ptr<SocketStream> SocketStream::connectLoopback(uint16_t port) {
    int fd = ::socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return nullptr;
    sockaddr_in address{};
    address.sin_family = AF_INET;
    address.sin_port = htons(port);
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int result;
    do {
        result = ::connect(fd, reinterpret_cast<const sockaddr*>(&address), sizeof address);
    } while (result != 0 && errno == EINTR);
    if (result != 0) {
        ::close(fd);
        return nullptr;
    }
    int one = 1;
    ::setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);  // control ACKs are small and latency-bound
    return std::unique_ptr<SocketStream>(new SocketStream(fd));
}

SocketStream::~SocketStream() {
    close();
    ::close(fd_);
}

bool SocketStream::readExact(uint8_t* out, size_t count) {
    while (count > 0) {
        if (closed_) return false;
        ssize_t n = ::recv(fd_, out, count, 0);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) return false;
        out += n;
        count -= static_cast<size_t>(n);
    }
    return !closed_;
}

bool SocketStream::writeAll(std::span<const uint8_t> data) {
    while (!data.empty()) {
        if (closed_) return false;
        ssize_t n = ::send(fd_, data.data(), data.size(), MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) return false;
        data = data.subspan(static_cast<size_t>(n));
    }
    return true;
}

void SocketStream::close() {
    if (!closed_.exchange(true)) ::shutdown(fd_, SHUT_RDWR);
}

}  // namespace anpod
