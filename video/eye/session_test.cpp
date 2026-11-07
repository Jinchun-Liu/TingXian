#include "session.h"
#include <atomic>
#include <condition_variable>
#include <functional>
#include <iostream>
#include <stdexcept>
#include <thread>
using namespace tingxian::eye;
namespace {
int checks = 0;
void require(bool good, const char* label) {
    if (!good) throw std::runtime_error(label);
    ++checks;
}
struct TestClock : Clock {
    std::atomic<uint64_t> now{1000};
    std::atomic<uint64_t> qpc{10000000};
    uint64_t now_ms() const noexcept override { return now.load(); }
    uint64_t now_qpc() const noexcept override { return qpc.load(); }
};
struct Barrier {
    std::mutex mutex;
    std::condition_variable cv;
    bool entered = false, released = false;
    void block() {
        std::unique_lock lock(mutex);
        entered = true; cv.notify_all();
        cv.wait(lock, [&] { return released; });
    }
    void wait() {
        std::unique_lock lock(mutex);
        if (!cv.wait_for(lock, std::chrono::seconds(3), [&] { return entered; })) std::terminate();
    }
    void release() { std::lock_guard lock(mutex); released = true; cv.notify_all(); }
};
struct Fake : EyeCorrectionProvider {
    int prepares = 0, processes = 0, stops = 0;
    bool cleanup_ok = true;
    std::function<Error()> on_prepare;
    std::function<Result(const Frame&)> on_process;
    std::function<void()> on_stop;
    Capabilities capabilities() const noexcept override { return {"synthetic", "1", true}; }
    Error prepare(Format) override { ++prepares; return on_prepare ? on_prepare() : Error::none; }
    Result process(const Frame& frame) override {
        ++processes;
        return on_process ? on_process(frame) : Result{Error::none, std::vector<uint8_t>(frame.bgr.size(), 200)};
    }
    bool stop() noexcept override { ++stops; if (on_stop) on_stop(); return cleanup_ok; }
};
struct Sink : OutputSink {
    std::vector<Frame> history;
    uint64_t revocations = 0;
    bool refuse = false, visible = false;
    void neutral(uint64_t) noexcept override { ++revocations; visible = false; }
    bool corrected(const Frame& f) noexcept override {
        if (refuse) return false;
        history.push_back(f); visible = true; return true;
    }
};
struct Test {
    Fake provider; Sink sink; TestClock clock;
    Session session{provider, sink, clock};
    const Format format{4, 2};
    uint64_t gen = 0;
    void start() { gen = session.begin(format); require(gen && session.prepare(gen), "explicit prepare"); }
    Frame frame(uint64_t id = 1) { return {gen, id, clock.now_ms(), std::vector<uint8_t>(24, 17)}; }
    ~Test() { session.stop(); session.drain_stop(); }
};
void ordinary_and_queue() {
    Test t;
    require(t.session.snapshot().state == State::off && !t.provider.prepares, "default off without SDK access");
    require(!t.session.begin({0, 1}) && !t.session.begin({1921, 1080}), "bounded valid format");
    t.start();
    require(!t.session.begin(t.format), "no implicit concurrent restart");
    for (uint64_t i = 1; i <= 10; ++i) require(t.session.submit(t.frame(i)), "bounded queue admission");
    auto status = t.session.snapshot();
    require(status.queued == 2 && status.dropped == 8, "oldest waiting frames dropped with exact count");
    require(t.session.process_one() && t.session.process_one(), "process current queued frames");
    require(t.sink.history[0].id == 9 && t.sink.history[1].id == 10, "no backlog catchup");
    require(t.sink.history[0].bgr[0] == 200, "only provider result delivered");
    require(!t.session.submit(t.frame(10)), "duplicate frame rejected");
    auto bad = t.frame(11); --bad.generation;
    require(!t.session.submit(bad), "old generation rejected");
    bad = t.frame(11); bad.bgr.pop_back();
    require(!t.session.submit(bad), "wrong payload rejected");
    bad = t.frame(11); ++bad.captured_ms;
    require(!t.session.submit(bad), "future timestamp rejected");
    bad = t.frame(11); bad.captured_ms -= 251;
    require(!t.session.submit(bad), "expired frame rejected");
    require(t.session.submit(t.frame(11)), "accept fresh before stop");
    t.session.stop();
    require(!t.sink.visible && t.session.snapshot().queued == 0, "stop revokes and clears queue");
    require(!t.session.begin(t.format), "stop cleanup precedes restart");
    t.session.drain_stop();
    for (int i = 0; i < 3; ++i) {
        const auto old = t.gen;
        t.start();
        require(t.gen > old && t.session.submit(t.frame()), "restart new generation and IDs");
        require(t.session.process_one(), "restarted current delivery");
        t.session.stop(); t.session.drain_stop();
    }
}
void failures() {
    for (const auto kind : {Error::no_face, Error::inference, Error::none, Error::exception}) {
        Test t; t.start();
        t.provider.on_process = [kind](const Frame& f) -> Result {
            if (kind == Error::exception) throw std::runtime_error("synthetic");
            // Failures deliberately carry input bytes. They MUST be ignored.
            return {kind, kind == Error::none ? std::vector<uint8_t>{} : f.bgr};
        };
        t.session.submit(t.frame()); t.session.submit(t.frame(2));
        require(!t.session.process_one(), "failure refuses delivery");
        const auto s = t.session.snapshot();
        require(s.state == State::fault && s.queued == 0 && s.dropped == 2, "failure stops and counts all dropped");
        require(t.sink.history.empty() && !t.sink.visible, "no raw fallback on error or malformed output");
        require(!t.session.submit(t.frame(3)) && !t.session.process_one(), "fault cannot resume silently");
    }
    Test t;
    t.provider.on_prepare = [] { return Error::unavailable; };
    const auto gen = t.session.begin(t.format);
    require(!t.session.prepare(gen) && t.provider.stops == 1, "partial preparation cleaned");
    require(t.session.snapshot().state == State::unavailable && !t.sink.visible, "missing runtime neutral");
    Test refuse; refuse.start(); refuse.sink.refuse = true; refuse.session.submit(refuse.frame());
    require(!refuse.session.process_one() && refuse.session.snapshot().state == State::fault, "sink refusal stops");
    Test uncertain; uncertain.start(); uncertain.provider.cleanup_ok = false;
    uncertain.session.stop(); uncertain.session.drain_stop(); uncertain.session.stop();
    require(uncertain.session.snapshot().cleanup_uncertain && !uncertain.session.begin(uncertain.format),
        "uncertain cleanup remains restart-blocked even after stop");
}
void cancellation() {
    Test queued;
    const auto old = queued.session.begin(queued.format);
    queued.session.stop(); queued.start();
    require(!queued.session.prepare(old) && queued.provider.prepares == 1, "stale queued initialization never runs");
    for (bool during_prepare : {true, false}) {
        Test t; Barrier barrier;
        if (during_prepare) {
            t.gen = t.session.begin(t.format);
            t.provider.on_prepare = [&] { barrier.block(); return Error::none; };
        } else {
            t.start(); t.session.submit(t.frame());
            t.provider.on_process = [&](const Frame& f) { barrier.block(); return Result{Error::none, f.bgr}; };
        }
        std::thread worker([&] { if (during_prepare) t.session.prepare(t.gen); else t.session.process_one(); });
        barrier.wait(); t.session.stop();
        const bool invalidated = !t.sink.visible && t.session.snapshot().state == State::stopping;
        const bool restart_refused = !t.session.begin(t.format);
        barrier.release(); worker.join();
        require(invalidated && restart_refused, "stop does not wait on vendor and prevents overlapping lifetime");
        require(t.session.snapshot().state == State::off && t.provider.stops == 1, "late result cleaned after stop");
        require(t.sink.history.empty(), "late successful result never emitted");
    }
}
void age_and_race() {
    Test t; t.start(); t.session.submit(t.frame());
    t.clock.now += 251;
    require(!t.session.process_one() && !t.provider.processes, "expire before GPU");
    t.session.submit(t.frame(2));
    t.provider.on_process = [&](const Frame& f) { t.clock.now += 251; return Result{Error::none, f.bgr}; };
    require(!t.session.process_one() && t.sink.history.empty(), "expire after GPU");
    require(t.session.snapshot().dropped == 2, "expiry counted");
    // Bounded stress of the two possible stop/inference interleavings.
    for (int i = 0; i < 30; ++i) {
        Test r; r.start(); r.session.submit(r.frame());
        std::thread worker([&] { r.session.process_one(); });
        r.session.stop(); worker.join(); r.session.drain_stop();
        require(!r.sink.visible && r.session.snapshot().state == State::off, "stop serialization keeps neutral final state");
    }
}
void qpc_capture_contract() {
    Fake provider; Sink sink; TestClock clock;
    Session session(provider, sink, clock, ClockDomain::qpc_100ns);
    const auto gen = session.begin({4, 2}); require(gen && session.prepare(gen), "QPC prepare");
    auto frame = [&](uint64_t id) { return Frame{gen, id, 0, std::vector<uint8_t>(24, 17), clock.now_qpc(), 0}; };
    auto mixed = frame(1); mixed.captured_ms = clock.now_ms();
    require(!session.submit(mixed), "QPC rejects mixed milliseconds");
    mixed = frame(1); mixed.completed_qpc = mixed.capture_qpc;
    require(!session.submit(mixed), "input cannot assert inference completion");
    mixed = frame(1); mixed.capture_qpc = 0;
    require(!session.submit(mixed), "QPC zero rejected");
    mixed = frame(1); ++mixed.capture_qpc;
    require(!session.submit(mixed), "future QPC rejected");
    mixed = frame(1); mixed.capture_qpc -= 2500001;
    require(!session.submit(mixed), "QPC stale rejected at original capture");
    const auto capture = clock.now_qpc();
    provider.on_process = [&](const Frame& f) {
        require(f.capture_qpc == capture && !f.captured_ms && !f.completed_qpc, "provider receives original capture only");
        clock.qpc += 400000;
        return Result{Error::none, std::vector<uint8_t>(24, 200)};
    };
    require(session.submit(frame(1)) && session.process_one(), "delayed QPC frame delivered");
    const auto& out = sink.history.back();
    require(out.capture_qpc == capture && out.completed_qpc == capture + 400000 && !out.captured_ms,
        "capture and inference completion remain distinct");
    require(out.bgr == std::vector<uint8_t>(24, 200), "only corrected QPC pixels delivered");
    provider.on_process = [&](const Frame&) { clock.qpc += 2500001; return Result{Error::none, std::vector<uint8_t>(24, 200)}; };
    require(session.submit(frame(2)) && !session.process_one() && !sink.visible && sink.history.size() == 1,
        "slow inference cannot renew capture age");
    provider.on_process = [&](const Frame&) { clock.qpc -= 1; return Result{Error::none, std::vector<uint8_t>(24, 200)}; };
    require(session.submit(frame(3)) && !session.process_one() && sink.history.size() == 1, "clock regression never publishes");
    session.stop(); session.drain_stop();
    require(!session.submit(frame(4)), "stopped QPC generation rejected");
    Test legacy; legacy.start(); auto wrong = legacy.frame(); wrong.capture_qpc = clock.now_qpc();
    require(!legacy.session.submit(wrong), "legacy never reinterprets QPC metadata");

    Fake delayed; Sink final_sink; TestClock final_clock; Barrier barrier;
    Session final(delayed, final_sink, final_clock, ClockDomain::qpc_100ns);
    const auto next = final.begin({4, 2}); require(next && final.prepare(next), "QPC cancellation prepare");
    delayed.on_process = [&](const Frame&) { barrier.block(); return Result{Error::none, std::vector<uint8_t>(24, 200)}; };
    require(final.submit({next, 1, 0, std::vector<uint8_t>(24, 17), final_clock.now_qpc(), 0}), "QPC cancellation input");
    std::thread worker([&] { final.process_one(); }); barrier.wait(); final.stop(); barrier.release(); worker.join(); final.drain_stop();
    require(final_sink.history.empty() && !final_sink.visible && final.snapshot().state == State::off,
        "late QPC inference cannot publish after stop");
}
void cleanup_race() {
    Test t; t.start(); Barrier barrier;
    t.provider.on_process = [](const Frame&) { return Result{Error::inference, {}}; };
    t.provider.on_stop = [&] { barrier.block(); };
    t.session.submit(t.frame());
    std::thread worker([&] { t.session.process_one(); });
    barrier.wait();
    const auto refused = !t.session.submit(t.frame(2));
    t.session.stop();
    const auto blocked = !t.session.begin(t.format);
    barrier.release(); worker.join(); t.provider.on_stop = {};
    require(refused && blocked, "fault cleanup cannot admit frames or restart");
    require(t.session.snapshot().state == State::off && t.session.snapshot().queued == 0,
        "stop during cleanup finishes off without a queued tail");
}
}
int main() {
    try {
        ordinary_and_queue(); failures(); cancellation(); age_and_race(); cleanup_race(); qpc_capture_contract();
        std::cout << "{\"checks\":" << checks << ",\"failures\":0,\"sdk_loaded\":false,\"media_started\":false}\n";
        return 0;
    } catch (const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
