#include "impact/mining_host_entries.h"
#include "player-ownership/callback_player_scope.h"
#include "player-ownership/resolve_invocation_owner.h"
#include "player-ownership/vein_control_api.h"
#include "player-ownership/vein_intents.h"
#include <windows.h>
#include <atomic>
#include <cstring>
#include <intrin.h>
namespace ImpactRuntime::MiningHost {
namespace E=TerraformingEffects;
namespace O=TerraformingObservation;
namespace P=PlayerOwnership;
using namespace NativeTerraforming;
namespace {
constexpr std::uint32_t closed=0x80000000u;
std::atomic<std::uint32_t> admission{closed},entries{};
std::atomic<bool> prepared{},completed{},attempted{},shutdown{};
std::atomic<std::uint64_t> invocation_sequence{};
PreparedNativeCalls calls{};O::AcceptedNaturalFrameProfile profile{};
struct Job {
    P::OriginalWorldInterval::Lease lease;
    void* descriptor{};NativeWord query{},manager{};bool admitted{};
    explicit Job(bool acquire):lease(acquire?P::TryReadCallbackWorld():P::OriginalWorldInterval::Lease{}){}
};
struct Action {Job* job{};P::VeinIntents::Read intent{};};
struct Query {Action* action{};bool publication{};};
struct EnumerationScope {Action* action{};const void* visitor{};bool admitted{};};
thread_local Job* job{};
thread_local Action* action{};
thread_local Query* query{};
thread_local EnumerationScope* enumeration{};
bool increment(std::atomic<std::uint32_t>& counter,bool gate) noexcept {
    auto value=counter.load();
    for(unsigned i=0;i<32;++i){
        if((gate&&(value&closed))||(value&~closed)==closed-1){CloseMiningAdmission();return false;}
        if(counter.compare_exchange_weak(value,value+1))return true;
    }
    CloseMiningAdmission();return false;
}
void release_admission() noexcept {admission.fetch_sub(1);WakeByAddressAll(&admission);}
bool copy(void*,const void* source,void* out,std::size_t size) noexcept {
    const auto p=reinterpret_cast<NativeWord>(source);
    if(!p||!out||size>UINTPTR_MAX-p)return false;
    __try{std::memcpy(out,source,size);return true;}
    __except(GetExceptionCode()==EXCEPTION_ACCESS_VIOLATION||GetExceptionCode()==EXCEPTION_IN_PAGE_ERROR?EXCEPTION_EXECUTE_HANDLER:EXCEPTION_CONTINUE_SEARCH){return false;}
}
bool stack(NativeWord pointer,std::size_t size) noexcept {
    const auto* tib=reinterpret_cast<const NT_TIB*>(NtCurrentTeb());
    const auto low=reinterpret_cast<NativeWord>(tib->StackLimit),high=reinterpret_cast<NativeWord>(tib->StackBase);
    return pointer>=low&&pointer<high&&size<=high-pointer;
}
bool read_word(NativeWord pointer,NativeWord& out) noexcept {return copy(nullptr,reinterpret_cast<const void*>(pointer),&out,sizeof(out));}
bool qualify_job(Job& local,EntrySite site,void* descriptor) noexcept {
    if(!local.lease.Held()||site.return_pc!=calls.image+0x644288||!stack(site.stack_pointer,8))return false;
    const auto d=reinterpret_cast<NativeWord>(descriptor);
    if(d<=site.stack_pointer||!stack(d,12)||!read_word(d,local.query)||
        local.query<=site.stack_pointer||!stack(local.query,8)||!read_word(local.query,local.manager))return false;
    if(local.manager!=local.lease.View().manager)return false;
    local.descriptor=descriptor;return true;
}
bool scoped_copy(P::CallbackPlayerScope& scope,P::CurrentSnapshot& output,NativeWord manager) noexcept {
    bool ok=false;
    __try {ok=scope.Validate()&&scope.Manager()==manager&&scope.Copy(output)&&scope.Validate();}
    __finally {const auto error=GetLastError();if(!scope.Release())ok=false;SetLastError(error);}
    return ok;
}
bool player_copy(Job& parent,P::CurrentSnapshot& output) noexcept {
    P::CallbackPlayerScope scope(parent.lease);
    return scoped_copy(scope,output,parent.manager);
}
bool slot(const O::CheckedInvocationViews& views,unsigned index,void* out,std::size_t size) noexcept {
    if(index>=views.types.size()||!views.types[index].verified||views.types[index].size!=size)return false;
    const auto variable=static_cast<std::size_t>(views.types[index].operand_word&0xffffu)*24;
    std::uint16_t offset{};
    return views.copy(views.copy_context,static_cast<const std::byte*>(views.variables.data)+variable+14,&offset,2)&&
        views.copy(views.copy_context,static_cast<const std::byte*>(views.data.data)+offset,out,size);
}
bool qualify_action(Action& output,void* result,const void* arguments,const void* frame) noexcept {
    if(!MiningAdmissionOpen()||!job||!job->admitted||!job->lease.Held()||
       !stack(reinterpret_cast<NativeWord>(frame),0x58)||!stack(reinterpret_cast<NativeWord>(arguments),16))return false;
    NativeWord context{},descriptor{};
    if(!read_word(reinterpret_cast<NativeWord>(frame)+0x30,context)||!stack(context,0x498)||
        !read_word(context,descriptor)||descriptor!=reinterpret_cast<NativeWord>(job->descriptor))return false;
    const auto sequence=invocation_sequence.fetch_add(1)+1;
    if(!sequence){CloseMiningAdmission();return false;}
    // Checked-view ordinals identify this actual host invocation only. They are
    // not native World epochs, entity incarnations or authority substitutes.
    O::InvocationIdentity identity{{result,arguments,frame},sequence,sequence};
    O::NaturalInvocationBorrow borrow;O::PreparedObservationViews checked;
    if(!O::NaturalInvocationBorrow::TryCreate(profile,identity,copy,nullptr,borrow)||
        O::PrepareTerraformingObservationViews(borrow,identity,checked)!=O::ViewStatus::Ready)return false;
    const auto& views=checked.Views();
    constexpr std::array<std::byte,16> guid{std::byte{0xb7},std::byte{0xd2},std::byte{0x50},std::byte{0xd8},std::byte{0x0a},std::byte{0xc0},std::byte{0x69},std::byte{0x4c},std::byte{0x91},std::byte{0x85},std::byte{0x57},std::byte{0x1c},std::byte{0x4e},std::byte{0xa4},std::byte{0x4f},std::byte{0x0b}};
    if(views.program_identity_status!=O::FieldStatus::Present||views.program_identity.program_id!=2921989002u||views.program_identity.program_guid!=guid)return false;
    std::uint8_t mode{};std::uint32_t actor{};
    if(!slot(views,2,&mode,1)||mode||!slot(views,0,&actor,4)||!actor)return false;
    P::CurrentSnapshot snapshot{};
    if(!player_copy(*job,snapshot))return false;
    const auto owner=P::resolve_invocation_owner(snapshot,actor);
    if(!owner)return false;
    const auto intent=P::NativeVeinIntents().GetActive(owner->world,owner->platform_uid,GetTickCount64());
    if(!intent||intent->binding.gameplay_key!=owner->gameplay_key||intent->binding.machine_key!=owner->machine_key||
        intent->tier<1||intent->tier>3)return false;
    output={job,*intent};return true;
}
bool revalidate(const void* context) noexcept {
    const auto& original=*static_cast<const Action*>(context);
    return action==&original&&job==original.job&&job&&job->admitted&&job->lease.Held()&&
        MiningAdmissionOpen()&&P::NativeVeinIntents().Unchanged(original.intent,GetTickCount64());
}
NativeWord invoke_job(Job& local,void* descriptor) {
    Job* previous=job;job=&local;const auto error=GetLastError();NativeWord returned{};
    __try {SetLastError(error);returned=calls.job(descriptor);}
    __finally {const auto after=GetLastError();local.lease.Release();job=previous;SetLastError(after);}
    return returned;
}
NativeWord run_job(bool granted,EntrySite site,void* descriptor) {
    const auto error=GetLastError();
    Job local(granted);local.admitted=granted&&qualify_job(local,site,descriptor);
    SetLastError(error);return invoke_job(local,descriptor);
}
}
bool PrepareMiningEntries(const EngineHooks::HeldEntryToken& token,const PreparedNativeCalls& originals,const O::HostProfileAttestation& attestation) noexcept {
    bool expected=false;
    if(!attempted.compare_exchange_strong(expected,true))return false;
    if(!token.ValidForCurrentThread()||shutdown.load()||!originals.image||originals.image>UINTPTR_MAX-0xa40000||
       !originals.job||!originals.terraforming||!originals.query||!originals.enumeration||!originals.region||
       !O::AcceptedNaturalFrameProfile::TryAccept(attestation,profile)){CloseMiningAdmission();return false;}
    calls=originals;prepared.store(true);return true;
}
bool MiningEntriesPrepared() noexcept {return prepared.load();}
bool MiningAdmissionOpen() noexcept {return prepared.load()&&!(admission.load()&closed)&&!shutdown.load()&&P::CallbackPlayerWorldReady()&&P::CallbackPlayerImage()==calls.image;}
void CloseMiningAdmission() noexcept {shutdown.store(true);admission.fetch_or(closed);WakeByAddressAll(&admission);}
void DrainMiningActions() noexcept {
    CloseMiningAdmission();
    for(;;){auto current=admission.load();if(!(current&~closed))return;WaitOnAddress(&admission,&current,sizeof(current),INFINITE);}
}
std::uint32_t ActiveMiningEntries() noexcept {return entries.load();}
bool NativeDetail::CompleteMiningEntriesPreparation(bool coordinated) noexcept {
    bool expected=false;
    if(!completed.compare_exchange_strong(expected,true)||!coordinated||!prepared.load()||shutdown.load()||
       !P::CallbackPlayerWorldReady()||P::CallbackPlayerImage()!=calls.image){CloseMiningAdmission();return false;}
    std::uint32_t waiting=closed;
    if(!admission.compare_exchange_strong(waiting,0)||shutdown.load()||
       !P::CallbackPlayerWorldReady()){CloseMiningAdmission();return false;}
    return true;
}
NativeWord ForwardJob(EntrySite site,void* descriptor) {
    if(!prepared.load())__fastfail(FAST_FAIL_INVALID_ARG);
    const auto error=GetLastError();const bool counted=increment(entries,false);
    const bool granted=!job&&site.return_pc==calls.image+0x644288&&MiningAdmissionOpen()&&increment(admission,true);
    NativeWord returned{};
    __try {SetLastError(error);returned=run_job(granted,site,descriptor);}
    __finally {const auto after=GetLastError();if(granted)release_admission();if(counted)entries.fetch_sub(1);SetLastError(after);}
    return returned;
}
void* ForwardTerraforming(EntrySite,void* result,const void* arguments,const void* frame) {
    if(!prepared.load())__fastfail(FAST_FAIL_INVALID_ARG);
    const auto error=GetLastError();const bool counted=increment(entries,false);
    Action local{};Action* previous=action;action=&local;E::Admission admitted{};void* returned{};
    __try {
        if(!previous&&qualify_action(local,result,arguments,frame))admitted={true,true,static_cast<std::uint8_t>(local.intent.tier),revalidate,&local};
        SetLastError(error);returned=E::WithCallScope(admitted,calls.terraforming,result,arguments,frame);
    }
    __finally {const auto after=GetLastError();action=previous;if(counted)entries.fetch_sub(1);SetLastError(after);}
    return returned;
}
std::uint8_t ForwardQuery(EntrySite site,void* source,void* optional,void* allocator,NativeQuantity* quantity,const NativeFloatSpan* coefficients,const float* center,const void* orientation,float range,std::uint8_t mode,std::uint8_t phase) {
    if(!prepared.load())__fastfail(FAST_FAIL_INVALID_ARG);
    const auto error=GetLastError();const bool counted=increment(entries,false);
    const bool natural=action&&action->job==job&&job&&job->admitted&&
        (site.return_pc==calls.image+0x222c16||site.return_pc==calls.image+0x222da4);
    Query local{action,natural&&phase==0};Query* previous=query;query=&local;std::uint8_t returned{};
    __try {SetLastError(error);returned=E::InvokeQuery(calls.query,{source,optional,allocator,quantity,coefficients,center,orientation,range,mode,phase,natural});}
    __finally {const auto after=GetLastError();query=previous;if(counted)entries.fetch_sub(1);SetLastError(after);}
    return returned;
}
NativeWord ForwardEnumeration(EntrySite site,void* source,void* optional,void* allocator,const Int3* low,const Int3* high,NativeWord flags,const void* visitor) {
    if(!prepared.load())__fastfail(FAST_FAIL_INVALID_ARG);
    const auto error=GetLastError();const bool counted=increment(entries,false);
    Enumeration owned{};EnumerationScope local{action,visitor,false};EnumerationScope* previous=enumeration;enumeration=&local;
    Int3 lower{},upper{};NativeWord returned{};
    __try {
        if(query&&query->publication&&query->action==action&&action&&action->job==job&&
            site.return_pc==calls.image+0xa33241&&E::CurrentRegionBinding().admitted&&
            copy(nullptr,low,&lower,sizeof(lower))&&copy(nullptr,high,&upper,sizeof(upper))&&PrepareEnumeration(visitor,lower,upper,owned)){
            low=&owned.low;high=&owned.high;visitor=owned.visitor.data();local.visitor=visitor;local.admitted=true;
        }
        SetLastError(error);returned=calls.enumeration(source,optional,allocator,low,high,flags,visitor);
    }
    __finally {const auto after=GetLastError();enumeration=previous;if(counted)entries.fetch_sub(1);SetLastError(after);}
    return returned;
}
std::uint8_t ForwardRegion(EntrySite site,const void* visitor,NativeCellBuffer* buffer,void* opaque,const Int3* offset,const UInt3* extent) {
    if(!prepared.load())__fastfail(FAST_FAIL_INVALID_ARG);
    const auto error=GetLastError();const bool counted=increment(entries,false);std::uint8_t returned{};
    __try {
        SetLastError(error);
        if(enumeration&&enumeration->admitted&&enumeration->action==action&&enumeration->visitor==visitor&&site.return_pc==calls.image+0xa28298)
            returned=InvokeRegion(calls.region,{visitor,buffer,opaque,offset,extent});
        else returned=calls.region(visitor,buffer,opaque,offset,extent);
    }
    __finally {const auto after=GetLastError();if(counted)entries.fetch_sub(1);SetLastError(after);}
    return returned;
}
}
extern "C" {
std::uintptr_t SF_MiningJobBridge(std::uintptr_t sp,std::uintptr_t pc,void* d){return ImpactRuntime::MiningHost::ForwardJob({pc,sp},d);}
void* SF_MiningTerraformingBridge(std::uintptr_t sp,std::uintptr_t pc,void* r,const void* a,const void* f){return ImpactRuntime::MiningHost::ForwardTerraforming({pc,sp},r,a,f);}
std::uint8_t SF_MiningQueryBridge(std::uintptr_t sp,std::uintptr_t pc,void* s,void* o,void* a,NativeTerraforming::NativeQuantity* q,const NativeTerraforming::NativeFloatSpan* c,const float* p,const void* v,float r,std::uint8_t m,std::uint8_t h){return ImpactRuntime::MiningHost::ForwardQuery({pc,sp},s,o,a,q,c,p,v,r,m,h);}
std::uintptr_t SF_MiningEnumerationBridge(std::uintptr_t sp,std::uintptr_t pc,void* s,void* o,void* a,const NativeTerraforming::Int3* l,const NativeTerraforming::Int3* h,std::uintptr_t flags,const void* v){return ImpactRuntime::MiningHost::ForwardEnumeration({pc,sp},s,o,a,l,h,flags,v);}
std::uint8_t SF_MiningRegionBridge(std::uintptr_t sp,std::uintptr_t pc,const void* v,NativeTerraforming::NativeCellBuffer* b,void* a,const NativeTerraforming::Int3* o,const NativeTerraforming::UInt3* e){return ImpactRuntime::MiningHost::ForwardRegion({pc,sp},v,b,a,o,e);}
}
