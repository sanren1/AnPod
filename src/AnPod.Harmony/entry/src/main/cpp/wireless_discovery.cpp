#include "wireless_discovery.h"
#include "crypto.h"

#include <algorithm>
#include <arpa/inet.h>
#include <cerrno>
#include <cstring>
#include <ifaddrs.h>
#include <net/if.h>
#include <poll.h>
#include <string_view>
#include <sys/socket.h>
#include <unistd.h>

#include "binding_store.h"

namespace anpod {
namespace {
using Clock = std::chrono::steady_clock;
constexpr uint16_t kPort = 49772;
bool header(std::span<const uint8_t> bytes, uint8_t version, uint8_t kind) {
    return bytes.size() >= 24 && std::memcmp(bytes.data(), "ANPD", 4) == 0 && bytes[4] == version &&
           bytes[5] == kind && bytes[6] == 0 && bytes[7] == 0 &&
           std::any_of(bytes.begin() + 8, bytes.begin() + 24, [](uint8_t b) { return b != 0; });
}

struct Lan { uint32_t address, mask; };
std::vector<Lan> networks() {
    ifaddrs* interfaces = nullptr;
    std::vector<Lan> result;
    if (getifaddrs(&interfaces) != 0) return result;
    for (auto* it = interfaces; it; it = it->ifa_next) {
        if (!it->ifa_addr || !it->ifa_netmask || it->ifa_addr->sa_family != AF_INET ||
            !(it->ifa_flags & IFF_UP) || !(it->ifa_flags & IFF_BROADCAST)) continue;
        auto addr = reinterpret_cast<sockaddr_in*>(it->ifa_addr)->sin_addr.s_addr;
        auto ip = ntohl(addr);
        if (ip >> 24 != 10 && ip >> 20 != 0xac1 && ip >> 16 != 0xc0a8) continue;
        auto mask = reinterpret_cast<sockaddr_in*>(it->ifa_netmask)->sin_addr.s_addr;
        if (mask && ntohl(mask) <= 0xfffffffc) result.push_back({addr, mask});
    }
    freeifaddrs(interfaces);
    return result;
}

sockaddr_in endpoint(uint32_t address, uint16_t port) {
    sockaddr_in target{};
    target.sin_family = AF_INET;
    target.sin_addr.s_addr = address;
    target.sin_port = htons(port);
    return target;
}

bool sendPacket(int fd, std::span<const uint8_t> bytes, const sockaddr_in& dest) {
    return sendto(fd, bytes.data(), bytes.size(), 0, reinterpret_cast<const sockaddr*>(&dest), sizeof(dest)) ==
           static_cast<ssize_t>(bytes.size());
}
}  // namespace

std::optional<std::array<uint8_t, 50>> authenticatedCandidate(const BindingMaterial& binding,
    std::span<const uint8_t> probe, uint16_t controlPort) {
    if (probe.size() != 50 || !header(probe, 3, 6) || probe[24] || probe[25] || !controlPort) return {};
    auto record = binding.encode();
    auto secret = std::span(record).last<32>();
    auto mac = [&](std::string_view label, bool withNonce, bool withPort) {
        Bytes input(label.begin(), label.end());
        if (withNonce) {
            input.insert(input.end(), probe.begin() + 8, probe.begin() + 24);
            input.insert(input.end(), probe.begin() + 26, probe.begin() + 34);
        }
        if (withPort) {
            input.push_back(static_cast<uint8_t>(controlPort >> 8));
            input.push_back(static_cast<uint8_t>(controlPort));
        }
        return hmacSha256(secret, input);
    };
    auto tag = mac("anpod-discovery-tag-v3", false, false);
    std::optional<std::array<uint8_t, 50>> response;
    if (constantTimeEqual(std::span(tag).first<8>(), probe.subspan(26, 8))) {
        auto expected = mac("anpod-probe-v3", true, false);
        if (constantTimeEqual(std::span(expected).first<16>(), probe.last<16>())) {
            auto& out = response.emplace();
            std::copy_n(probe.begin(), 34, out.begin());
            out[5] = 7;
            out[24] = static_cast<uint8_t>(controlPort >> 8);
            out[25] = static_cast<uint8_t>(controlPort);
            auto signature = mac("anpod-candidate-v3", true, true);
            std::copy_n(signature.begin(), 16, out.begin() + 34);
        }
    }
    secureZero(record.data(), record.size());
    return response;
}

std::optional<std::array<uint8_t, 26>> connectionOffer(std::span<const uint8_t> candidate,
    const Nonce16& intent, uint16_t controlPort) {
    if (candidate.size() != 26 || !header(candidate, 2, 4) ||
        ((candidate[24] << 8) | candidate[25]) != kPort ||
        !std::equal(intent.begin(), intent.end(), candidate.begin() + 8) || !controlPort) return {};
    std::array<uint8_t, 26> offer{};
    std::copy_n(candidate.begin(), 24, offer.begin());
    offer[5] = 5;
    offer[24] = static_cast<uint8_t>(controlPort >> 8);
    offer[25] = static_cast<uint8_t>(controlPort);
    return offer;
}

WirelessDiscovery::~WirelessDiscovery() { stop(); }

bool WirelessDiscovery::start(uint16_t controlPort) {
    stop();
    auto ids = BindingStore::list();
    if (!ids || ids->empty() || !controlPort) return false;
    std::vector<BindingMaterial> bindings;
    for (const auto& id : *ids) {
        std::optional<BindingMaterial> material;
        if (!BindingStore::find(id, material)) return false;
        if (material) bindings.push_back(*material);
    }
    if (bindings.empty()) return false;
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) return false;
    int enabled = 1;
    auto address = endpoint(INADDR_ANY, kPort);
    if (setsockopt(fd, SOL_SOCKET, SO_BROADCAST, &enabled, sizeof(enabled)) < 0 ||
        setsockopt(fd, IPPROTO_IP, IP_PKTINFO, &enabled, sizeof(enabled)) < 0 ||
        bind(fd, reinterpret_cast<sockaddr*>(&address), sizeof(address)) < 0) {
        close(fd);
        return false;
    }
    socket_ = fd;
    port_ = controlPort;
    bindings_ = std::move(bindings);
    running_ = true;
    worker_ = std::thread(&WirelessDiscovery::serve, this);
    return true;
}

bool WirelessDiscovery::request(const Id16& selectedHost, const Nonce16& intent) {
    if (!running_ || intent == Nonce16{} || std::none_of(bindings_.begin(), bindings_.end(),
        [&](const auto& binding) { return binding.hostId() == selectedHost; })) return false;
    std::array<uint8_t, 24> probe{'A','N','P','D',2,3};
    std::copy(intent.begin(), intent.end(), probe.begin() + 8);
    std::lock_guard lock(intentMutex_);
    expires_ = {};
    offered_.clear();
    searchAddresses_.clear();
    for (auto lan : networks()) {
        if (sendPacket(socket_, probe, endpoint(lan.address | ~lan.mask, kPort))) searchAddresses_.push_back(lan.address);
    }
    if (!searchAddresses_.empty()) {
        intent_ = intent;
        expires_ = Clock::now() + std::chrono::seconds(10);
    }
    return !searchAddresses_.empty();
}

void WirelessDiscovery::serve() {
    pollfd input{socket_, POLLIN, 0};
    auto nextAuth = Clock::time_point{};
    while (running_) {
        int ready = poll(&input, 1, 100);
        if (ready < 0 && errno == EINTR) continue;
        if (ready < 0 || (input.revents & (POLLNVAL | POLLERR | POLLHUP))) break;
        if (!ready) continue;
        std::array<uint8_t, 64> buffer{};
        sockaddr_in peer{};
        alignas(cmsghdr) char ancillary[CMSG_SPACE(sizeof(in_pktinfo))]{};
        iovec payload{buffer.data(), buffer.size()};
        msghdr message{};
        message.msg_name = &peer;
        message.msg_namelen = sizeof(peer);
        message.msg_iov = &payload;
        message.msg_iovlen = 1;
        message.msg_control = ancillary;
        message.msg_controllen = sizeof(ancillary);
        auto size = recvmsg(socket_, &message, 0);
        if (size != 26 && size != 50) continue;
        auto* info = CMSG_FIRSTHDR(&message);
        if (!info || info->cmsg_level != IPPROTO_IP || info->cmsg_type != IP_PKTINFO ||
            info->cmsg_len < CMSG_LEN(sizeof(in_pktinfo))) continue;
        in_pktinfo incoming{};
        std::memcpy(&incoming, CMSG_DATA(info), sizeof(incoming));
        auto local = incoming.ipi_spec_dst.s_addr;
        auto packet = std::span(buffer).first(static_cast<size_t>(size));
        auto lans = networks();
        if (std::none_of(lans.begin(), lans.end(), [&](Lan lan) {
            return lan.address == local && ((local ^ peer.sin_addr.s_addr) & lan.mask) == 0;
        })) continue;
        if (size == 50 && Clock::now() >= nextAuth) {
            for (const auto& binding : bindings_) {
                auto response = authenticatedCandidate(binding, packet, port_);
                if (!response) continue;
                sendPacket(socket_, *response, peer);
                nextAuth = Clock::now() + std::chrono::milliseconds(200);
                break;
            }
        } else if (size == 26) {
            std::lock_guard lock(intentMutex_);
            if (Clock::now() >= expires_ || offered_.size() >= 16 ||
                std::find(searchAddresses_.begin(), searchAddresses_.end(), local) == searchAddresses_.end() ||
                std::find(offered_.begin(), offered_.end(), peer.sin_addr.s_addr) != offered_.end()) continue;
            auto offer = connectionOffer(packet, intent_, port_);
            if (offer && sendPacket(socket_, *offer, endpoint(peer.sin_addr.s_addr, kPort)))
                offered_.push_back(peer.sin_addr.s_addr);
        }
    }
    running_ = false;
}

void WirelessDiscovery::stop() {
    running_ = false;
    if (worker_.joinable()) worker_.join();
    if (socket_ >= 0) close(socket_);
    socket_ = -1;
    bindings_.clear();
    expires_ = {};
    offered_.clear();
}
}  // namespace anpod
