#pragma once

#include "shroudweave_native_scalar_v2.h"

#include <windows.h>

#include <cstdint>

namespace shroudweave::flight {

using ShroudweaveGetNativeScalarApiV1Fn =
    const ShroudweaveNativeScalarApiV1*(SHROUDWEAVE_NATIVE_SCALAR_CALL*)(void);

class NativeScalarSdk final {
public:
    NativeScalarSdk() = default;
    NativeScalarSdk(const NativeScalarSdk&) = delete;
    NativeScalarSdk& operator=(const NativeScalarSdk&) = delete;

    bool LoadForAddress(const void* callerAddress) noexcept;
    bool Ready() const noexcept;
    bool Registered() const noexcept;
    HMODULE Module() const noexcept;
    const ShroudweaveNativeScalarApiV1* Api() const noexcept;
    DWORD LoadError() const noexcept;
    bool RegistrationAccepted() const noexcept;
    std::int32_t RegisterOne(const ShroudweaveNativeScalarRequestV1& request) noexcept;
    std::int32_t RegisterOneExtended(const ShroudweaveNativeScalarRequestV2& request) noexcept;
    std::int32_t RemoveFinal() noexcept;
    std::int32_t RemoveFinalWithRetries(std::uint32_t attempts) noexcept;
#if defined(SHROUDWEAVE_FLIGHT_TESTING)
    void OverrideApiForTests(const ShroudweaveNativeScalarApiV1* api,
                             const ShroudweaveNativeScalarApiV2* apiV2) noexcept;
#endif

private:
    HMODULE module_{};
    const ShroudweaveNativeScalarApiV1* api_{};
    const ShroudweaveNativeScalarApiV2* apiV2_{};
    HANDLE sdk_file_{};
    std::uint64_t handle_{};
    DWORD load_error_{};
    bool load_attempted_{};
    bool registration_attempted_{};
    bool registration_accepted_{};
};

NativeScalarSdk& ProcessNativeScalarSdk() noexcept;

} // namespace shroudweave::flight
