#pragma once

#include "pairing_protocol.h"
#include <atomic>
#include <chrono>
#include <hilog/log.h>
#include <memory>

namespace anpod {

// Wireless diagnostics: an event name and an error code only (no addresses, ids, nonces or keys), at most one line
// per call site every 10 s.
#define ANPOD_WIRELESS_LOG(event, code)                                                                          \
    do {                                                                                                        \
        static std::atomic<int64_t> lastLog{INT64_MIN / 2};                                                     \
        int64_t nowS = std::chrono::duration_cast<std::chrono::seconds>(                                        \
                           std::chrono::steady_clock::now().time_since_epoch()).count();                        \
        if (nowS - lastLog.load() >= 10) {                                                                      \
            lastLog = nowS;                                                                                     \
            OH_LOG_Print(LOG_APP, LOG_INFO, 0xA0D0, "AnPodWireless", "%{public}s (%{public}d)", event,          \
                         static_cast<int>(code));                                                               \
        }                                                                                                       \
    } while (0)

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
