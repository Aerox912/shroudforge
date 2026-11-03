// SPDX-License-Identifier: MIT
#pragma once
namespace shroudforge::flight {
// Called by the trusted host on final demand retirement before provider stop.
// Leaves resident scalar callback in stock mode; never tears down Vein.
void stop_client() noexcept;
}
