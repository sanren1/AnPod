#include "wireless_listener.h"
#include "wireless_discovery.h"

#include <algorithm>
#include <cerrno>
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
    if (fd < 0) {
        ANPOD_WIRELESS_LOG("listener socket failed", errno);
        return {};
    }
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    socklen_t size = sizeof addr;
    if (::bind(fd, reinterpret_cast<sockaddr*>(&addr), size) != 0 || ::listen(fd, 8) != 0 ||
        ::getsockname(fd, reinterpret_cast<sockaddr*>(&addr), &size) != 0) {
        ANPOD_WIRELESS_LOG("listener open failed", errno);
        ::close(fd);
        return {};
    }
    return std::unique_ptr<WirelessListener>(new WirelessListener(fd, ntohs(addr.sin_port)));
}

WirelessListener::~WirelessListener() { ::close(fd_); }

int WirelessListener::accept(int timeoutMs) const {
    pollfd p{fd_, POLLIN, 0};
    if (::poll(&p, 1, timeoutMs) != 1 || !(p.revents & POLLIN)) return -1;
    int fd = ::accept4(fd_, nullptr, nullptr, SOCK_CLOEXEC);
    if (fd < 0) return -1;
    // Only peers on a private LAN of this device, like discovery (the listener itself binds all interfaces).
    sockaddr_in local{}, peer{};
    socklen_t localSize = sizeof local, peerSize = sizeof peer;
    if (::getsockname(fd, reinterpret_cast<sockaddr*>(&local), &localSize) != 0 ||
        ::getpeername(fd, reinterpret_cast<sockaddr*>(&peer), &peerSize) != 0 || local.sin_family != AF_INET ||
        !onPrivateLan(local.sin_addr.s_addr, peer.sin_addr.s_addr)) {
        ANPOD_WIRELESS_LOG("connection off the local subnet", 0);
        ::close(fd);
        return -1;
    }
    return fd;
}

std::optional<Id16> wirelessHelloHost(int fd) {
    uint8_t bytes[24];
    constexpr uint8_t header[] = {'A','N','P','5',2,1,0,96};
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    do {
        ssize_t n = ::recv(fd, bytes, sizeof bytes, MSG_PEEK | MSG_DONTWAIT);
        if (n == 0 || (n < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) {
            ANPOD_WIRELESS_LOG("hello read failed", n < 0 ? errno : 0);
            return {};
        }
        if (n > 0 && std::memcmp(bytes, header, std::min<size_t>(n, sizeof header))) {
            ANPOD_WIRELESS_LOG("hello rejected", n);
            return {};
        }
        if (n == sizeof bytes) {
            Id16 id{};
            std::copy_n(bytes + 8, id.size(), id.begin());
            return id;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    } while (std::chrono::steady_clock::now() < deadline);
    ANPOD_WIRELESS_LOG("hello timed out", 0);
    return {};
}

}  // namespace anpod
