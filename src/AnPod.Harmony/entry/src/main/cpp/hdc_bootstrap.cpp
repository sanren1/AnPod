#include "hdc_bootstrap.h"

#include "anhb_codec.h"
#include "binding_store.h"
#include "crypto.h"

#include <cstring>
#include <hilog/log.h>
#include <deque>
#include <optional>
#include <thread>

namespace anpod {
namespace {

constexpr uint8_t kCancelUser = 1, kCancelTimeout = 2;

}  // namespace

TabletBootstrap::TabletBootstrap(ByteStream& stream, Token16 launchToken, Id16 hostId, BootstrapUi ui,
                                 FindBinding findBinding)
    : stream_(stream), token_(launchToken), hostId_(hostId), ui_(std::move(ui)), findBinding_(std::move(findBinding)) {}

void TabletBootstrap::answerPairing(bool accept) {
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!codeShown_ || localAnswer_ == 0) return;
        localAnswer_ = accept ? 1 : 0;
    }
    changed_.notify_all();
}

BootstrapOutcome TabletBootstrap::run() {
    const auto deadline = std::chrono::steady_clock::now() + kTimeout;
    enum class Phase { AwaitFirst, Pairing, AwaitProceed } phase = Phase::AwaitFirst;

    // PC frames arrive through one reader thread so a local answer or the deadline can interrupt the wait.
    // It stops after Proceed / PairCancel / a read failure, so it never consumes the ANPU records that follow.
    std::deque<std::optional<AnhbFrame>> frames;
    std::thread reader;
    auto finish = [&](BootstrapOutcome outcome) {
        if (!std::holds_alternative<BootstrapProceed>(outcome)) {
            stream_.close();
            // Saved, but the PC may not have stored its side: leave no half pairing behind.
            if (phase == Phase::AwaitProceed && !BindingStore::remove(hostId_))
                OH_LOG_Print(LOG_APP, LOG_WARN, 0xA0D0, "AnPod", "unconfirmed pairing could not be removed");
        }
        if (reader.joinable()) reader.join();
        return outcome;
    };
    auto send = [&](AnhbKind kind, std::span<const uint8_t> body) { return stream_.writeAll(encodeAnhb(kind, body)); };
    auto cancel = [&](uint8_t reason) { send(AnhbKind::PairCancel, std::span<const uint8_t>(&reason, 1)); };

    std::optional<BindingMaterial> saved;
    if (!findBinding_(hostId_, saved)) return finish(BootstrapFailed{"binding store unavailable"});
    uint8_t hello[17];
    std::memcpy(hello, token_.data(), 16);
    hello[16] = saved ? 1 : 0;
    if (!send(AnhbKind::TabletHello, hello)) return finish(BootstrapFailed{"bootstrap write failed"});
    reader = std::thread([&] {
        for (;;) {
            std::optional<AnhbFrame> frame = readAnhb(stream_);
            bool last = !frame || frame->kind == AnhbKind::Proceed || frame->kind == AnhbKind::PairCancel;
            {
                std::lock_guard<std::mutex> lock(mutex_);
                frames.push_back(std::move(frame));
            }
            changed_.notify_all();
            if (last) return;
        }
    });

    std::optional<BindingMaterial> offered;
    bool pcConfirmed = false;
    for (;;) {
        std::optional<AnhbFrame> frame;
        bool haveFrame = false;
        int answer;
        {
            std::unique_lock<std::mutex> lock(mutex_);
            bool timedOut = !changed_.wait_until(lock, deadline, [&] {
                return !frames.empty() ||
                       (phase == Phase::Pairing && (localAnswer_ == 0 || (localAnswer_ == 1 && pcConfirmed)));
            });
            answer = localAnswer_;
            if (timedOut) {
                codeShown_ = false;
                lock.unlock();
                if (phase != Phase::AwaitFirst) cancel(kCancelTimeout);
                return finish(BootstrapFailed{"pairing timed out"});
            }
            if (!frames.empty()) {
                frame = std::move(frames.front());
                frames.pop_front();
                haveFrame = true;
            }
        }
        if (phase == Phase::Pairing && answer == 0) {
            cancel(kCancelUser);
            return finish(BootstrapCancelled{});
        }
        if (haveFrame) {
            if (!frame) return finish(BootstrapFailed{"bootstrap stream closed"});
            if (frame->kind == AnhbKind::PairCancel) return finish(BootstrapCancelled{});
            if (phase == Phase::AwaitFirst && frame->kind == AnhbKind::Proceed) {
                if (!saved) return finish(BootstrapFailed{"proceed without a stored binding"});
                return finish(BootstrapProceed{std::move(*saved)});
            } else if (phase == Phase::AwaitFirst && frame->kind == AnhbKind::PairOffer) {
                // The ability is exported: never let an offer replace a stored binding (the PC re-pairs under a new hostId).
                if (saved) return finish(BootstrapFailed{"pairing offer for an already paired PC"});
                std::span<const uint8_t> body = frame->body;
                Nonce16 pcNonce, tabletNonce;
                std::memcpy(pcNonce.data(), body.data() + BindingMaterial::kBytes, 16);
                offered = BindingMaterial::decode(body.first<BindingMaterial::kBytes>());
                secureZero(frame->body.data(), frame->body.size());
                if (!offered || offered->hostId() != hostId_) {
                    cancel(kCancelUser);
                    return finish(BootstrapFailed{"pairing offer is not for this PC"});
                }
                secureRandom(tabletNonce);
                if (!send(AnhbKind::PairNonce, tabletNonce)) return finish(BootstrapFailed{"bootstrap write failed"});
                {
                    std::lock_guard<std::mutex> lock(mutex_);
                    codeShown_ = true;
                }
                phase = Phase::Pairing;
                if (ui_.showCode) ui_.showCode(offered->pairingCode(pcNonce, tabletNonce));
            } else if (phase == Phase::Pairing && frame->kind == AnhbKind::PairConfirmed && !pcConfirmed) {
                pcConfirmed = true;
                if (ui_.codeConfirmedByPc) ui_.codeConfirmedByPc();
            } else if (phase == Phase::AwaitProceed && frame->kind == AnhbKind::Proceed) {
                return finish(BootstrapProceed{std::move(*offered)});
            } else {
                if (phase != Phase::AwaitFirst) cancel(kCancelUser);
                return finish(BootstrapFailed{"unexpected bootstrap frame"});
            }
        }
        {
            std::lock_guard<std::mutex> lock(mutex_);
            answer = localAnswer_;
        }
        if (phase == Phase::Pairing && pcConfirmed && answer == 1) {
            {
                std::lock_guard<std::mutex> lock(mutex_);
                codeShown_ = false;
            }
            if (!BindingStore::save(*offered)) {
                cancel(kCancelUser);
                return finish(BootstrapFailed{"binding could not be saved"});
            }
            phase = Phase::AwaitProceed;  // from here any failure removes the saved binding again
            if (!send(AnhbKind::PairSaved, offered->commitment()))
                return finish(BootstrapFailed{"bootstrap write failed"});
        }
    }
}

}  // namespace anpod
