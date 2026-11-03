#pragma once
#ifndef SF_SEMANTIC_ATTRIBUTION_TESTING
#error This access surface is restricted to the owned offline fixture target.
#endif
#include "semantic-attribution/native_event_adapter.h"

namespace SemanticAttribution::testing {
// Test-only access to the actual adapter state machine. No production issuer,
// transport publication entry or synthetic native authority is exported.
class NativeAdapterTestAccess final {
public:
    static NativeEventAdapter* Construct(std::span<std::byte>,Ledger&,
        std::span<const Native::ResidentSite>) noexcept;
    static bool Publish(NativeEventAdapter&) noexcept;
    static void LockCapture(NativeEventAdapter&) noexcept;
    static void UnlockCapture(NativeEventAdapter&) noexcept;
    static void Poison(NativeEventAdapter&) noexcept;
    static bool Poisoned(const NativeEventAdapter&) noexcept;
    static bool Published(const NativeEventAdapter&) noexcept;
    static bool TransportIs(const NativeEventAdapter*) noexcept;
    // Requires the fixture to hold the actual capture gate.
    static EventReply SendPass(NativeEventAdapter&) noexcept;
    static void Capture(NativeEventAdapter&,Native::SavedMachine&) noexcept;
};
}
