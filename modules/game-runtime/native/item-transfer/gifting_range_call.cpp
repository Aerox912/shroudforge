#include "item-transfer/gifting_range_call.h"

#include "engine-hooks/allocate_near_code.h"
#include "engine-hooks/write_executable_bytes.h"
#include "host/runtime_lifecycle_state.h"
#include "item-transfer/gifting_range_image_validation.h"
#include "profiles/game_profile.h"

#include <windows.h>
#include <intrin.h>

#include <array>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <limits>

namespace {
using KfcRuntimeCompatibility::EnshroudedClient::image_target;
constexpr std::array<std::uint8_t, 32> caller_sha256{
    0x7e,0x75,0xeb,0x0d,0x5c,0xb0,0x79,0x1c,0xdf,0x19,0xe2,0xcc,0x9f,0x36,0xc4,0x6b,
    0xbf,0x51,0xc9,0xc4,0xb0,0x4e,0x01,0xe6,0xcf,0xb2,0xbc,0x38,0x3b,0x2c,0xa2,0x85};
constexpr std::array<std::uint8_t, 32> getter_sha256{
    0x0b,0x16,0x94,0xd0,0xa8,0x18,0xfd,0xfd,0x01,0xac,0x33,0x27,0x22,0x47,0xe3,0x6d,
    0xe2,0x9c,0x72,0x67,0x22,0xa2,0x37,0x78,0xa1,0x52,0xc5,0x45,0xdc,0x63,0xcf,0x69};

GiftingRange::PublicationGate publication_gate;
std::atomic<GiftingRange::DistanceGetter> original_getter{};
std::atomic<void*> process_relay{};

bool read_main_module_memory(const void* context, std::uintptr_t address,
                             void* output, std::size_t size) noexcept {
    if (!context || !output || !size) return false;
    const auto module_base = *static_cast<const std::uintptr_t*>(context);
    if (!module_base || address < module_base ||
        size > std::numeric_limits<std::uintptr_t>::max() - address)
        return false;
    MEMORY_BASIC_INFORMATION region{};
    if (VirtualQuery(reinterpret_cast<const void*>(address), &region,
            sizeof(region)) != sizeof(region) ||
        region.AllocationBase != reinterpret_cast<void*>(module_base) ||
        region.Type != MEM_IMAGE || region.State != MEM_COMMIT ||
        (region.Protect & (PAGE_NOACCESS | PAGE_GUARD)))
        return false;
    const auto region_base = reinterpret_cast<std::uintptr_t>(region.BaseAddress);
    if (address < region_base || size > region.RegionSize ||
        address - region_base > region.RegionSize - size)
        return false;
    SIZE_T actual{};
    return ReadProcessMemory(GetCurrentProcess(), reinterpret_cast<const void*>(address),
        output, size, &actual) != FALSE && actual == size;
}

bool validate_loaded_image(std::uintptr_t base) noexcept {
    if (!base || image_target != "enshrouded_server.exe") return false;
    const GiftingRange::ImageValidation::CodeDigests expected{
        caller_sha256, getter_sha256};
    std::uint32_t validated_size{};
    return GiftingRange::ImageValidation::validate_loaded_image(
        base, &read_main_module_memory, &base, expected, validated_size);
}

float __cdecl gifting_distance_callback() noexcept {
    const auto original = original_getter.load(std::memory_order_acquire);
    if (!original) __fastfail(FAST_FAIL_INVALID_ARG);
    return GiftingRange::TransferDistance(publication_gate, original);
}

bool prepare_callsite(const EngineHooks::HeldEntryToken& token, void*) noexcept {
    const auto provider_ready = ProviderImplementation::provider_initialized.load(std::memory_order_acquire);
    if (!token.ValidForCurrentThread() ||
        !GiftingRange::SelectsDedicatedServerInstall(1, image_target == "enshrouded_server.exe", provider_ready)) return false;
    const auto image = reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    if (!image || !validate_loaded_image(image)) return false;

    HMODULE pinned{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            reinterpret_cast<LPCWSTR>(&gifting_distance_callback), &pinned) || !pinned) return false;

    using namespace GiftingRange::ImageValidation;
    auto expected_getter = reinterpret_cast<GiftingRange::DistanceGetter>(
        image + getter_rva);
    GiftingRange::DistanceGetter empty{};
    if (!original_getter.compare_exchange_strong(empty, expected_getter,
            std::memory_order_acq_rel, std::memory_order_acquire)) return false;

    const auto site = image + call_rva;
    auto* relay = static_cast<std::uint8_t*>(EngineHooks::AllocateNearCode(site, 16));
    if (!relay) return false;
    // An indirect absolute jump leaves the caller's ABI and the callback's
    // return address intact. The page is sealed RX and flushed before publish.
    std::array<std::uint8_t, 14> relay_bytes{0xff,0x25,0,0,0,0};
    const auto destination = reinterpret_cast<std::uintptr_t>(&gifting_distance_callback);
    std::memcpy(relay_bytes.data() + 6, &destination, sizeof(destination));
    std::memcpy(relay, relay_bytes.data(), relay_bytes.size());
    DWORD previous_protection{};
    if (!VirtualProtect(relay, 16, PAGE_EXECUTE_READ, &previous_protection) ||
        !FlushInstructionCache(GetCurrentProcess(), relay, relay_bytes.size())) {
        VirtualFree(relay, 0, MEM_RELEASE);
        return false;
    }
    std::array<std::uint8_t, stock_call.size()> replacement{};
    if (!GiftingRange::BuildDirectCallPatch(site, stock_call, image + getter_rva,
            reinterpret_cast<std::uintptr_t>(relay), replacement)) {
        VirtualFree(relay, 0, MEM_RELEASE);
        return false;
    }

    // Store the relay before the write can make it reachable. If the writer
    // publishes, no path frees this allocation or unpins the provider.
    void* no_relay{};
    if (!process_relay.compare_exchange_strong(no_relay, relay,
            std::memory_order_acq_rel, std::memory_order_acquire)) {
        VirtualFree(relay, 0, MEM_RELEASE);
        return false;
    }
    if (!token.ValidForCurrentThread() ||
        !EngineHooks::ReplaceExecutableBytes(site, stock_call.data(), replacement.data(), replacement.size())) {
        process_relay.store(nullptr, std::memory_order_release);
        VirtualFree(relay, 0, MEM_RELEASE);
        return false;
    }
    return true;
}
}

namespace GiftingRange {
bool PrepareDedicatedServer(const EngineHooks::HeldEntryProof* proof, unsigned demand) noexcept {
    if (demand == 0) return true;
    if (demand != 1 || !publication_gate.MarkPrepared()) return false;
    const bool dedicated_server = image_target == "enshrouded_server.exe";
    const bool provider_ready = ProviderImplementation::provider_initialized.load(std::memory_order_acquire);
    if (!SelectsDedicatedServerInstall(demand, dedicated_server, provider_ready) || !proof) {
        publication_gate.Complete(false);
        return false;
    }
    // Complete only after RunHeldEntryPreparation returns its actual final
    // proof result. A post-callback failure therefore leaves the relay closed.
    const auto coordinated = EngineHooks::RunHeldEntryPreparation(*proof, &prepare_callsite, nullptr);
    const auto active = publication_gate.Complete(coordinated);
    return coordinated && active;
}
}
