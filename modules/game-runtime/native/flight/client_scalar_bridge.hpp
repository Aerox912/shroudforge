// SPDX-License-Identifier: MIT
#pragma once
namespace shroudforge::flight {
bool prepare_scalar() noexcept;
bool scalar_ready() noexcept;
void set_scalar_enabled(bool) noexcept;
}
