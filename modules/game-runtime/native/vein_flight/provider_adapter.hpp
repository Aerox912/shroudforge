// SPDX-License-Identifier: MIT
#pragma once
#include <filesystem>
#include "vein/ikea_client/coordinator.hpp"

namespace VeinFlight {
struct ProviderConfig {
    bool is_client = false;
    bool is_server = false;
    // Explicit admitted dedicated-guest mode, not inferred from Steam peers.
    bool dedicated_guest_qualified = false;
    bool allow_unverified_cursor_gate = false;
    bool flight_transport_enabled = false;
    std::filesystem::path vein_settings_path;
    void (*log)(const char*) = nullptr;
};
// Once-only resident preparation. Failed/partial starts cannot be retried; any
// callback originals remain pinned. Host calls before consumer activation.
bool prepare_provider(const ProviderConfig&) noexcept;
bool set_vein_enabled(bool) noexcept;
void tick_provider() noexcept;
// Terminal admission closure; does not unload hooks or another consumer.
void stop_provider() noexcept;
bool provider_ready() noexcept;
vein::ikea_client::NoticeEvents take_vein_notices() noexcept;
}
