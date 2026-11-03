#pragma once
#include "impact/terraforming_call_scope.h"
#include "impact/build_terraforming_observation_views.h"
#include "engine-hooks/held_entry_preparation.h"
#include <cstdint>
namespace ImpactRuntime::MiningHost {
using NativeWord=std::uintptr_t;
using OriginalJob=NativeWord(__fastcall*)(void*);
using OriginalEnumeration=NativeWord(__fastcall*)(void*,void*,void*,const NativeTerraforming::Int3*,const NativeTerraforming::Int3*,NativeWord,const void*);
struct EntrySite {NativeWord return_pc{},stack_pointer{};};
struct PreparedNativeCalls {
    NativeWord image{};OriginalJob job{};
    TerraformingEffects::StockInvocation terraforming{};
    TerraformingEffects::NativeQuery query{};
    OriginalEnumeration enumeration{};NativeTerraforming::NativeTransform region{};
};
// Root has already prepared every exact bypass and verified the loaded code.
// This validates the real checked-frame attestation before publishing the table.
bool PrepareMiningEntries(const EngineHooks::HeldEntryToken&,const PreparedNativeCalls&,
    const TerraformingObservation::HostProfileAttestation&) noexcept;
bool MiningEntriesPrepared() noexcept;
bool MiningAdmissionOpen() noexcept;
void CloseMiningAdmission() noexcept;
// Drains admitted original jobs/World leases only. No native-code retirement.
void DrainMiningActions() noexcept;
std::uint32_t ActiveMiningEntries() noexcept;
namespace NativeDetail {
// Hook owner alone calls with the actual overall held-coordinator result.
// One-shot completion, after RunHeldEntryPreparation has returned.
bool CompleteMiningEntriesPreparation(bool coordinated) noexcept;
}
NativeWord ForwardJob(EntrySite,void*);
void* ForwardTerraforming(EntrySite,void*,const void*,const void*);
std::uint8_t ForwardQuery(EntrySite,void*,void*,void*,NativeTerraforming::NativeQuantity*,
    const NativeTerraforming::NativeFloatSpan*,const float*,const void*,float,std::uint8_t,std::uint8_t);
NativeWord ForwardEnumeration(EntrySite,void*,void*,void*,const NativeTerraforming::Int3*,
    const NativeTerraforming::Int3*,NativeWord,const void*);
std::uint8_t ForwardRegion(EntrySite,const void*,NativeTerraforming::NativeCellBuffer*,void*,
    const NativeTerraforming::Int3*,const NativeTerraforming::UInt3*);
}
extern "C" {
std::uintptr_t SF_MiningJobEntry(void*) noexcept(false);
void* SF_MiningTerraformingEntry(void*,const void*,const void*) noexcept(false);
std::uint8_t SF_MiningQueryEntry(void*,void*,void*,NativeTerraforming::NativeQuantity*,
    const NativeTerraforming::NativeFloatSpan*,const float*,const void*,float,std::uint8_t,std::uint8_t) noexcept(false);
std::uintptr_t SF_MiningEnumerationEntry(void*,void*,void*,const NativeTerraforming::Int3*,
    const NativeTerraforming::Int3*,std::uintptr_t,const void*) noexcept(false);
std::uint8_t SF_MiningRegionEntry(const void*,NativeTerraforming::NativeCellBuffer*,void*,
    const NativeTerraforming::Int3*,const NativeTerraforming::UInt3*) noexcept(false);
}
