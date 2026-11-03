#pragma once
#include "early-probe/host_api.h"
#include "engine-hooks/allocate_near_code.h"
#include "engine-hooks/executable_write_pair_transaction.h"
#include "engine-hooks/write_executable_bytes.h"
#include <windows.h>
#include <array>
#include <atomic>
#include <cstring>
#include <limits>
#include <optional>
#include <mutex>

namespace EarlyOwnerProbe {
inline bool image_code(std::uintptr_t image,std::uintptr_t address) noexcept {
    MEMORY_BASIC_INFORMATION memory{};
    if(!image || !address || VirtualQuery(reinterpret_cast<void*>(address),&memory,sizeof(memory))!=sizeof(memory) ||
       memory.State!=MEM_COMMIT || memory.Type!=MEM_IMAGE ||
       reinterpret_cast<std::uintptr_t>(memory.AllocationBase)!=image || (memory.Protect&(PAGE_GUARD|PAGE_NOACCESS)))return false;
    const auto protection=memory.Protect&0xff;
    return protection==PAGE_EXECUTE_READ || protection==PAGE_EXECUTE_READWRITE || protection==PAGE_EXECUTE_WRITECOPY;
}

// Only this dedicated host issuer can construct the writer's retained token.
// The callback is checked against the actual SF runtime image before invoking
// the Rust admission registry. The feature DLL supplies neither this callback
// nor a token, and the token never replaces the live HeldEntryToken.
class RetainedOwnerIssuer final {
public:
    static std::optional<EngineHooks::RetainedModuleOwnerToken> Issue(
        const EngineHooks::HeldEntryToken& held,const OwnerProofV1& owner) noexcept {
        if(!held.ValidForCurrentThread() || owner.size!=sizeof(owner) || owner.version!=1 ||
           !owner.admitted_module || !owner.load_generation || !owner.controller_identity || !owner.validator ||
           !image_code(owner.runtime_image,reinterpret_cast<std::uintptr_t>(owner.validator)) ||
           !owner.validator(owner.admitted_module,owner.load_generation,owner.controller_identity))return std::nullopt;
        EngineHooks::RetainedModuleOwnerToken token(owner.admitted_module,owner.load_generation,
            owner.controller_identity,owner.validator);
        return std::optional<EngineHooks::RetainedModuleOwnerToken>{std::move(token)};
    }
};

struct PreparationState final {
    DWORD owner_thread{GetCurrentThreadId()};
    OwnerProofV1 owner{};
    SFALProbeAttachV1 attach{};
    PublicationResultV1 result{};
    std::array<std::array<unsigned char,5>,2> replacements{};
    std::array<void*,2> relays{};
    std::optional<EngineHooks::ExecutableWrite::PairClaim> claim;
};
inline PreparationState* retained_preparation{}; // Deliberately never destroyed.
inline std::atomic<bool> preparation_attempted{};
inline std::mutex preparation_mutex;

inline bool validate_attachment(const PreparationState& state) noexcept {
    const auto image=reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    const auto& attach=state.attach;
    if(!image || image>UINTPTR_MAX-0x65cb40 || attach.struct_size!=sizeof(attach) || attach.abi_version!=1 ||
       attach.admitted_load_generation!=state.owner.load_generation || attach.transfer_capability ||
       attach.required_host_services!=(SFAL_PROBE_HOST_PROCESS_RETENTION_V1|SFAL_PROBE_HOST_ATOMIC_CALL_PAIR_V1) ||
       attach.required_capture_fields!=(SFAL_PROBE_FIELD_STACK_FRAME_V1|SFAL_PROBE_FIELD_EXACT_CALLSITE_V1|SFAL_PROBE_FIELD_CALLBACK_CALLER_V1) ||
       attach.optional_capture_fields!=(SFAL_PROBE_FIELD_STACK_QUERY_TAGS_V1|SFAL_PROBE_FIELD_HOST_CONTEXT_V1))return false;
    constexpr std::uintptr_t sites[]{0x19fb1e,0x1a020f},originals[]{0x65cb40,0x1af4a0};
    constexpr unsigned char bytes[][5]{{0xe8,0x1d,0xd0,0x4b,0},{0xe8,0x8c,0xf2,0,0}};
    for(unsigned i=0;i<2;++i){
        const auto& call=attach.calls[i];
        if(call.callsite!=image+sites[i] || call.original_entry!=image+originals[i] ||
           std::memcmp(call.expected_call,bytes[i],5) || call.reserved[0] || call.reserved[1] || call.reserved[2] ||
           !image_code(image,call.callsite) || !image_code(image,call.original_entry) ||
           !image_code(state.owner.admitted_module,call.probe_entry))return false;
    }
    return attach.calls[0].probe_entry!=attach.calls[1].probe_entry;
}

inline bool prepare_under_gate(const EngineHooks::HeldEntryToken& held,void* context) noexcept {
    auto& state=*static_cast<PreparationState*>(context);
    auto owner=RetainedOwnerIssuer::Issue(held,state.owner);
    if(!owner || !validate_attachment(state))return false;
    HMODULE provider{};
    if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
        reinterpret_cast<LPCWSTR>(&prepare_under_gate),&provider))return false;
    for(unsigned i=0;i<2;++i){
        const auto& call=state.attach.calls[i];
        auto* relay=static_cast<unsigned char*>(EngineHooks::AllocateNearCode(call.callsite,16));
        state.relays[i]=relay; // Keep even failed preparation; no Free executable memory.
        if(!relay)return false;
        const unsigned char jump[]{0xff,0x25,0,0,0,0};
        std::memcpy(relay,jump,sizeof(jump));std::memcpy(relay+6,&call.probe_entry,sizeof(call.probe_entry));
        relay[14]=relay[15]=0xcc;
        DWORD previous{};
        if(!VirtualProtect(relay,16,PAGE_EXECUTE_READ,&previous) ||
           !FlushInstructionCache(GetCurrentProcess(),relay,16))return false;
        const auto delta=static_cast<std::int64_t>(reinterpret_cast<std::uintptr_t>(relay))-
            static_cast<std::int64_t>(call.callsite+5);
        if(delta<INT32_MIN || delta>INT32_MAX)return false;
        state.replacements[i][0]=0xe8;const auto rel32=static_cast<std::int32_t>(delta);
        std::memcpy(state.replacements[i].data()+1,&rel32,4);
        state.result.relays[i]=reinterpret_cast<std::uintptr_t>(relay);state.result.relay_bytes[i]=16;
    }
    const EngineHooks::ExecutableWrite::Pair pair{{
        {state.attach.calls[0].callsite,5,state.attach.calls[0].expected_call,state.replacements[0].data()},
        {state.attach.calls[1].callsite,5,state.attach.calls[1].expected_call,state.replacements[1].data()}}};
    auto claim=EngineHooks::ReserveExecutableWritePair(held,*owner,pair);
    if(!claim)return false;
    state.result.ranges_claimed=1;state.claim.emplace(std::move(claim));
    const auto publication=EngineHooks::PublishExecutableWritePair(held,*owner,*state.claim,pair);
    if(publication.outcome==EngineHooks::ExecutableWrite::PairOutcome::Indeterminate){
        state.result.status=PublicationStatus::Indeterminate;return false;
    }
    if(publication.outcome!=EngineHooks::ExecutableWrite::PairOutcome::Published)return false;
    state.result.pair_published=1;
    // Publication is already durable. A failed postcondition is unknown, not
    // an unchanged refusal, and cannot authorize a retry/unload/unowned restore.
    for(unsigned i=0;i<2;++i){
        std::array<unsigned char,16> expected_relay{0xff,0x25,0,0,0,0};
        std::memcpy(expected_relay.data()+6,&state.attach.calls[i].probe_entry,8);
        expected_relay[14]=expected_relay[15]=0xcc;
        std::array<unsigned char,16> actual_relay{};std::array<unsigned char,5> actual_call{};
        SIZE_T copied{};MEMORY_BASIC_INFORMATION memory{};
        if(!ReadProcessMemory(GetCurrentProcess(),state.relays[i],actual_relay.data(),actual_relay.size(),&copied) || copied!=actual_relay.size() ||
           actual_relay!=expected_relay ||
           VirtualQuery(state.relays[i],&memory,sizeof(memory))!=sizeof(memory) || memory.State!=MEM_COMMIT || memory.Protect!=PAGE_EXECUTE_READ ||
           !ReadProcessMemory(GetCurrentProcess(),reinterpret_cast<void*>(state.attach.calls[i].callsite),actual_call.data(),actual_call.size(),&copied) ||
           copied!=actual_call.size() || actual_call!=state.replacements[i]) {
            state.result.status=PublicationStatus::Indeterminate;return false;
        }
    }
    state.result.status=PublicationStatus::Published;
    return true;
}

inline unsigned Prepare(const EngineHooks::HeldEntryProof* proof,const OwnerProofV1* owner,
    const SFALProbeAttachV1* attach,PublicationResultV1* output) noexcept {
    if(!proof || !owner || !attach || !output)return static_cast<unsigned>(PublicationStatus::Refused);
    std::lock_guard lock(preparation_mutex);
    bool expected=false;
    if(!preparation_attempted.compare_exchange_strong(expected,true)) {
        // The exact prior outcome is observable, but never retried/rebound.
        if(!retained_preparation || retained_preparation->owner_thread!=GetCurrentThreadId() ||
           proof->size!=sizeof(*proof) || proof->version!=1 || !proof->is_held || !proof->is_held(proof->parked_thread_id) ||
           owner->admitted_module!=retained_preparation->owner.admitted_module ||
           owner->load_generation!=retained_preparation->owner.load_generation ||
           owner->runtime_image!=retained_preparation->owner.runtime_image || owner->validator!=retained_preparation->owner.validator ||
           owner->controller_identity!=retained_preparation->owner.controller_identity ||
           std::memcmp(attach,&retained_preparation->attach,sizeof(*attach)))return static_cast<unsigned>(PublicationStatus::Refused);
        *output=retained_preparation->result;return static_cast<unsigned>(output->status);
    }
    try {
        retained_preparation=new PreparationState;
        auto& state=*retained_preparation;state.owner=*owner;state.attach=*attach;
        const bool completed=EngineHooks::RunHeldEntryPreparation(*proof,&prepare_under_gate,&state);
        if(!completed && state.result.pair_published)state.result.status=PublicationStatus::Indeterminate;
        *output=state.result;return static_cast<unsigned>(output->status);
    }catch(...){
        if(retained_preparation && retained_preparation->result.pair_published){
            retained_preparation->result.status=PublicationStatus::Indeterminate;
            *output=retained_preparation->result;
            return static_cast<unsigned>(PublicationStatus::Indeterminate);
        }
        return static_cast<unsigned>(PublicationStatus::Refused);
    }
}
}
