#pragma once

namespace PlayerOwnership {
// Normal Session unregister drains registry readers. The reviewed alternate
// route is covered by the 0x9E9FD0 wrapper before its original destructor/frees.
// The bounded lifetime and positive-preparation reviews are accepted. This
// policy does not grant runtime readiness: production starts Unprepared and
// requires successful held-entry preparation and a non-retired service phase.
// It is not a runtime setting or permission for late hook installation.
inline constexpr bool service_lifetime_reviewed() noexcept {
    return true;
}
bool ServiceLifetimeReady() noexcept;
inline bool service_lifetime_admitted() noexcept {return ServiceLifetimeReady();}
}
