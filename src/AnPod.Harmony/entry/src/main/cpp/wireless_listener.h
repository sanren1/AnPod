#pragma once

#include "pairing_protocol.h"
#include <memory>

namespace anpod {

class WirelessListener {
public:
    static std::unique_ptr<WirelessListener> open();
    ~WirelessListener();
    WirelessListener(const WirelessListener&) = delete;
    uint16_t port() const { return port_; }
    int accept(int timeoutMs) const;

private:
    WirelessListener(int fd, uint16_t port) : fd_(fd), port_(port) {}
    int fd_;
    uint16_t port_;
};

// Routing hint only. P5-A authenticates this identity after selecting its binding.
std::optional<Id16> wirelessHelloHost(int fd);

}  // namespace anpod
