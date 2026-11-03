#pragma once
#include "probe_api_v1.h"
#include "../engine-hooks/held_entry_preparation.h"

// Private trusted bootstrap ABI. No mod service or numeric grant.
namespace EarlyOwnerProbe {
using RetainedValidator = bool (__cdecl*)(std::uintptr_t, std::uint64_t, const void*) noexcept;
struct OwnerProofV1 {
    std::uint32_t size{}, version{};
    std::uint64_t admitted_module{}, load_generation{}, runtime_image{};
    const void* controller_identity{};
    RetainedValidator validator{};
};
enum class PublicationStatus : std::uint32_t { Published=0, Refused=1, Indeterminate=2 };
struct PublicationResultV1 {
    std::uint32_t size{sizeof(PublicationResultV1)}, version{1};
    PublicationStatus status{PublicationStatus::Refused};
    std::uint32_t ranges_claimed{}, pair_published{}, reserved{};
    std::uint64_t relays[2]{};
    std::uint32_t relay_bytes[2]{};
};
using Publish = unsigned (__cdecl*)(const EngineHooks::HeldEntryProof*,
    const OwnerProofV1*, const SFALProbeAttachV1*, PublicationResultV1*) noexcept;
struct BootstrapHostV1 {
    std::uint32_t size{sizeof(BootstrapHostV1)}, version{1};
    std::uint64_t runtime_image{}, executable_image{};
    const EngineHooks::HeldEntryProof* held_entry{};
    Publish publish{};
};
static_assert(sizeof(OwnerProofV1)==48);
static_assert(sizeof(PublicationResultV1)==48);
static_assert(sizeof(BootstrapHostV1)==40);
}
