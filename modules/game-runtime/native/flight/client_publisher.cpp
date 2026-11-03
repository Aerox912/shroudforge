#include "flight_publisher.hpp"

#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
#include "flight_diagnostics.hpp"
#endif

#include <windows.h>
#include <psapi.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <limits>

namespace shroudweave::flight {
namespace {
constexpr char kGetResetSerialExport[] = "ESM_VeinGetFlightResetSerialV1";
constexpr char kPublishExport[] = "ESM_VeinPublishFlightIntentV2";
constexpr std::uint64_t kRefreshMs = 100;
constexpr std::uint64_t kDiscoveryMs = 1000;
std::atomic_uint64_t g_epoch{};

std::uint64_t NextEpoch() noexcept {
    LARGE_INTEGER counter{};
    const auto clock_value = QueryPerformanceCounter(&counter) && counter.QuadPart > 0
        ? static_cast<std::uint64_t>(counter.QuadPart)
        : GetTickCount64();
    auto previous = g_epoch.load(std::memory_order_relaxed);
    for (;;) {
        if (previous == (std::numeric_limits<std::uint64_t>::max)()) return 0;
        const auto next = (std::max)(clock_value, previous + 1);
        if (g_epoch.compare_exchange_weak(previous, next, std::memory_order_relaxed)) return next;
    }
}
}

FlightPublisher::~FlightPublisher() { Stop(); }

bool FlightPublisher::Start(ResetCallback reset_callback, void* reset_context) noexcept {
    std::lock_guard lock(gate_);
    if (running_) return true;
    const auto epoch = NextEpoch();
    if (!epoch) return false;
    epoch_ = epoch;
    emergency_off_.store(false,std::memory_order_release);
    desired_on_ = false;
    stop_requested_ = false;
    has_valid_serial_ = false;
    serial_valid_ = false;
    observed_serial_ = 0;
    last_on_publish_ms_ = 0;
    reset_generation_ = 0;
    prepared_enable_generation_ = 0;
    bound_off_published_ = false;
    enable_preparation_pending_ = false;
    reset_generation_exhausted_ = false;
    reset_callback_ = reset_callback;
    reset_context_ = reset_context;

    (void)ObserveResetLocked();
    if (!bound_off_published_)
        (void)SendIntentLocked(false, serial_valid_ ? observed_serial_ : 0, false);
    try {
        running_ = true;
        worker_ = std::thread(&FlightPublisher::Run, this);
    } catch (...) {
        running_ = false;
        stop_requested_ = true;
        ReleaseProviderLocked();
        provider_module_ = nullptr;
        get_reset_serial_ = nullptr;
        publish_ = nullptr;
        reset_callback_ = nullptr;
        reset_context_ = nullptr;
        return false;
    }
    return true;
}

bool FlightPublisher::PrepareEnable() noexcept {
    std::lock_guard lock(gate_);
    enable_preparation_pending_ = false;
    if (!running_ || stop_requested_ || emergency_off_.load(std::memory_order_acquire)) return false;
    if (reset_generation_exhausted_ || ObserveResetLocked(true) || reset_generation_exhausted_)
        return false;
    if (!serial_valid_ || !has_valid_serial_) return false;
    prepared_enable_generation_ = reset_generation_;
    enable_preparation_pending_ = true;
    return true;
}

void FlightPublisher::SetEnabled(bool enabled) noexcept {
    std::lock_guard lock(gate_);
    if (!running_ || stop_requested_) return;
    if (enabled && emergency_off_.load(std::memory_order_acquire)) return;
    if (!enabled) enable_preparation_pending_ = false;
    if (ObserveResetLocked(enabled)) {
        if (enabled) enable_preparation_pending_ = false;
        return;
    }
    if (enabled && reset_generation_exhausted_) {
        enable_preparation_pending_ = false;
        return;
    }
    if (enabled && enable_preparation_pending_) {
        const bool preparation_matches = prepared_enable_generation_ == reset_generation_;
        enable_preparation_pending_ = false;
        if (!preparation_matches) return;
    }
    if (enabled && !serial_valid_) return;
    if (desired_on_ == enabled) return;

    desired_on_ = enabled;
    const auto epoch = NextEpoch();
    if (!epoch) {
        desired_on_ = false;
        return;
    }
    epoch_ = epoch;
    last_on_publish_ms_ = enabled ? GetTickCount64() : 0;
    (void)SendIntentLocked(enabled, serial_valid_ ? observed_serial_ : 0);
    changed_.notify_all();
}

void FlightPublisher::Stop() noexcept {
    std::thread worker;
    {
        std::lock_guard lock(gate_);
        if (!running_) return;
        enable_preparation_pending_ = false;
        (void)ObserveResetLocked();
        desired_on_ = false;
        const auto epoch = NextEpoch();
        if (epoch) epoch_ = epoch;
        last_on_publish_ms_ = 0;
        (void)SendIntentLocked(false, serial_valid_ ? observed_serial_ : 0, false);
        stop_requested_ = true;
        running_ = false;
        changed_.notify_all();
        worker = std::move(worker_);
    }
    if (worker.joinable()) {
        try { worker.join(); } catch (...) { }
    }
    std::lock_guard lock(gate_);
    ReleaseProviderLocked();
    get_reset_serial_ = nullptr;
    publish_ = nullptr;
    provider_module_ = nullptr;
    reset_callback_ = nullptr;
    reset_context_ = nullptr;
    desired_on_ = false;
    has_valid_serial_ = false;
    serial_valid_ = false;
    stop_requested_ = false;
    epoch_ = 0;
    observed_serial_ = 0;
    last_on_publish_ms_ = 0;
    reset_generation_ = 0;
    prepared_enable_generation_ = 0;
    bound_off_published_ = false;
    enable_preparation_pending_ = false;
    reset_generation_exhausted_ = false;
}

bool FlightPublisher::ResolveLocked() noexcept {
    // Typed references inside the admitted resident kfc-runtime, no DLL scan.
    get_reset_serial_ = &ESM_VeinGetFlightResetSerialV1;
    publish_ = &ESM_VeinPublishFlightIntentV2;
    provider_module_ = this; // availability marker, not an HMODULE reference
    return true;
}

bool FlightPublisher::QuerySerialLocked(std::uint64_t& serial) noexcept {
    serial = 0;
    if (!get_reset_serial_) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        ProcessFlightDiagnostics().RecordQuerySerial(false, 0);
#endif
        return false;
    }
    bool result{};
    try { result = get_reset_serial_(&serial); } catch (...) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        ProcessFlightDiagnostics().RecordQuerySerial(false, 0);
#endif
        return false;
    }
    const bool valid = result && serial != 0;
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    ProcessFlightDiagnostics().RecordQuerySerial(valid, valid ? serial : 0);
#endif
    return valid;
}

bool FlightPublisher::ObserveResetLocked(bool reject_new_binding) noexcept {
    if (!ResolveLocked()) return false;
    std::uint64_t current_serial{};
    const bool valid = QuerySerialLocked(current_serial);

    if (!has_valid_serial_) {
        if (!valid) return false; // An exported provider may still be disabled or not runtime-ready.
        has_valid_serial_ = true;
        serial_valid_ = true;
        observed_serial_ = current_serial;
        if (desired_on_ || reject_new_binding || enable_preparation_pending_) {
            ResetLocked(current_serial, true);
            return true;
        }
        PublishBoundOffOnceLocked(current_serial);
        return false;
    }

    if (!valid) {
        if (serial_valid_ || desired_on_) {
            serial_valid_ = false;
            ResetLocked(0, false);
            return true;
        }
        return false;
    }

    if (!serial_valid_) {
        observed_serial_ = current_serial;
        serial_valid_ = true;
        ResetLocked(current_serial, true);
        return true;
    }

    if (current_serial != observed_serial_) {
        observed_serial_ = current_serial;
        ResetLocked(current_serial, true);
        return true;
    }
    PublishBoundOffOnceLocked(current_serial);
    return false;
}

void FlightPublisher::PublishBoundOffOnceLocked(std::uint64_t serial) noexcept {
    if (bound_off_published_ || !serial_valid_ || serial == 0) return;
    bound_off_published_ = true;
    (void)SendIntentLocked(false, serial, false);
}

void FlightPublisher::ResetLocked(std::uint64_t serial, bool serial_valid) noexcept {
    if (reset_generation_ == (std::numeric_limits<std::uint64_t>::max)())
        reset_generation_exhausted_ = true;
    else
        ++reset_generation_;
    desired_on_ = false;
    serial_valid_ = serial_valid;
    bound_off_published_ = serial_valid;
    if (serial_valid) observed_serial_ = serial;
    last_on_publish_ms_ = 0;
    const auto epoch = NextEpoch();
    if (epoch) epoch_ = epoch;
    if (reset_callback_) reset_callback_(reset_context_);
    (void)SendIntentLocked(false, serial_valid ? serial : observed_serial_, false);
}

bool FlightPublisher::SendIntentLocked(bool enabled, std::uint64_t serial,
                                       bool inspect_rejection) noexcept {
    if(enabled&&emergency_off_.load(std::memory_order_acquire))return false;
    if (!provider_module_ || !publish_ || !epoch_) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        ProcessFlightDiagnostics().RecordPublishSkipped();
#endif
        return false;
    }
    if (enabled && !serial_valid_) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        ProcessFlightDiagnostics().RecordPublishSkipped();
#endif
        return false;
    }
    const VeinFlightIntentV2 intent{32, 2, epoch_, enabled ? 1u : 0u, 0, serial};
    bool accepted{};
    try { accepted = publish_(&intent); } catch (...) { accepted = false; }
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    ProcessFlightDiagnostics().RecordPublishResult(accepted);
#endif
    if (accepted || !inspect_rejection || !get_reset_serial_) return accepted;

    std::uint64_t current_serial{};
    const bool valid = QuerySerialLocked(current_serial);
    if (!valid) {
        if (serial_valid_ || desired_on_) {
            serial_valid_ = false;
            ResetLocked(0, false);
        }
    } else if (!serial_valid_ || current_serial != observed_serial_) {
        observed_serial_ = current_serial;
        serial_valid_ = true;
        ResetLocked(current_serial, true);
    } else if (enabled) {
        // A resident export and unchanged reset serial do not prove current
        // native input admission. Refused On must close local selection too.
        ResetLocked(current_serial, true);
    }
    return false;
}

bool FlightPublisher::Requested() noexcept {
    std::lock_guard lock(gate_);return running_ && desired_on_ && serial_valid_ && !emergency_off_.load(std::memory_order_acquire);
}
void FlightPublisher::EmergencyOff() noexcept {
    emergency_off_.store(true,std::memory_order_release);
    changed_.notify_all();
}

void FlightPublisher::ReleaseProviderLocked() noexcept {
    provider_module_ = nullptr; // provider and callbacks are resident in this module
}

void FlightPublisher::Run() noexcept {
    std::unique_lock lock(gate_);
    while (!stop_requested_) {
        if(emergency_off_.load(std::memory_order_acquire)&&desired_on_)
            ResetLocked(serial_valid_?observed_serial_:0,serial_valid_);
        if (!desired_on_) {
            (void)ObserveResetLocked();
            if (stop_requested_ || desired_on_) continue;
            const auto interval = provider_module_ && get_reset_serial_ && publish_
                ? kRefreshMs : kDiscoveryMs;
            changed_.wait_for(lock, std::chrono::milliseconds(interval),
                [this] { return stop_requested_ || desired_on_; });
            continue;
        }
        (void)ObserveResetLocked();
        if (stop_requested_ || !desired_on_) continue;
        const auto now = GetTickCount64();
        const auto elapsed = now >= last_on_publish_ms_ ? now - last_on_publish_ms_ : 0;
        if (elapsed < kRefreshMs) {
            changed_.wait_for(lock, std::chrono::milliseconds(kRefreshMs - elapsed),
                [this] { return stop_requested_ || !desired_on_; });
            continue;
        }
        (void)SendIntentLocked(true, serial_valid_ ? observed_serial_ : 0);
        last_on_publish_ms_ = GetTickCount64();
    }
}

} // namespace shroudweave::flight
