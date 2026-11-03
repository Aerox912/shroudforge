#include "semantic-attribution/prepare_attribution.h"
#include "semantic-attribution/native_event_adapter.h"
#include "semantic-attribution/native_cpu_state.h"
#include <windows.h>
#include <bcrypt.h>
#include <array>
#include <atomic>
#include <cstring>
#include <limits>
#include <new>

extern "C" void SF_SemanticNativeThunk();
extern "C" void SF_SemanticNativeDispatch(SemanticAttribution::Native::SavedMachine*,const void*) noexcept;
extern "C" std::uint64_t SF_SemanticXsaveMask;
namespace SemanticAttribution::Native {
namespace {
constexpr std::size_t MaxSites=100,MaxSpans=EngineHooks::ExecutableWrite::MaxWriteSetSpans;
constexpr std::size_t MaximumOwnedBytes=128*1024*1024;
std::atomic_flag preparation_busy=ATOMIC_FLAG_INIT;
std::atomic<std::uint64_t> next_incarnation{1};
std::uint64_t take_incarnation() noexcept {
    auto value=next_incarnation.load();
    while(value && value!=UINT64_MAX) {
        if(next_incarnation.compare_exchange_weak(value,value+1))return value;
    }
    return 0;
}
struct Resident final {
    NativeEventAdapter* adapter{};
    Ledger* ledger{};
    void* owned_records{};
    void* relay_memory{};
    bool function_table_registered{},reachable{};
    std::array<RUNTIME_FUNCTION,MaxSites> functions{};
    std::array<ResidentSite,MaxSites> sites{};
    std::array<DescriptorDefinition,1024> catalog{};
    std::array<std::array<std::uint8_t,16>,MaxSites> replacements{};
    std::array<EngineHooks::ExecutableWrite::Request,MaxSpans> requests{};
    EngineHooks::ExecutableWrite::SetClaim claim{};
    HMODULE pinned_provider{};
};
// The process owns these records once code could be reachable. Deliberately no
// generic delete/unregister path is exposed after successful or partial writes.
Resident* retained_resident{};
bool hash_image_file(HMODULE module,std::string_view expected) noexcept {
    if(expected.size()!=64)return false;
    std::array<wchar_t,32768> path{};
    const auto length=GetModuleFileNameW(module,path.data(),static_cast<DWORD>(path.size()));
    if(!length || length>=path.size())return false;
    const auto file=CreateFileW(path.data(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_SEQUENTIAL_SCAN,nullptr);
    if(file==INVALID_HANDLE_VALUE)return false;
    BCRYPT_ALG_HANDLE algorithm{};BCRYPT_HASH_HANDLE hash{};
    bool good=BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)>=0;
    if(good)good=BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0)>=0;
    std::array<std::uint8_t,65536> data{};DWORD count{};
    while(good) {
        if(!ReadFile(file,data.data(),static_cast<DWORD>(data.size()),&count,nullptr)){good=false;break;}
        if(!count)break;
        if(BCryptHashData(hash,data.data(),count,0)<0){good=false;break;}
    }
    std::array<std::uint8_t,32> digest{};
    if(good)good=BCryptFinishHash(hash,digest.data(),static_cast<ULONG>(digest.size()),0)>=0;
    if(hash)BCryptDestroyHash(hash);if(algorithm)BCryptCloseAlgorithmProvider(algorithm,0);CloseHandle(file);
    if(!good)return false;
    constexpr char hex[]="0123456789abcdef";
    for(std::size_t i=0;i<digest.size();++i)if(expected[2*i]!=hex[digest[i]>>4] || expected[2*i+1]!=hex[digest[i]&15])return false;
    return true;
}
bool validate_loaded_profile(std::uintptr_t image,const Profile& profile) noexcept {
    __try {
        const auto* dos=reinterpret_cast<const IMAGE_DOS_HEADER*>(image);
        if(dos->e_magic!=IMAGE_DOS_SIGNATURE || dos->e_lfanew<=0 || dos->e_lfanew>0x100000)return false;
        const auto* nt=reinterpret_cast<const IMAGE_NT_HEADERS64*>(image+dos->e_lfanew);
        if(nt->Signature!=IMAGE_NT_SIGNATURE || nt->FileHeader.Machine!=IMAGE_FILE_MACHINE_AMD64 ||
           nt->OptionalHeader.Magic!=IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
           nt->OptionalHeader.SizeOfImage!=profile.image_size || nt->FileHeader.TimeDateStamp!=profile.timestamp)return false;
        for(const auto& site:profile.sites) {
            if(site.rva>profile.image_size-site.expected_size ||
               std::memcmp(reinterpret_cast<const void*>(image+site.rva),site.expected.data(),site.expected_size))return false;
        }
        for(const auto& range:profile.unchanged_operations)
            if(range.rva>profile.image_size-range.size ||
               std::memcmp(reinterpret_cast<const void*>(image+range.rva),range.expected.data(),range.size))return false;
        return true;
    } __except(EXCEPTION_EXECUTE_HANDLER) {return false;}
}
void* allocate_near_rw(std::uintptr_t target,std::size_t size) noexcept {
    SYSTEM_INFO info{};GetSystemInfo(&info);const auto granularity=std::uintptr_t(info.dwAllocationGranularity);
    const auto low=target>0x70000000?target-0x70000000:granularity;
    const auto high=target+0x70000000;
    for(auto cursor=low;cursor<high;) {
        MEMORY_BASIC_INFORMATION region{};
        if(!VirtualQuery(reinterpret_cast<const void*>(cursor),&region,sizeof(region)))break;
        const auto start=reinterpret_cast<std::uintptr_t>(region.BaseAddress);
        if(region.RegionSize>UINTPTR_MAX-start)break;
        const auto end=start+region.RegionSize;
        if(region.State==MEM_FREE) {
            const auto candidate=(start+granularity-1)&~(granularity-1);
            if(candidate>=low && candidate<high && candidate<=end && size<=end-candidate && size<=high-candidate)
                if(auto* p=VirtualAlloc(reinterpret_cast<void*>(candidate),size,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE))return p;
        }
        if(end<=cursor)break;cursor=end;
    }
    return nullptr;
}
void discard_unreachable(Resident* r) noexcept {
    if(!r || r->reachable)return;
    if(r->function_table_registered)RtlDeleteFunctionTable(r->functions.data());
    if(r->adapter)r->adapter->~NativeEventAdapter();
    if(r->ledger)r->ledger->~Ledger();
    if(r->owned_records)VirtualFree(r->owned_records,0,MEM_RELEASE);
    if(r->relay_memory)VirtualFree(r->relay_memory,0,MEM_RELEASE);
    r->~Resident();VirtualFree(r,0,MEM_RELEASE);
}
}
PreparationResult PrepareAttribution(const EngineHooks::HeldEntryToken& held,
    const EngineHooks::RetainedModuleOwnerToken& owner,Limits limits) noexcept {
    if(!held.ValidForCurrentThread() || !owner.IsRetained() || preparation_busy.test_and_set())return {};
    struct Finish {~Finish(){preparation_busy.clear();}} finish;
    if(retained_resident)return {};
    const auto& profile=ArchivedServerProfile();const auto image=reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    if(!image || !hash_image_file(reinterpret_cast<HMODULE>(image),profile.file_sha256) || !validate_loaded_profile(image,profile))
        return {PreparationStatus::Rejected,PreparationFailure::Profile,0};
    CpuStateSupport cpu{};
    if(!QueryCpuStateSupport(cpu))return {PreparationStatus::Rejected,PreparationFailure::CpuState,0};
    if(profile.sites.size()>MaxSites || profile.sites.size()+profile.unchanged_operations.size()>MaxSpans ||
       profile.semantic_catalog.size()>1024 || limits.max_registration_entries>1024)
        return {PreparationStatus::Rejected,PreparationFailure::Capacity,0};
    const auto incarnation=take_incarnation();
    if(!incarnation)return {PreparationStatus::Rejected,PreparationFailure::Capacity,0};
    limits.expected_adapter_incarnation=incarnation;limits.expected_profile_identity=ArchivedProfileIdentity;
    auto* raw=VirtualAlloc(nullptr,sizeof(Resident),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if(!raw)return {PreparationStatus::Rejected,PreparationFailure::Allocation,0};
    auto* resident=new(raw)Resident;
    auto reject=[&](PreparationFailure failure){discard_unreachable(resident);return PreparationResult{PreparationStatus::Rejected,failure,0};};
    if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
        reinterpret_cast<LPCWSTR>(&SF_SemanticNativeDispatch),&resident->pinned_provider))return reject(PreparationFailure::Prerequisite);
    for(std::size_t i=0;i<profile.semantic_catalog.size();++i)
        resident->catalog[i]={profile.semantic_catalog[i].hash,profile.semantic_catalog[i].key};
    const auto ledger_size=Ledger::RequiredStorage(limits);
    const auto adapter_size=NativeEventAdapter::RequiredStorage();
    if(!ledger_size || ledger_size>MaximumOwnedBytes-64 || adapter_size>MaximumOwnedBytes-ledger_size-64)
        return reject(PreparationFailure::Capacity);
    const auto owned_size=ledger_size+adapter_size+64;
    resident->owned_records=VirtualAlloc(nullptr,owned_size,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if(!resident->owned_records)return reject(PreparationFailure::Allocation);
    const auto constructed=Ledger::Construct({static_cast<std::byte*>(resident->owned_records),ledger_size},limits,
        DescriptorCatalog{std::span(resident->catalog).first(profile.semantic_catalog.size())});
    if(!constructed)return reject(PreparationFailure::Capacity);
    resident->ledger=constructed.ledger;
    const auto count=profile.sites.size();const auto relay_size=count*RelayPlan::SlotSize;
    resident->relay_memory=allocate_near_rw(image+0x680000,relay_size);
    if(!resident->relay_memory)return reject(PreparationFailure::Allocation);
    const auto base=reinterpret_cast<std::uintptr_t>(resident->relay_memory);
    for(std::size_t i=0;i<count;++i) {
        const auto& site=profile.sites[i];const auto slot=base+i*RelayPlan::SlotSize;RelayPlan plan{};
        if(!BuildRelay(site,image,slot,reinterpret_cast<std::uintptr_t>(&SF_SemanticNativeThunk),plan) ||
           !EncodeSiteCall(site,image,slot,std::span(resident->replacements[i]).first(site.expected_size)))return reject(PreparationFailure::Relay);
        std::memcpy(reinterpret_cast<void*>(slot),plan.bytes.data(),plan.bytes.size());
        resident->functions[i]={static_cast<DWORD>(i*RelayPlan::SlotSize),static_cast<DWORD>(i*RelayPlan::SlotSize+plan.code_size),
            static_cast<DWORD>(i*RelayPlan::SlotSize+RelayPlan::UnwindOffset)};
        resident->sites[i]={&site,slot,plan.before_return,plan.after_return};
        resident->requests[i]={image+site.rva,site.expected_size,site.expected.data(),resident->replacements[i].data()};
    }
    for(std::size_t i=0;i<profile.unchanged_operations.size();++i) {
        const auto& original=profile.unchanged_operations[i];
        resident->requests[count+i]={image+original.rva,original.size,original.expected.data(),original.expected.data()};
    }
    auto* adapter_storage=static_cast<std::byte*>(resident->owned_records)+ledger_size;
    resident->adapter=NativeEventAdapter::Construct({adapter_storage,adapter_size+64},*resident->ledger,incarnation,image,profile.image_size,
        std::span(resident->sites).first(count));
    if(!resident->adapter)return reject(PreparationFailure::Capacity);
    DWORD old{};
    if(!VirtualProtect(resident->relay_memory,relay_size,PAGE_EXECUTE_READ,&old) ||
       !FlushInstructionCache(GetCurrentProcess(),resident->relay_memory,relay_size))return reject(PreparationFailure::Protection);
    if(!RtlAddFunctionTable(resident->functions.data(),static_cast<DWORD>(count),base))return reject(PreparationFailure::Unwind);
    resident->function_table_registered=true;
    SF_SemanticXsaveMask=cpu.enabled_mask;
    const auto requests=std::span(resident->requests).first(count+profile.unchanged_operations.size());
    resident->claim=EngineHooks::ReserveExecutableWriteSet(held,owner,requests);
    if(!resident->claim)return reject(PreparationFailure::Reservation);
    if(!held.ValidForCurrentThread() || !owner.IsRetained())return reject(PreparationFailure::Prerequisite);
    if(!validate_loaded_profile(image,profile))return reject(PreparationFailure::Profile);
    const auto result=EngineHooks::PublishExecutableWriteSet(held,owner,resident->claim,requests);
    if(result.outcome==EngineHooks::ExecutableWrite::WriteSetOutcome::Rejected)return reject(PreparationFailure::Publication);
    // From this point every allocation, unwind row and provider must survive.
    resident->reachable=true;retained_resident=resident;
    if(result.outcome!=EngineHooks::ExecutableWrite::WriteSetOutcome::Published || !resident->adapter->PublishTransport()) {
        resident->adapter->Close();
        return {PreparationStatus::PartialPublished,PreparationFailure::Publication,result.published_count};
    }
    return {PreparationStatus::Published,PreparationFailure::None,result.published_count};
}
}
