// SPDX-License-Identifier: MIT
#pragma once

#include "vein/ikea_client/coordinator.hpp"

#include <atomic>
#include <memory>
#include <mutex>

namespace VeinFlight {

// No hooks, input sampling, host services or Flight lifetime are owned here.
// Initialize once before provider callbacks. The owner keeps this object alive
// until all callers have quiesced. The path is host-admitted, never DLL-derived.
class A6Consumer final {
public:
    using InputObservation = vein::ikea_client::InputObservation;
    using PacketTarget = vein::ikea_client::PacketTarget;
    using PacketResult = vein::ikea_client::PacketResult;
    using NoticeEvents = vein::ikea_client::NoticeEvents;

    [[nodiscard]] vein::settings_file::LoadResult initialize(
        std::filesystem::path admitted_config,
        vein::settings_file::FileOperations operations = {});
    [[nodiscard]] bool begin_activation(std::uint64_t epoch);
    void retire_activation(std::uint64_t epoch);
    void reset_native_session(std::uint64_t epoch);
    [[nodiscard]] std::optional<vein::Settings> settings() const;
    [[nodiscard]] vein::InputResult observe_worker_input(
        std::uint64_t epoch, std::uint64_t native_generation,
        const InputObservation& input, std::uint64_t completed_tick_ms);
    [[nodiscard]] PacketResult on_sampled_packet(
        std::uint64_t epoch, std::uint64_t native_generation,
        PacketTarget target, const InputObservation& context,
        bool fresh_hold_key_down, std::uint64_t now_tick_ms);
    // Management-thread only. File I/O holds no mutex used by packet callers.
    // Any failure latches this consumer off; no automatic retry or restoration.
    [[nodiscard]] vein::settings_file::Error flush_settings();
    [[nodiscard]] NoticeEvents take_notices(std::uint64_t epoch);
    [[nodiscard]] bool settings_pending() const;
    [[nodiscard]] std::uint32_t last_serial() const;
    [[nodiscard]] bool faulted() const noexcept { return faulted_.load(); }

private:
    mutable std::mutex gate_;
    std::unique_ptr<vein::ikea_client::Coordinator> coordinator_;
    bool attempted_ = false, ready_ = false;
    std::uint64_t epoch_ = 0;
    std::optional<unsigned> pending_radius_notice_;
    std::atomic<bool> faulted_{false};
    std::atomic<vein::settings_file::Error> persistence_error_{
        vein::settings_file::Error::none};
};

} // namespace VeinFlight
