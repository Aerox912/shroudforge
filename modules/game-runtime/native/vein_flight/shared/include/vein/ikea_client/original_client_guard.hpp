// SPDX-License-Identifier: MIT
#pragma once
#include <filesystem>
namespace vein::ikea_client {
// Known canonical original Vein path only. Installer must separately attest
// exclusion from every active loader discovery root; INI disabled is insufficient.
[[nodiscard]] bool canonical_original_client_absent(const std::filesystem::path& absolute_game_directory) noexcept;
}
