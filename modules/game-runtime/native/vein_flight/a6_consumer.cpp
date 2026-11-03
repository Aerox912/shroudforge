// SPDX-License-Identifier: MIT
#include "vein_flight/a6_consumer.hpp"

#include <utility>

namespace VeinFlight {
namespace sf = vein::settings_file;
namespace client = vein::ikea_client;

sf::LoadResult A6Consumer::initialize(std::filesystem::path path,
                                     sf::FileOperations operations) {
    std::lock_guard guard(gate_);
    if (attempted_) return {sf::Error::not_loaded, std::nullopt, false};
    attempted_ = true;
    coordinator_ = std::make_unique<client::Coordinator>(std::move(path), operations);
    auto result = coordinator_->initialize();
    // This support phase retains the actual admitted settings file. Missing
    // files must not silently turn into defaults at the relocated provider path.
    if (result && !result.file_existed)
        return {sf::Error::not_loaded, std::nullopt, false};
    ready_ = static_cast<bool>(result);
    return result;
}

bool A6Consumer::begin_activation(std::uint64_t epoch) {
    std::lock_guard guard(gate_);
    if (!ready_ || faulted_.load() || !coordinator_->begin_activation(epoch)) return false;
    if (epoch_ != epoch) pending_radius_notice_.reset();
    epoch_ = epoch;
    return true;
}

void A6Consumer::retire_activation(std::uint64_t epoch) {
    std::lock_guard guard(gate_);
    if (!ready_) return;
    coordinator_->retire_activation(epoch);
    if (epoch_ == epoch) { epoch_ = 0; pending_radius_notice_.reset(); }
}

void A6Consumer::reset_native_session(std::uint64_t epoch) {
    std::lock_guard guard(gate_);
    if (!ready_) return;
    coordinator_->reset_native_session(epoch);
    if (epoch_ == epoch) pending_radius_notice_.reset();
}

std::optional<vein::Settings> A6Consumer::settings() const {
    std::lock_guard guard(gate_);
    return ready_ && !faulted_.load() ? coordinator_->settings() : std::nullopt;
}

vein::InputResult A6Consumer::observe_worker_input(
    std::uint64_t epoch, std::uint64_t generation, const InputObservation& input,
    std::uint64_t completed_tick_ms) {
    std::lock_guard guard(gate_);
    if (!ready_ || faulted_.load()) return {};
    return coordinator_->observe_worker_input(epoch, generation, input, completed_tick_ms);
}

A6Consumer::PacketResult A6Consumer::on_sampled_packet(
    std::uint64_t epoch, std::uint64_t generation, PacketTarget target,
    const InputObservation& context, bool fresh_hold, std::uint64_t now_tick_ms) {
    std::lock_guard guard(gate_);
    if (!ready_ || faulted_.load()) return {};
    return coordinator_->on_sampled_packet(epoch, generation, target, context,
                                           fresh_hold, now_tick_ms);
}

sf::Error A6Consumer::flush_settings() {
    // Initialization is completed before the owner publishes this object.
    // coordinator_ is never replaced, including consumer deactivation/fault.
    if (faulted_.load()) return persistence_error_.load();
    if (!ready_) return sf::Error::not_loaded;
    auto error = sf::Error::io_error;
    try { error = coordinator_->flush_settings(); }
    catch (...) { /* Persistence failures disarm Vein, not the shared provider. */ }
    if (error != sf::Error::none) {
        persistence_error_.store(error);
        faulted_.store(true);
        std::lock_guard guard(gate_);
        coordinator_->retire_activation(epoch_);
        epoch_ = 0;
        pending_radius_notice_.reset();
    }
    return error;
}

A6Consumer::NoticeEvents A6Consumer::take_notices(std::uint64_t epoch) {
    std::lock_guard guard(gate_);
    if (!ready_ || faulted_.load() || epoch_ != epoch) return {};
    auto events = coordinator_->take_notices(epoch);
    if (events.radius) pending_radius_notice_ = events.radius;
    events.radius.reset();
    if (!coordinator_->settings_pending()) {
        events.radius = std::exchange(pending_radius_notice_, std::nullopt);
    }
    return events;
}

bool A6Consumer::settings_pending() const {
    std::lock_guard guard(gate_);
    return ready_ && !faulted_.load() && coordinator_->settings_pending();
}

std::uint32_t A6Consumer::last_serial() const {
    std::lock_guard guard(gate_);
    return ready_ ? coordinator_->last_serial() : 0;
}

} // namespace VeinFlight
