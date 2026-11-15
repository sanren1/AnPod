// Tablet side of the ANHB bootstrap on the hdc rport stream (peer: host_* in src-tauri/src/hdc_bootstrap.rs).
// TabletHello{launchToken, hasBinding} -> Proceed (stored binding), or the first-time pairing:
// PairOffer -> PairNonce, show the 6-digit code -> PairConfirmed from the PC AND a local answerPairing(true)
// -> BindingStore::save -> PairSaved{commitment} -> Proceed. Any cancel (either side) stores nothing.
#pragma once

#include "byte_reader.h"
#include "pairing_protocol.h"

#include <chrono>
#include <condition_variable>
#include <functional>
#include <mutex>
#include <string>
#include <variant>

namespace anpod {

using Token16 = std::array<uint8_t, 16>;

// Both are called on the run() thread.
struct BootstrapUi {
    std::function<void(const std::string& code)> showCode;
    std::function<void()> codeConfirmedByPc;
};

struct BootstrapProceed {
    BindingMaterial binding;
};
struct BootstrapCancelled {};
struct BootstrapFailed {
    std::string message;
};
using BootstrapOutcome = std::variant<BootstrapProceed, BootstrapCancelled, BootstrapFailed>;

class TabletBootstrap {
public:
    static constexpr std::chrono::seconds kTimeout{120};

    TabletBootstrap(ByteStream& stream, Token16 launchToken, Id16 hostId, BootstrapUi ui);
    // Blocks until the outcome; the whole bootstrap (pairing included) is bounded by kTimeout. On Proceed the
    // stream is positioned at the first ANPU record; otherwise it has been closed.
    BootstrapOutcome run();
    // Any thread. Only counts while a code is shown; false cancels the pairing.
    void answerPairing(bool accept);

private:
    ByteStream& stream_;
    const Token16 token_;
    const Id16 hostId_;
    const BootstrapUi ui_;
    std::mutex mutex_;
    std::condition_variable changed_;
    bool codeShown_ = false;
    int localAnswer_ = -1;  // -1 none, 0 reject, 1 accept
};

}  // namespace anpod
