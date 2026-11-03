// SPDX-License-Identifier: MIT
#include "vein/ikea_client/native_input_session.hpp"

#include <limits>

namespace vein::ikea_client {

void VeinInputSession::clear_identity() noexcept {
    context_ = 0;
    channel_ = 0;
    last_input_tick_ms_ = 0;
    identity_known_ = false;
}

bool VeinInputSession::advance_generation() noexcept {
    if (exhausted_ || generation_ == (std::numeric_limits<std::uint64_t>::max)()) {
        exhausted_ = true;
        clear_identity();
        return false;
    }
    ++generation_;
    return true;
}

VeinInputUpdate VeinInputSession::observe_input(std::uintptr_t context,
                                               std::uint32_t channel,
                                               std::uint64_t completed_tick_ms,
                                               VeinInputTicket& ticket) noexcept {
    ticket = {};
    if (exhausted_ || transition_depth_ || !context || !channel) return VeinInputUpdate::invalid;
    if (identity_known_ && completed_tick_ms < last_input_tick_ms_) {
        invalidate();
        return VeinInputUpdate::invalid;
    }

    const bool changed = !identity_known_ || context != context_ || channel != channel_;
    if (changed) {
        if (!advance_generation()) return VeinInputUpdate::invalid;
        context_ = context;
        channel_ = channel;
        identity_known_ = true;
    }
    last_input_tick_ms_ = completed_tick_ms;
    ticket = {context_, channel_, generation_};
    return changed ? VeinInputUpdate::changed_session : VeinInputUpdate::same_session;
}

bool VeinInputSession::is_current(const VeinInputTicket& ticket) const noexcept {
    return !exhausted_ && transition_depth_ == 0 && identity_known_ && ticket.generation &&
           ticket.generation == generation_ && ticket.context == context_ &&
           ticket.channel == channel_;
}

bool VeinInputSession::current_for_worker(std::uint64_t now_tick_ms,
                                          VeinInputTicket& ticket) const noexcept {
    ticket = {};
    if (exhausted_ || transition_depth_ || !identity_known_ || !generation_ ||
        now_tick_ms < last_input_tick_ms_ ||
        now_tick_ms - last_input_tick_ms_ > worker_input_max_age_ms) {
        return false;
    }
    ticket = {context_, channel_, generation_};
    return true;
}

void VeinInputSession::begin_transition() noexcept {
    clear_identity();
    if (transition_depth_ == (std::numeric_limits<std::uint32_t>::max)()) {
        exhausted_ = true;
        return;
    }
    ++transition_depth_;
    (void)advance_generation();
}

void VeinInputSession::end_transition() noexcept {
    if (!transition_depth_) return;
    --transition_depth_;
    clear_identity();
    (void)advance_generation();
}

void VeinInputSession::invalidate() noexcept {
    clear_identity();
    (void)advance_generation();
}

} // namespace vein::ikea_client
