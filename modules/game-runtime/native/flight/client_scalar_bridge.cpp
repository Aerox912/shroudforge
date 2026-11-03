// SPDX-License-Identifier: MIT
// Adapted scalar callback/profile helpers only; no Mod/ModContext or Flight DLL.
#include "client_scalar_bridge.hpp"
#include "flight_client_frame.hpp"
#include "flight_client_profile_pins.hpp"
#include "flight_glider_gate.hpp"
#include "flight_layout.hpp"
#include "flight_scalar_sdk.hpp"
#include "flight_compiler_profile.hpp"
#include <array>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <limits>
#include <windows.h>
#include <bcrypt.h>
#include <psapi.h>
namespace {

#ifndef SHROUDWEAVE_FLIGHT_WITH_STAMINA
#define SHROUDWEAVE_FLIGHT_WITH_STAMINA 0
#endif


using shroudweave::flight::FindTargetLayout;
using shroudweave::flight::NativeScalarSdk;
using shroudweave::flight::ProcessNativeScalarSdk;
using shroudweave::flight::ScanStatus;
using shroudweave::flight::ScanStatusName;
using shroudweave::flight::TargetLayout;

constexpr std::uint32_t kClientImageSize = 0x2A40000;
constexpr std::uint32_t kPitchSiteRva = 0x26F61E;
constexpr std::uint32_t kLowerScalarRva = 0x1528D30;
constexpr std::uint32_t kUpperScalarRva = 0x15218D0;
constexpr std::size_t kClampSignatureSize = 48;
constexpr std::uint8_t kClampSignature[kClampSignatureSize]{
    0xF3, 0x0F, 0x10, 0x05, 0x0A, 0x97, 0x2B, 0x01,
    0xF2, 0x0F, 0x11, 0x4C, 0x24, 0x40, 0x77, 0x15,
    0x41, 0x0F, 0x2F, 0xC0, 0x77, 0x0F, 0xF3, 0x0F,
    0x10, 0x35, 0x94, 0x22, 0x2B, 0x01, 0xF3, 0x41,
    0x0F, 0x5D, 0xF0, 0xEB, 0x03, 0x0F, 0x28, 0xF0,
    0xF3, 0x0F, 0x10, 0x97, 0xD0, 0x04, 0x00, 0x00,
};
constexpr std::uint8_t kOriginalScalarInstruction[8]{
    0xF3, 0x0F, 0x10, 0x05, 0x0A, 0x97, 0x2B, 0x01,
};
constexpr std::uint32_t kFinalScalarRemoveAttempts = 3;
std::uint8_t g_callbackAnchor{};

std::atomic_bool g_scalarRegistered{};
std::atomic_bool g_scalarHandleOwned{};
std::atomic_bool g_scalarRegistrationAttempted{};
std::atomic_bool g_modulePinned{};
HMODULE g_pinnedFlightModule{};
TargetLayout g_admittedTarget{};
std::uintptr_t g_admittedImageBase{};
std::size_t g_admittedImageSize{};

#if defined(SHROUDWEAVE_FLIGHT_TESTING)
ShroudweaveNativeScalarRequestV1 g_testScalarRequest{};
std::int32_t g_testRegistrationResult{SHROUDWEAVE_NATIVE_SCALAR_OK};
std::uint32_t g_testRegistrationAttempts{};
std::uint32_t g_testRemovalAttempts{};
std::int32_t g_testRemoveStatus{SHROUDWEAVE_NATIVE_SCALAR_OK};
#endif

std::int32_t SHROUDWEAVE_NATIVE_SCALAR_CALL ClientScalarCallback(
    const ShroudweaveNativeScalarFrameV1* sourceFrame,
    std::uint64_t context,
    std::uint32_t* selectedScalarBits) noexcept;

bool ReadClientImage(std::uintptr_t& imageBase, std::size_t& imageSize) noexcept {
    const auto executable = GetModuleHandleW(nullptr);
    MODULEINFO info{};
    if (executable == nullptr ||
        !GetModuleInformation(GetCurrentProcess(), executable, &info,
                              static_cast<DWORD>(sizeof(info))) || info.SizeOfImage == 0) {
        return false;
    }
    imageBase = reinterpret_cast<std::uintptr_t>(executable);
    imageSize = info.SizeOfImage;
    return true;
}

template<class T>
bool ReadLoadedImageValue(std::uintptr_t imageBase, std::size_t imageSize,
                          std::uintptr_t rva, T& value) noexcept {
    if (imageBase == 0 || rva > imageSize || sizeof(T) > imageSize - rva ||
        imageBase > (std::numeric_limits<std::uintptr_t>::max)() - rva) return false;
    std::memcpy(&value, reinterpret_cast<const void*>(imageBase + rva), sizeof(T));
    return true;
}

bool MatchesRelocatedImagePointer(std::uintptr_t imageBase, std::size_t imageSize,
                                  std::uintptr_t fieldRva,
                                  std::uintptr_t expectedTargetRva) noexcept {
    std::uintptr_t actual{};
    if (!ReadLoadedImageValue(imageBase, imageSize, fieldRva, actual) ||
        expectedTargetRva >= imageSize ||
        imageBase > (std::numeric_limits<std::uintptr_t>::max)() - expectedTargetRva) return false;
    return actual == imageBase + expectedTargetRva;
}

bool MatchesLoadedImageName(std::uintptr_t imageBase, std::size_t imageSize,
                            std::uintptr_t recordRva, std::uintptr_t pointerOffset,
                            std::uintptr_t lengthOffset, std::uintptr_t nameRva,
                            const char* expectedName, std::size_t expectedLength) noexcept {
    if (expectedName == nullptr || expectedLength == 0 ||
        recordRva > (std::numeric_limits<std::uintptr_t>::max)() - pointerOffset ||
        recordRva > (std::numeric_limits<std::uintptr_t>::max)() - lengthOffset ||
        nameRva > imageSize || expectedLength >= imageSize - nameRva) return false;
    std::uintptr_t actualPointer{};
    std::uint64_t actualLength{};
    if (!ReadLoadedImageValue(imageBase, imageSize, recordRva + pointerOffset, actualPointer) ||
        !ReadLoadedImageValue(imageBase, imageSize, recordRva + lengthOffset, actualLength) ||
        actualLength != expectedLength ||
        nameRva >= imageSize ||
        imageBase > (std::numeric_limits<std::uintptr_t>::max)() - nameRva ||
        actualPointer != imageBase + nameRva) return false;
    const auto* bytes = reinterpret_cast<const char*>(imageBase + nameRva);
    return std::memcmp(bytes, expectedName, expectedLength) == 0 && bytes[expectedLength] == '\0';
}

bool MatchesLoadedImageBytes(std::uintptr_t imageBase, std::size_t imageSize,
                             std::uintptr_t rva, const char* expected,
                             std::size_t expectedLength) noexcept {
    if (imageBase == 0 || expected == nullptr || rva > imageSize ||
        expectedLength > imageSize - rva ||
        imageBase > (std::numeric_limits<std::uintptr_t>::max)() - rva) return false;
    return std::memcmp(reinterpret_cast<const void*>(imageBase + rva), expected,
                       expectedLength) == 0;
}

bool ExactClientSemanticFields(std::uintptr_t imageBase, std::size_t imageSize) noexcept {
    constexpr std::uintptr_t kDynamicType = 0x18EAB10;
    constexpr std::uintptr_t kDynamicMembers = 0x18E37D0;
    constexpr std::uintptr_t kStateMember = 0x18E3980;
    constexpr std::uintptr_t kLocomotionStateType = 0x1AD9CF0;
    constexpr std::uintptr_t kLocomotionStateEnum = 0x1AD8FE0;
    constexpr std::uintptr_t kHangGlidingEntry = 0x1AD9080;
    constexpr std::uint64_t kDynamicLocomotionFullKey = 0x717D8E414C234795ull;
    constexpr std::uint64_t kDynamicLocomotionSize = 0x1C8;
    constexpr std::uint64_t kHangGlidingValue = 4;
    constexpr std::uint64_t kStateOffset = 0x59;
    constexpr char kDynamicLocomotionName[] = "DynamicLocomotion";
    constexpr char kDynamicLocomotionQualifiedName[] = "keen::ecs::DynamicLocomotion";
    constexpr char kLocomotionStateName[] = "LocomotionState";
    constexpr char kLocomotionStateQualifiedName[] = "keen::ecs::LocomotionState";
    constexpr char kStateName[] = "state";
    constexpr char kHangGlidingName[] = "HangGliding";
    constexpr char kEcsPrefix[] = "keen::ecs::";

    std::uint64_t fullKey{};
    std::uint64_t componentSize{};
    std::uint64_t stateOffset{};
    std::uint64_t hangGlidingValue{};
    if (!ReadLoadedImageValue(imageBase, imageSize, kDynamicType + 0x50, fullKey) ||
        !ReadLoadedImageValue(imageBase, imageSize, kDynamicType + 0x78, componentSize) ||
        !ReadLoadedImageValue(imageBase, imageSize, kStateMember + 0x18, stateOffset) ||
        !ReadLoadedImageValue(imageBase, imageSize, kHangGlidingEntry + 0x10, hangGlidingValue) ||
        fullKey != kDynamicLocomotionFullKey || componentSize != kDynamicLocomotionSize ||
        stateOffset != kStateOffset || hangGlidingValue != kHangGlidingValue) return false;

    return MatchesLoadedImageName(imageBase, imageSize, kDynamicType, 0x00, 0x08,
                                  0x1C9E2C8, kDynamicLocomotionName,
                                  sizeof(kDynamicLocomotionName) - 1) &&
           MatchesLoadedImageName(imageBase, imageSize, kDynamicType, 0x20, 0x28,
                                  0x1C9E3F0, kDynamicLocomotionQualifiedName,
                                  sizeof(kDynamicLocomotionQualifiedName) - 1) &&
           MatchesRelocatedImagePointer(imageBase, imageSize, kDynamicType + 0x58,
                                        kDynamicMembers) &&
           MatchesLoadedImageName(imageBase, imageSize, kStateMember, 0x00, 0x08,
                                  0x1526AA0, kStateName, sizeof(kStateName) - 1) &&
           MatchesRelocatedImagePointer(imageBase, imageSize, kStateMember + 0x10,
                                        kLocomotionStateType) &&
           MatchesLoadedImageName(imageBase, imageSize, kLocomotionStateType, 0x00, 0x08,
                                  0x1D1B370, kLocomotionStateName,
                                  sizeof(kLocomotionStateName) - 1) &&
           MatchesLoadedImageName(imageBase, imageSize, kLocomotionStateType, 0x20, 0x28,
                                  0x1D1B558, kLocomotionStateQualifiedName,
                                  sizeof(kLocomotionStateQualifiedName) - 1) &&
           MatchesRelocatedImagePointer(imageBase, imageSize, kLocomotionStateType + 0x60,
                                        kLocomotionStateEnum) &&
           MatchesLoadedImageName(imageBase, imageSize, kHangGlidingEntry, 0x00, 0x08,
                                  0x1C70330, kHangGlidingName,
                                  sizeof(kHangGlidingName) - 1) &&
           MatchesLoadedImageBytes(imageBase, imageSize, 0x1D83FD0, kEcsPrefix,
                                   sizeof(kEcsPrefix) - 1);
}

bool ExactClientNativeImagePins(std::uintptr_t imageBase, std::size_t imageSize) noexcept {
    using namespace shroudweave::flight::native_profile_pins;
    BCRYPT_ALG_HANDLE algorithm{};
    if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0)
        return false;
    bool valid = true;
    for (const auto& pin : kClientNativeSpanPins) {
        if (pin.size == 0 || pin.rva > imageSize || pin.size > imageSize - pin.rva ||
            imageBase > (std::numeric_limits<std::uintptr_t>::max)() - pin.rva) {
            valid = false;
            break;
        }
        if (!pin.hashInLoadedImage) continue;
        BCRYPT_HASH_HANDLE hash{};
        std::array<UCHAR, 4096> object{};
        std::array<UCHAR, 32> digest{};
        if (BCryptCreateHash(algorithm, &hash, object.data(),
                             static_cast<ULONG>(object.size()), nullptr, 0, 0) < 0) {
            valid = false;
            break;
        }
        const auto* bytes = reinterpret_cast<const UCHAR*>(imageBase + pin.rva);
        const bool hashed = BCryptHashData(hash, const_cast<PUCHAR>(bytes), pin.size, 0) >= 0 &&
            BCryptFinishHash(hash, digest.data(), static_cast<ULONG>(digest.size()), 0) >= 0 &&
            std::memcmp(digest.data(), pin.sha256.data(), digest.size()) == 0;
        BCryptDestroyHash(hash);
        if (!hashed) {
            valid = false;
            break;
        }
    }
    BCryptCloseAlgorithmProvider(algorithm, 0);
    return valid;
}

bool ExactClientProfile(std::uintptr_t imageBase, std::size_t imageSize,
                        TargetLayout& layout) noexcept {
    if (imageBase == 0 || imageSize != kClientImageSize ||
        imageBase > (std::numeric_limits<std::uintptr_t>::max)() - kClampSignatureSize ||
        kPitchSiteRva > imageSize || kClampSignatureSize > imageSize - kPitchSiteRva ||
        kLowerScalarRva > imageSize || sizeof(std::uint32_t) > imageSize - kLowerScalarRva ||
        kUpperScalarRva > imageSize || sizeof(std::uint32_t) > imageSize - kUpperScalarRva ||
        !ExactClientNativeImagePins(imageBase, imageSize) ||
        !ExactClientSemanticFields(imageBase, imageSize)) {
        return false;
    }
    const auto scan = FindTargetLayout(reinterpret_cast<const std::uint8_t*>(imageBase), imageSize);
    if (scan.status != ScanStatus::found || scan.target.site_rva != kPitchSiteRva ||
        scan.target.lower_limit_rva != kLowerScalarRva ||
        scan.target.upper_limit_rva != kUpperScalarRva) return false;
    const auto* image = reinterpret_cast<const std::uint8_t*>(imageBase);
    if (std::memcmp(image + kPitchSiteRva, kClampSignature, sizeof(kClampSignature)) != 0) return false;
    constexpr std::uint32_t lowerBits = 0x3DB2B8C2u;
    constexpr std::uint32_t upperBits = 0x3FB2B8C2u;
    if (std::memcmp(image + kLowerScalarRva, &lowerBits, sizeof(lowerBits)) != 0 ||
        std::memcmp(image + kUpperScalarRva, &upperBits, sizeof(upperBits)) != 0) return false;
    layout = scan.target;
    return true;
}

bool PinFlightModule() noexcept {
    if (g_modulePinned.load(std::memory_order_acquire)) return true;
    HMODULE module{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                            reinterpret_cast<LPCWSTR>(&ClientScalarCallback), &module)) return false;
    g_pinnedFlightModule = module;
    g_modulePinned.store(true, std::memory_order_release);
    return true;
}

std::int32_t ClientScalarCallbackCore(
    const ShroudweaveNativeScalarFrameV1* sourceFrame,
    std::uint64_t context,
    std::uint32_t* selectedScalarBits,
    const ShroudweaveNativeScalarFrameV2* operands,
    std::uint32_t* selectedPitchBits) noexcept {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    const auto record = [](shroudweave::flight::PitchDecisionPath path,
                           shroudweave::client_context::Reason reason,
                           bool hasLocomotionState,
                           std::uint8_t locomotionState,
                           bool frameRspRelationKnown,
                           bool frameRspRelationMatches,
                           std::uint8_t mismatchMask=0,
                           shroudweave::client_context::Reason readError=
                               shroudweave::client_context::Reason::none) noexcept {
        shroudweave::flight::ProcessFlightDiagnostics().RecordCallback(
            path, reason, hasLocomotionState, locomotionState,
            frameRspRelationKnown, frameRspRelationMatches, mismatchMask, readError);
    };
#endif
    if (selectedScalarBits == nullptr || sourceFrame == nullptr) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        record(shroudweave::flight::PitchDecisionPath::invalid_frame,
               shroudweave::client_context::Reason::none, false, 0, false, false);
#endif
        return SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK;
    }
    *selectedScalarBits = sourceFrame->originalScalarBits;
    if (sourceFrame->cbSize != sizeof(*sourceFrame) ||
        sourceFrame->version != SHROUDWEAVE_NATIVE_SCALAR_FRAME_VERSION_V1 ||
        sourceFrame->reserved != 0 ||
        context != reinterpret_cast<std::uint64_t>(&g_callbackAnchor)) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        record(shroudweave::flight::PitchDecisionPath::invalid_frame,
               shroudweave::client_context::Reason::none, false, 0, false, false);
#endif
        return SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK;
    }
    if (!shroudweave::flight::CurrentPitchSelectionEnabled()) {
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
        record(shroudweave::flight::PitchDecisionPath::disabled,
               shroudweave::client_context::Reason::none, false, 0, false, false);
#endif
        return SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK;
    }

    const auto& profile = shroudweave::flight::CurrentNativeProfile();
    ULONG_PTR stackLow{};
    ULONG_PTR stackHigh{};
    GetCurrentThreadStackLimits(&stackLow, &stackHigh);
    const shroudweave::flight::NativeScalarFrame frame{
        sourceFrame->site, sourceFrame->nativeRbp, sourceFrame->nativeRsp,
        sourceFrame->originalScalarBits,
        operands ? operands->nativeXmm8LowBits : 0u,
        operands ? operands->nativeXmm1Lane1Bits : 0u, operands != nullptr};
    const auto decision = shroudweave::flight::EvaluateCurrentPitch(
        frame, profile, shroudweave::flight::CurrentPitchSelectionEnabledAtomic(),
        shroudweave::flight::CurrentPitchSelectionEpochAtomic(),
        shroudweave::flight::ReadCurrentNativeMemory, nullptr,
        static_cast<std::uintptr_t>(stackLow), static_cast<std::uintptr_t>(stackHigh));
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    record(decision.path, decision.localSubjectReason,
           decision.hasLocomotionState, decision.locomotionState,
           decision.frameRspRelationKnown, decision.frameRspRelationMatches,
           decision.canonicalIndexMismatchMask, decision.diagnosticReadError);
#endif
    *selectedScalarBits = decision.scalarBits;
    if (decision.overrideRequestedPitch && selectedPitchBits) {
        *selectedPitchBits=decision.requestedPitchBits;
        return SHROUDWEAVE_NATIVE_SCALAR_USE_SCALAR_AND_XMM8_OVERRIDE_V2;
    }
    return decision.path == shroudweave::flight::PitchDecisionPath::enabled_local_glider
        ? SHROUDWEAVE_NATIVE_SCALAR_USE_OVERRIDE
        : SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK;
}

std::int32_t SHROUDWEAVE_NATIVE_SCALAR_CALL ClientScalarCallback(
    const ShroudweaveNativeScalarFrameV1* frame, std::uint64_t context,
    std::uint32_t* scalar) noexcept {
    return ClientScalarCallbackCore(frame,context,scalar,nullptr,nullptr);
}

std::int32_t SHROUDWEAVE_NATIVE_SCALAR_CALL ClientScalarCallbackV2(
    const ShroudweaveNativeScalarFrameV2* frame, std::uint64_t context,
    std::uint32_t* scalar, std::uint32_t* pitch) noexcept {
    if (!frame || !scalar || !pitch || frame->base.cbSize!=sizeof(*frame) ||
        frame->base.version!=SHROUDWEAVE_NATIVE_SCALAR_FRAME_VERSION_V2)
        return SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK_V2;
    *pitch=frame->nativeXmm8LowBits;
    auto base=frame->base;
    base.cbSize=sizeof(base);
    base.version=SHROUDWEAVE_NATIVE_SCALAR_FRAME_VERSION_V1;
    return ClientScalarCallbackCore(&base,context,scalar,frame,pitch);
}

bool RegisterScalarHook(std::uintptr_t imageBase, std::uintptr_t site) noexcept {
    if (g_scalarRegistered.load(std::memory_order_acquire))
        return g_admittedImageBase == imageBase && g_admittedTarget.site_rva == kPitchSiteRva;
    if (g_scalarHandleOwned.load(std::memory_order_acquire)) return false;
    if (g_scalarRegistrationAttempted.exchange(true, std::memory_order_acq_rel)) return false;
    if (!PinFlightModule()) return false;

    ShroudweaveNativeScalarRequestV1 request{};
    request.cbSize = sizeof(request);
    request.version = SHROUDWEAVE_NATIVE_SCALAR_REQUEST_VERSION_V1;
    request.targetAddress = site;
    request.expectedScalarAddress = imageBase + kLowerScalarRva;
    std::memcpy(request.expectedInstruction, kOriginalScalarInstruction,
                sizeof(request.expectedInstruction));
    request.callback = &ClientScalarCallback;
    request.context = reinterpret_cast<std::uint64_t>(&g_callbackAnchor);

#if defined(SHROUDWEAVE_FLIGHT_TESTING) && !defined(SHROUDWEAVE_FLIGHT_TEST_REAL_SDK)
    ++g_testRegistrationAttempts;
    if (g_testRegistrationResult != SHROUDWEAVE_NATIVE_SCALAR_OK) return false;
    g_testScalarRequest = request;
    g_scalarHandleOwned.store(true, std::memory_order_release);
    g_scalarRegistered.store(true, std::memory_order_release);
    return true;
#else
    auto& sdk = ProcessNativeScalarSdk();
    if (!sdk.LoadForAddress(reinterpret_cast<const void*>(&ClientScalarCallback))) return false;
    ShroudweaveNativeScalarRequestV2 extended{};
    extended.cbSize=sizeof(extended);
    extended.version=SHROUDWEAVE_NATIVE_SCALAR_REQUEST_VERSION_V2;
    extended.targetAddress=request.targetAddress;
    extended.expectedScalarAddress=request.expectedScalarAddress;
    std::memcpy(extended.expectedInstruction,request.expectedInstruction,8);
    extended.callback=&ClientScalarCallbackV2;
    extended.context=request.context;
    const auto result = sdk.RegisterOneExtended(extended);
    if (sdk.Registered()) g_scalarHandleOwned.store(true, std::memory_order_release);
    if (result != SHROUDWEAVE_NATIVE_SCALAR_OK || !sdk.RegistrationAccepted()) return false;
    g_scalarRegistered.store(true, std::memory_order_release);
    return true;
#endif
}

void RemoveScalarHookFinal() noexcept {
    if (!g_scalarHandleOwned.load(std::memory_order_acquire)) return;
#if defined(SHROUDWEAVE_FLIGHT_TESTING) && !defined(SHROUDWEAVE_FLIGHT_TEST_REAL_SDK)
    ++g_testRemovalAttempts;
    if (g_testRemoveStatus == SHROUDWEAVE_NATIVE_SCALAR_OK) {
        g_scalarRegistered.store(false, std::memory_order_release);
        g_scalarHandleOwned.store(false, std::memory_order_release);
    }
#else
    const auto result = ProcessNativeScalarSdk().RemoveFinalWithRetries(
        kFinalScalarRemoveAttempts);
    if (result == SHROUDWEAVE_NATIVE_SCALAR_OK) {
        g_scalarRegistered.store(false, std::memory_order_release);
        g_scalarHandleOwned.store(false, std::memory_order_release);
    }
#endif
}

} // namespace

namespace shroudforge::flight {
bool prepare_scalar() noexcept {
    if(g_scalarRegistered.load(std::memory_order_acquire))return true;
    std::uintptr_t base=0;std::size_t size=0;TargetLayout layout{};
    if(!ReadClientImage(base,size)||!ExactClientProfile(base,size,layout)||
       !shroudweave::flight::AdmitCurrentClientProfile(base,size,1))return false;
    g_admittedImageBase=base;g_admittedImageSize=size;g_admittedTarget=layout;
    return RegisterScalarHook(base,base+layout.site_rva);
}
void set_scalar_enabled(bool enabled) noexcept {
    if(enabled&&g_scalarRegistered.load(std::memory_order_acquire))shroudweave::flight::EnableCurrentPitchSelection();
    else shroudweave::flight::DisableCurrentPitchSelection();
}
bool scalar_ready() noexcept {return g_scalarRegistered.load(std::memory_order_acquire);}
} // namespace shroudforge::flight
