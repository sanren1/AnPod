#pragma once

#include "pairing_protocol.h"

#include <atomic>
#include <chrono>
#include <mutex>
#include <thread>
#include <vector>

namespace anpod {

std::optional<std::array<uint8_t, 50>> authenticatedCandidate(const BindingMaterial& binding,
    std::span<const uint8_t> probe, uint16_t controlPort);

std::optional<std::array<uint8_t, 26>> connectionOffer(std::span<const uint8_t> candidate,
    const Nonce16& intent, uint16_t controlPort);

// Lifecycle calls are serialized by the receiver. Stop joins the UDP worker.
class WirelessDiscovery {
public:
    ~WirelessDiscovery();
    bool start(uint16_t controlPort);
    bool request(const Id16& selectedHost, const Nonce16& intent);
    void stop();

private:
    void serve();
    int socket_ = -1;
    uint16_t port_ = 0;
    std::atomic<bool> running_{false};
    std::thread worker_;
    std::vector<BindingMaterial> bindings_;
    std::mutex intentMutex_;
    Nonce16 intent_{};
    std::chrono::steady_clock::time_point expires_{};
    std::vector<uint32_t> offered_;
    std::vector<uint32_t> searchAddresses_;
};

}  // namespace anpod
