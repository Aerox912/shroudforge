#pragma once

#include <cstddef>
#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <mutex>
#include <thread>
#include "vein/ikea_client/flight_intent_abi.h"

namespace shroudweave::flight {

// The pinned canonical reset-aware ABI is used here. Runtime provider
// availability and behavior remain unverified.
using VeinFlightIntentV2 = EsmVeinFlightIntentV2;

class FlightPublisher final {
public:
    using ResetCallback = void (*)(void*) noexcept;

    FlightPublisher() = default;
    ~FlightPublisher();
    FlightPublisher(const FlightPublisher&) = delete;
    FlightPublisher& operator=(const FlightPublisher&) = delete;

    // Starts the optional reset-aware bridge sender and submits default Off.
    // A qualified provider is required; no SP/listen fallback.
    [[nodiscard]] bool Start(ResetCallback reset_callback = nullptr,
                             void* reset_context = nullptr) noexcept;
    // Observe native reset state before local movement can transition On.
    // Missing or unready providers refuse On.
    [[nodiscard]] bool PrepareEnable() noexcept;
    void SetEnabled(bool enabled) noexcept;
    void Stop() noexcept;
    bool Requested() noexcept;
    // Lock-independent admission close for an exceptional control-gate failure.
    // The worker sends bound Off; no later On refresh is admitted.
    void EmergencyOff() noexcept;

private:
    using GetResetSerialFn = EsmVeinGetFlightResetSerialV1Fn;
    using PublishFn = EsmVeinPublishFlightIntentV2Fn;

    void Run() noexcept;
    bool ResolveLocked() noexcept;
    bool QuerySerialLocked(std::uint64_t& serial) noexcept;
    bool ObserveResetLocked(bool reject_new_binding = false) noexcept;
    void PublishBoundOffOnceLocked(std::uint64_t serial) noexcept;
    void ResetLocked(std::uint64_t serial, bool serial_valid) noexcept;
    bool SendIntentLocked(bool enabled, std::uint64_t serial,
                          bool inspect_rejection = true) noexcept;
    void ReleaseProviderLocked() noexcept;

    std::mutex gate_;
    std::condition_variable changed_;
    std::thread worker_;
    void* provider_module_{};
    GetResetSerialFn get_reset_serial_{};
    PublishFn publish_{};
    ResetCallback reset_callback_{};
    void* reset_context_{};
    std::uint64_t epoch_{}, observed_serial_{}, last_on_publish_ms_{};
    std::uint64_t reset_generation_{}, prepared_enable_generation_{};
    bool has_valid_serial_{}, serial_valid_{}, desired_on_{}, running_{}, stop_requested_{};
    bool bound_off_published_{}, enable_preparation_pending_{}, reset_generation_exhausted_{};
    std::atomic_bool emergency_off_{false};
};

} // namespace shroudweave::flight
