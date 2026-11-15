// NAPI bridge of libanpod_core.so (types: types/libanpod_core/index.d.ts). One wired run at a time: a worker
// thread connects to the rport loopback port, runs the ANHB bootstrap, then the StreamSession into OHAudio,
// and reports to ArkTS through a thread-safe function. Session callbacks only signal the worker; the worker
// destroys the session (never inside a callback).
#include "binding_store.h"
#include "hdc_bootstrap.h"
#include "ohaudio_output.h"
#include "socket_stream.h"
#include "stream_session.h"
#include "wireless_discovery.h"
#include "wireless_listener.h"
#include <unistd.h>

#include <napi/native_api.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

namespace anpod {
std::vector<std::string> runSelfTests();
bool selfTestWirelessPeer(const BindingMaterial& binding, const std::function<void(int)>& deliver,
                          const std::function<void()>& ready, int command, uint32_t& capabilities, int& result);
}

namespace {

using namespace anpod;

struct Event {
    const char* kind = "state";
    const char* state = nullptr;
    std::string code, message, host;  // host: hex hostId of the wireless peer, on state events
    uint32_t sampleRate = 0;
    uint32_t channels = 0;
    bool fast = false;
    bool pcConfirmed = false;
};

struct Run {
    napi_threadsafe_function events = nullptr;
    std::mutex mutex;
    ByteStream* stream = nullptr;         // set while the worker or session owns an open stream
    TabletBootstrap* bootstrap = nullptr;  // set while the bootstrap runs
    StreamSession* session = nullptr;      // set while the session runs
    std::atomic<bool> stopRequested{false};
    bool userStop = false;
    bool sessionEnded = false;
    std::string endMessage;
    std::condition_variable ended;
    std::atomic<bool> interrupted{false};
    std::atomic<int> sessionState{-1};
    bool finished = false;  // the worker reported its end (under mutex, signalled on ended)
};

std::mutex gMutex;
std::shared_ptr<Run> gRun;

// Events of a replaced run are dropped, so a late "stopped" never overwrites the new run's state.
void emit(const std::shared_ptr<Run>& run, Event event) {
    {
        std::lock_guard<std::mutex> lock(gMutex);
        if (run != gRun) return;
    }
    auto* heap = new Event(std::move(event));
    if (napi_call_threadsafe_function(run->events, heap, napi_tsfn_nonblocking) != napi_ok) delete heap;
}

void emitState(const std::shared_ptr<Run>& run, const char* state) {
    Event e;
    e.state = state;
    emit(run, std::move(e));
}

void emitEnd(const std::shared_ptr<Run>& run, const std::string& error) {
    {
        std::lock_guard<std::mutex> lock(run->mutex);
        run->finished = true;
    }
    run->ended.notify_all();
    if (!error.empty()) {
        Event e;
        e.kind = "error";
        e.message = error;
        emit(run, std::move(e));
    }
    emitState(run, error.empty() ? "stopped" : "failed");
}

// An audio interrupt shows as paused while the PC still streams.
const char* mapState(int state, bool interrupted) {
    switch (static_cast<SessionState>(state)) {
    case SessionState::Streaming: return interrupted ? "paused" : "playing";
    case SessionState::ReceiverRunning: return interrupted ? "paused" : "connected";
    case SessionState::Paused: return "paused";
    case SessionState::Stopped: return "stopped";
    default: return "connected";
    }
}

void setValue(napi_env env, napi_value object, const char* name, napi_value value) {
    napi_set_named_property(env, object, name, value);
}

void callJs(napi_env env, napi_value callback, void*, void* data) {
    std::unique_ptr<Event> e(static_cast<Event*>(data));
    if (!env || !callback) return;
    napi_value object, value;
    napi_create_object(env, &object);
    napi_create_string_utf8(env, e->kind, NAPI_AUTO_LENGTH, &value);
    setValue(env, object, "kind", value);
    if (e->state) {
        napi_create_string_utf8(env, e->state, NAPI_AUTO_LENGTH, &value);
        setValue(env, object, "state", value);
    }
    if (!e->code.empty()) {
        napi_create_string_utf8(env, e->code.c_str(), e->code.size(), &value);
        setValue(env, object, "code", value);
        napi_get_boolean(env, e->pcConfirmed, &value);
        setValue(env, object, "pcConfirmed", value);
    }
    if (!e->message.empty()) {
        napi_create_string_utf8(env, e->message.c_str(), e->message.size(), &value);
        setValue(env, object, "message", value);
    }
    if (!e->host.empty()) {
        napi_create_string_utf8(env, e->host.c_str(), e->host.size(), &value);
        setValue(env, object, "host", value);
    }
    if (e->sampleRate) {
        napi_create_uint32(env, e->sampleRate, &value);
        setValue(env, object, "sampleRate", value);
        napi_create_uint32(env, e->channels, &value);
        setValue(env, object, "channels", value);
        napi_create_string_utf8(env, "float32", NAPI_AUTO_LENGTH, &value);
        setValue(env, object, "encoding", value);
        napi_create_string_utf8(env, e->fast ? "fast" : "normal", NAPI_AUTO_LENGTH, &value);
        setValue(env, object, "latencyMode", value);
    }
    napi_value undefined, result;
    napi_get_undefined(env, &undefined);
    napi_call_function(env, undefined, callback, 1, &object, &result);
}

OhAudioOutput audioOutput(const std::shared_ptr<Run>& run) {
    return OhAudioOutput([run](bool interrupted) {
        run->interrupted = interrupted;
        if (run->sessionState >= 0) emitState(run, mapState(run->sessionState, interrupted));
    });
}

SessionCallbacks sessionCallbacks(const std::shared_ptr<Run>& run, OhAudioOutput& sink) {
    SessionCallbacks callbacks;
    callbacks.onState = [run](SessionState state) {
        run->sessionState = static_cast<int>(state);
        emitState(run, mapState(run->sessionState, run->interrupted));
    };
    callbacks.onFormat = [run, &sink](const AudioFormatV2& format) {
        Event e;
        e.kind = "format";
        e.sampleRate = format.sampleRate;
        e.channels = format.channels;
        e.fast = sink.fast();
        emit(run, std::move(e));
    };
    callbacks.onEnded = [run](const std::string& message) {
        std::lock_guard<std::mutex> lock(run->mutex);
        run->sessionEnded = true;
        run->endMessage = message;
        run->ended.notify_all();
    };
    return callbacks;
}

void runWired(std::shared_ptr<Run> run, uint16_t port, Id16 hostId, Token16 token) {
    emitState(run, "connecting");
    std::unique_ptr<SocketStream> stream = SocketStream::connectLoopback(port);
    std::string error;
    if (!stream) {
        error = "cannot reach the PC on the hdc forward";
    } else {
        std::string code;
        BootstrapUi ui;
        ui.showCode = [&](const std::string& shown) {
            code = shown;
            Event e;
            e.kind = "pairingCode";
            e.code = shown;
            emit(run, e);
            emitState(run, "pairing");
        };
        ui.codeConfirmedByPc = [&] {
            Event e;
            e.kind = "pairingCode";
            e.code = code;
            e.pcConfirmed = true;
            emit(run, std::move(e));
        };
        TabletBootstrap bootstrap(*stream, token, hostId, ui);
        {
            std::lock_guard<std::mutex> lock(run->mutex);
            run->stream = stream.get();
            run->bootstrap = &bootstrap;
            if (run->stopRequested) stream->close();
        }
        BootstrapOutcome outcome = bootstrap.run();
        bool userStop;
        {
            std::lock_guard<std::mutex> lock(run->mutex);
            run->bootstrap = nullptr;
            userStop = run->userStop;
        }
        if (auto* failed = std::get_if<BootstrapFailed>(&outcome)) {
            if (!userStop) error = failed->message;
        } else if (auto* proceed = std::get_if<BootstrapProceed>(&outcome)) {
            auto sink = audioOutput(run);
            auto callbacks = sessionCallbacks(run, sink);
            std::unique_ptr<StreamSession> session =
                StreamSession::start(std::move(stream), std::move(proceed->binding), sink, std::move(callbacks));
            std::unique_lock<std::mutex> lock(run->mutex);
            run->stream = nullptr;
            run->session = session.get();
            if (run->stopRequested) session->stop(run->userStop);
            run->ended.wait(lock, [&] { return run->sessionEnded; });
            run->session = nullptr;
            error = run->userStop ? "" : run->endMessage;  // a local Disconnect is a stop, not a failure
            lock.unlock();
            session.reset();
        }
    }
    {
        std::lock_guard<std::mutex> lock(run->mutex);
        run->stream = nullptr;
    }
    stream.reset();
    emitEnd(run, error);
    napi_release_threadsafe_function(run->events, napi_tsfn_release);
}

// Serialize the lifetime of the fixed discovery port during mode changes.
std::mutex wirelessOwner;

std::string toHex(const Id16& id);

// The session loop of a wireless run: `accept(timeoutMs)` yields the next connection (< 0 when none), `alive()`
// whether discovery still runs. Returns the run's terminal error, "" on a local stop.
std::string serveWireless(const std::shared_ptr<Run>& run, std::optional<Id16> selected,
                          std::optional<Nonce16> intent, const std::function<int(int)>& accept,
                          const std::function<bool()>& alive) {
    using Clock = std::chrono::steady_clock;
    const auto deadline = Clock::now() + std::chrono::seconds(10);
    bool established = false;
    std::optional<Clock::time_point> reconnectUntil;
    while (!run->stopRequested) {
        if (!alive()) return "wireless discovery unavailable";
        if (selected && !established && Clock::now() >= deadline) return "paired PC did not respond";
        if (reconnectUntil && Clock::now() >= *reconnectUntil) {
            // The PC gave up: stop showing reconnecting, release the continuous task, and the intent is spent.
            reconnectUntil.reset();
            selected.reset();
            intent.reset();
            emitState(run, "listening");
        }
        int fd = accept(150);
        if (fd < 0) continue;
        auto host = wirelessHelloHost(fd);
        std::optional<BindingMaterial> binding;
        if (!host || (selected && *host != *selected) || run->stopRequested) {
            close(fd);
            continue;
        }
        if (!BindingStore::find(*host, binding)) {
            close(fd);
            return "binding store unavailable";
        }
        if (!binding) { close(fd); continue; }
        auto sink = audioOutput(run);
        auto callbacks = sessionCallbacks(run, sink);
        // The peer is named only once P5-A has authenticated it (the hello hostId is a routing hint).
        callbacks.onState = [run, base = callbacks.onState, hex = toHex(*host)](SessionState state) {
            if (state != SessionState::Authenticated) return base(state);
            run->sessionState = static_cast<int>(state);
            Event e;
            e.state = mapState(run->sessionState, run->interrupted);
            e.host = hex;
            emit(run, std::move(e));
        };
        std::unique_lock lock(run->mutex);
        run->sessionState = -1;
        run->sessionEnded = false;
        run->interrupted = false;
        auto session = StreamSession::startWireless(fd, std::move(*binding), intent, sink, std::move(callbacks));
        run->session = session.get();
        if (run->stopRequested) session->stop(run->userStop);
        run->ended.wait(lock, [&] { return run->sessionEnded; });
        run->session = nullptr;
        auto error = run->endMessage;
        lock.unlock();
        session.reset();
        if (run->stopRequested) return {};
        if (error.empty()) {
            // The PC stopped (a manual disconnect, or the stop before a quality / mode change reconnect): keep
            // listening like Java's standby, but the old intent is spent (spec section 9).
            selected.reset();
            intent.reset();
            reconnectUntil.reset();
            emitState(run, "listening");
            continue;
        }
        if (run->sessionState < static_cast<int>(SessionState::Authenticated)) continue;  // UI never left waiting
        established |= run->sessionState >= static_cast<int>(SessionState::TransportConnected);
        if (error.starts_with(kOutputOpenError)) return error;
        // After an unexpected drop the PC reconnects the way it was connected; before that, keep waiting.
        if (established) reconnectUntil = Clock::now() + std::chrono::seconds(30);
        emitState(run, established ? "reconnecting" : selected ? "searching" : "listening");
    }
    return {};
}

std::string listenWireless(const std::shared_ptr<Run>& run, const std::optional<Id16>& selected) {
    std::unique_lock owner(wirelessOwner);
    if (run->stopRequested) return {};
    auto ids = BindingStore::list();
    if (!ids) return "binding store unavailable";
    if (ids->empty()) return {};  // nothing to listen for: the page shows that no PC is paired
    auto listener = WirelessListener::open();
    WirelessDiscovery discovery;
    if (!listener || !discovery.start(listener->port())) return "wireless discovery unavailable";
    std::optional<Nonce16> intent;
    if (selected) {
        secureRandom(intent.emplace());
        if (!discovery.request(*selected, *intent)) return "wireless discovery send failed";
    }
    emitState(run, selected ? "searching" : "listening");
    return serveWireless(
        run, selected, intent, [&](int timeoutMs) { return listener->accept(timeoutMs); },
        [&] { return discovery.running(); });
}

void runWireless(std::shared_ptr<Run> run, std::optional<Id16> selected) {
    auto error = listenWireless(run, selected);
    emitEnd(run, run->stopRequested ? "" : error);
    napi_release_threadsafe_function(run->events, napi_tsfn_release);
}

// ---- argument helpers ------------------------------------------------------------------------------------

std::vector<napi_value> args(napi_env env, napi_callback_info info, size_t count) {
    std::vector<napi_value> argv(count);
    size_t argc = count;
    napi_get_cb_info(env, info, &argc, argv.data(), nullptr, nullptr);
    if (argc < count) argv.clear();
    return argv;
}

bool stringArg(napi_env env, napi_value value, std::string& out) {
    char buffer[80];
    size_t length = 0;
    if (napi_get_value_string_utf8(env, value, buffer, sizeof buffer, &length) != napi_ok || length >= 79)
        return false;
    out.assign(buffer, length);
    return true;
}

template <size_t N> bool parseHex(const std::string& hex, std::array<uint8_t, N>& out) {
    if (hex.size() != N * 2) return false;
    auto nibble = [](char c) { return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : -1; };
    for (size_t i = 0; i < N; i++) {
        int hi = nibble(hex[i * 2]), lo = nibble(hex[i * 2 + 1]);
        if (hi < 0 || lo < 0) return false;
        out[i] = static_cast<uint8_t>(hi << 4 | lo);
    }
    return true;
}

std::string toHex(const Id16& id) {
    static const char* digits = "0123456789abcdef";
    std::string hex;
    for (uint8_t b : id) {
        hex.push_back(digits[b >> 4]);
        hex.push_back(digits[b & 15]);
    }
    return hex;
}

napi_value undefinedValue(napi_env env) {
    napi_value value;
    napi_get_undefined(env, &value);
    return value;
}

napi_value stringArray(napi_env env, const std::vector<std::string>& items) {
    napi_value out;
    napi_create_array_with_length(env, items.size(), &out);
    for (size_t i = 0; i < items.size(); i++) {
        napi_value item;
        napi_create_string_utf8(env, items[i].c_str(), items[i].size(), &item);
        napi_set_element(env, out, static_cast<uint32_t>(i), item);
    }
    return out;
}

// waitMs > 0: blocks until the run ended (a user stop ends after the ACK carrying USER_STOPPED was written)
// or the time is up.
void stopRun(const std::shared_ptr<Run>& run, bool userInitiated, int waitMs = 0) {
    std::unique_lock<std::mutex> lock(run->mutex);
    run->stopRequested = true;
    run->userStop = run->userStop || userInitiated;
    // A user stop stays one when the run is then replaced: the PC still gets USER_STOPPED with its next command.
    if (run->session) run->session->stop(run->userStop);
    else if (run->stream) run->stream->close();
    if (waitMs > 0) run->ended.wait_for(lock, std::chrono::milliseconds(waitMs), [&] { return run->finished; });
}

void stopCurrent(bool userInitiated, int waitMs = 0) {
    std::shared_ptr<Run> run;
    {
        std::lock_guard<std::mutex> lock(gMutex);
        run = gRun;
    }
    if (run) stopRun(run, userInitiated, waitMs);
}

// ---- exports ---------------------------------------------------------------------------------------------

std::shared_ptr<Run> replaceRun(napi_env env, napi_value callback) {
    auto run = std::make_shared<Run>();
    napi_value name;
    napi_create_string_utf8(env, "anpodEvents", NAPI_AUTO_LENGTH, &name);
    if (napi_create_threadsafe_function(env, callback, nullptr, name, 0, 1, nullptr, nullptr, nullptr, callJs,
                                        &run->events) != napi_ok) {
        napi_throw_error(env, nullptr, "cannot create the event channel");
        return nullptr;
    }
    stopCurrent(false);  // a new launch from the PC replaces the old run
    {
        std::lock_guard<std::mutex> lock(gMutex);
        gRun = run;
    }
    return run;
}

napi_value StartWireless(napi_env env, napi_callback_info info) {
    auto argv = args(env, info, 2);
    std::string hex;
    std::optional<Id16> host;
    napi_valuetype callbackType = napi_undefined;
    if (argv.empty() || !stringArg(env, argv[0], hex) || (!hex.empty() && !parseHex(hex, host.emplace())) ||
        napi_typeof(env, argv[1], &callbackType) != napi_ok || callbackType != napi_function) {
        napi_throw_type_error(env, nullptr, "startWireless: invalid host or callback");
        return nullptr;
    }
    auto run = replaceRun(env, argv[1]);
    if (!run) return nullptr;
    std::thread(runWireless, run, host).detach();
    return undefinedValue(env);
}

napi_value StartHdcSession(napi_env env, napi_callback_info info) {
    auto argv = args(env, info, 4);
    double port = 0;
    std::string hostHex, tokenHex;
    Id16 hostId;
    Token16 token;
    napi_valuetype callbackType = napi_undefined;
    if (argv.empty() || napi_get_value_double(env, argv[0], &port) != napi_ok || std::trunc(port) != port ||
        port < 40000 || port > 49999 ||
        !stringArg(env, argv[1], hostHex) || !parseHex(hostHex, hostId) || !stringArg(env, argv[2], tokenHex) ||
        !parseHex(tokenHex, token) || napi_typeof(env, argv[3], &callbackType) != napi_ok ||
        callbackType != napi_function) {
        napi_throw_type_error(env, nullptr, "startHdcSession: invalid port, hostId, launchToken or callback");
        return nullptr;
    }
    auto run = replaceRun(env, argv[3]);
    if (!run) return nullptr;
    std::thread(runWired, run, static_cast<uint16_t>(port), hostId, token).detach();
    return undefinedValue(env);
}

napi_value AnswerPairing(napi_env env, napi_callback_info info) {
    auto argv = args(env, info, 1);
    bool accept = false;
    if (argv.empty() || napi_get_value_bool(env, argv[0], &accept) != napi_ok) return undefinedValue(env);
    std::shared_ptr<Run> run;
    {
        std::lock_guard<std::mutex> lock(gMutex);
        run = gRun;
    }
    if (run) {
        std::lock_guard<std::mutex> lock(run->mutex);
        if (run->bootstrap) run->bootstrap->answerPairing(accept);
    }
    return undefinedValue(env);
}

napi_value StopSession(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    size_t argc = 2;
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    bool userInitiated = false;
    int32_t waitMs = 0;
    if (argc >= 1) napi_get_value_bool(env, argv[0], &userInitiated);
    if (argc >= 2 && napi_get_value_int32(env, argv[1], &waitMs) == napi_ok) waitMs = std::clamp(waitMs, 0, 3000);
    stopCurrent(userInitiated, waitMs);
    return undefinedValue(env);
}

napi_value ListBindings(napi_env env, napi_callback_info) {
    auto ids = BindingStore::list();
    if (!ids) {
        napi_throw_error(env, nullptr, "binding store unavailable");
        return nullptr;
    }
    std::vector<std::string> hex;
    for (const Id16& id : *ids) hex.push_back(toHex(id));
    return stringArray(env, hex);
}

napi_value ForgetBinding(napi_env env, napi_callback_info info) {
    auto argv = args(env, info, 1);
    std::string hex;
    Id16 id;
    bool removed = !argv.empty() && stringArg(env, argv[0], hex) && parseHex(hex, id) && BindingStore::remove(id);
    napi_value out;
    napi_get_boolean(env, removed, &out);
    return out;
}

// ---- run-loop self-tests: serveWireless against simulated PCs over loopback TCP ----------------------------

struct LoopTest {
    static BindingMaterial testBinding() {
        Id16 host, receiver;
        Key32 secret;
        host.fill(0xc5);
        receiver.fill(0xc6);
        secret.fill(0xc7);
        return {host, receiver, secret};
    }

    BindingMaterial binding = testBinding();
    std::shared_ptr<Run> run = std::make_shared<Run>();  // never gRun: its events go nowhere
    std::mutex mutex;
    std::deque<int> pending;
    std::string error = "running";
    std::thread loop;

    explicit LoopTest(std::optional<Nonce16> intent) {
        BindingStore::remove(binding.hostId());
        if (!BindingStore::save(binding)) return;
        std::optional<Id16> selected;
        if (intent) selected = binding.hostId();
        loop = std::thread([this, selected, intent] {
            error = serveWireless(
                run, selected, intent,
                [this](int timeoutMs) {
                    {
                        std::lock_guard<std::mutex> lock(mutex);
                        if (!pending.empty()) {
                            int fd = pending.front();
                            pending.pop_front();
                            return fd;
                        }
                    }
                    std::this_thread::sleep_for(std::chrono::milliseconds(std::min(timeoutMs, 20)));
                    return -1;
                },
                [] { return true; });
        });
    }
    // One PC session; false when it never authenticated.
    bool peer(int command, uint32_t& capabilities, int& result, const std::function<void()>& ready = [] {}) {
        return loop.joinable() && selfTestWirelessPeer(
                                      binding,
                                      [this](int fd) {
                                          std::lock_guard<std::mutex> lock(mutex);
                                          pending.push_back(fd);
                                      },
                                      ready, command, capabilities, result);
    }
    // Stops the run like the UI does and returns the loop's end error.
    std::string finish() {
        stopRun(run, false);
        if (loop.joinable()) loop.join();
        for (int fd : pending) close(fd);
        BindingStore::remove(binding.hostId());
        return error;
    }
};

// R-M2-4: a PC STOP (manual disconnect, or the stop before a quality / mode change reconnect) ends only that
// session. The run keeps listening, and the tablet-selected intent is spent: the next session offers none.
bool wirelessPcStopKeepsListening() {
    Nonce16 intent;
    intent.fill(0x5a);
    LoopTest test(intent);
    uint32_t first = 0, second = 0;
    int stopped = -1, none = -1;
    bool ok = test.peer(p5c::kStop, first, stopped) && test.peer(-1, second, none);
    std::string error = test.finish();
    return ok && stopped == 0 && (first & p5c::kCapConnectionIntent) && !(second & p5c::kCapConnectionIntent) &&
           error.empty();
}

// Switching wired -> wireless stops the old run as a user stop, then replaceRun stops it again as a replacement:
// the PC must still get USER_STOPPED with its next command, or it reconnects the old way.
bool replacedUserStopStillSendsUserStopped() {
    LoopTest test(std::nullopt);
    uint32_t capabilities = 0;
    int result = -1;
    bool ok = test.peer(p5c::kHeartbeat, capabilities, result, [&] {
        stopRun(test.run, true);
        stopRun(test.run, false);
    });
    test.finish();
    return ok && result == p5c::kResultUserStopped;
}

napi_value SelfTest(napi_env env, napi_callback_info) {
    auto failures = runSelfTests();
    if (!wirelessPcStopKeepsListening()) failures.emplace_back("wireless_pc_stop_keeps_listening");
    if (!replacedUserStopStillSendsUserStopped()) failures.emplace_back("replaced_user_stop_still_sends_user_stopped");
    return stringArray(env, failures);
}

// ---- selfTestLoopback (Task 9) ---------------------------------------------------------------------------

// Checks the Float32 pattern of adb_record_stream_production_coordinator_loopback (src-tauri/src/p5c_tests.rs):
// every 143-frame packet repeats FLOAT_BITS per interleaved stereo sample, 14 packets, then PAUSE, RESUME, STOP.
class VerifyingSink final : public PlaybackSink {
public:
    static constexpr uint32_t kBits[8] = {0x3f800000, 0xbf000000, 0x00000001, 0x80000000,
                                          0x7f7fffff, 0x80000001, 0x3eaaaaab, 0x00800000};
    static constexpr uint64_t kFramesPerPacket = 143, kPackets = 14;

    bool open(const AudioFormatV2& format) override {
        opened = format.sampleFormat == 3 && format.channels == 2;
        return opened;
    }
    bool write(const float* samples, size_t frames) override {
        for (size_t f = 0; f < frames; f++, frameIndex++) {
            for (size_t c = 0; c < 2; c++) {
                uint32_t bits;
                std::memcpy(&bits, &samples[f * 2 + c], sizeof bits);
                if (bits != kBits[((frameIndex % kFramesPerPacket) * 2 + c) % 8] && mismatchAt < 0)
                    mismatchAt = static_cast<int64_t>(frameIndex);
            }
        }
        return true;
    }
    void pause() override { pauses++; }
    void resume() override { resumes++; }
    void close() override { closes++; }
    PlaybackStats stats() const override {
        PlaybackStats s;
        s.playbackAdvancing = opened && frameIndex > 0;
        return s;
    }

    bool opened = false;
    uint64_t frameIndex = 0;  // frames received in order (the reader thread is the only writer)
    int64_t mismatchAt = -1;
    int pauses = 0, resumes = 0, closes = 0;
};

// shortcut: blocks the calling JS thread (up to 60 s) and runs one session per call; fine for aa test, make it
// async before any UI calls it.
napi_value SelfTestLoopback(napi_env env, napi_callback_info info) {
    auto argv = args(env, info, 1);
    int32_t port = 0;
    if (argv.empty() || napi_get_value_int32(env, argv[0], &port) != napi_ok || port <= 0 || port > 65535)
        return stringArray(env, {"loopback_invalid_port"});
    auto stream = SocketStream::connectLoopback(static_cast<uint16_t>(port));
    if (!stream) return stringArray(env, {"loopback_connect_failed"});
    Id16 hostId, receiverId;
    Key32 secret;
    hostId.fill(0x01);
    receiverId.fill(0x02);
    secret.fill(0x03);
    VerifyingSink sink;
    std::mutex mutex;
    std::condition_variable changed;
    std::optional<std::string> message;
    SessionCallbacks callbacks;
    callbacks.onEnded = [&](const std::string& m) {
        std::lock_guard<std::mutex> lock(mutex);
        message = m;
        changed.notify_all();
    };
    auto session = StreamSession::start(std::move(stream), BindingMaterial(hostId, receiverId, secret), sink, callbacks);
    {
        std::unique_lock<std::mutex> lock(mutex);
        changed.wait_for(lock, std::chrono::seconds(60), [&] { return message.has_value(); });
    }
    session.reset();  // joins the reader: the sink fields are stable from here
    std::vector<std::string> failures;
    if (!message) failures.push_back("loopback_timeout");
    else if (!message->empty()) failures.push_back("loopback_session_error: " + *message);
    if (!sink.opened) failures.push_back("loopback_not_opened");
    if (sink.frameIndex != VerifyingSink::kFramesPerPacket * VerifyingSink::kPackets)
        failures.push_back("loopback_frames " + std::to_string(sink.frameIndex));
    if (sink.mismatchAt >= 0) failures.push_back("loopback_sample_mismatch at frame " + std::to_string(sink.mismatchAt));
    if (sink.pauses != 1 || sink.resumes != 1 || sink.closes != 1)
        failures.push_back("loopback_pause_resume_close " + std::to_string(sink.pauses) + "/" +
                           std::to_string(sink.resumes) + "/" + std::to_string(sink.closes));
    return stringArray(env, failures);
}

napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"startWireless", nullptr, StartWireless, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"startHdcSession", nullptr, StartHdcSession, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"answerPairing", nullptr, AnswerPairing, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"stopSession", nullptr, StopSession, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"listBindings", nullptr, ListBindings, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"forgetBinding", nullptr, ForgetBinding, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"selfTest", nullptr, SelfTest, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"selfTestLoopback", nullptr, SelfTestLoopback, nullptr, nullptr, nullptr, napi_default, nullptr},
    };
    napi_define_properties(env, exports, sizeof desc / sizeof desc[0], desc);
    return exports;
}

napi_module mod = {1, 0, nullptr, Init, "anpod_core", nullptr, {nullptr}};

}  // namespace

extern "C" __attribute__((constructor)) void RegisterAnpodCore() { napi_module_register(&mod); }
