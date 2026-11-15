#include "socket_stream.h"

#include <arpa/inet.h>
#include <cerrno>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
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
    int result = ::connect(fd, reinterpret_cast<const sockaddr*>(&address), sizeof address);
    if (result != 0 && errno == EINTR) {
        // The connect goes on in the background after EINTR (calling it again fails): wait for its outcome.
        pollfd waiter{fd, POLLOUT, 0};
        int ready;
        do {
            ready = ::poll(&waiter, 1, 5000);
        } while (ready < 0 && errno == EINTR);
        int error = 0;
        socklen_t length = sizeof error;
        result = ready == 1 && ::getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &length) == 0 && error == 0 ? 0 : -1;
    }
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
