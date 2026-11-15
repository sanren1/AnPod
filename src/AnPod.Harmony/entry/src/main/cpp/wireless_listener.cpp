#include "wireless_listener.h"

#include <algorithm>
#include <chrono>
#include <cstring>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <thread>
#include <unistd.h>

namespace anpod {

std::unique_ptr<WirelessListener> WirelessListener::open() {
    int fd = ::socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return {};
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    socklen_t size = sizeof addr;
    if (::bind(fd, reinterpret_cast<sockaddr*>(&addr), size) != 0 || ::listen(fd, 8) != 0 ||
        ::getsockname(fd, reinterpret_cast<sockaddr*>(&addr), &size) != 0) {
        ::close(fd);
        return {};
    }
    return std::unique_ptr<WirelessListener>(new WirelessListener(fd, ntohs(addr.sin_port)));
}

WirelessListener::~WirelessListener() { ::close(fd_); }

int WirelessListener::accept(int timeoutMs) const {
    pollfd p{fd_, POLLIN, 0};
    if (::poll(&p, 1, timeoutMs) != 1 || !(p.revents & POLLIN)) return -1;
    return ::accept4(fd_, nullptr, nullptr, SOCK_CLOEXEC);
}

std::optional<Id16> wirelessHelloHost(int fd) {
    uint8_t bytes[24];
    constexpr uint8_t header[] = {'A','N','P','5',2,1,0,96};
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    do {
        ssize_t n = ::recv(fd, bytes, sizeof bytes, MSG_PEEK | MSG_DONTWAIT);
        if (n == 0 || (n < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) return {};
        if (n > 0 && std::memcmp(bytes, header, std::min<size_t>(n, sizeof header))) return {};
        if (n == sizeof bytes) {
            Id16 id{};
            std::copy_n(bytes + 8, id.size(), id.begin());
            return id;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    } while (std::chrono::steady_clock::now() < deadline);
    return {};
}

}  // namespace anpod
