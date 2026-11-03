#include "build_terraforming_observation_views.h"
#include <cstring>
#include <limits>
#if defined(_WIN32)
#include <windows.h>
#endif

namespace ImpactRuntime::TerraformingObservation {
namespace {
constexpr std::array<std::uint8_t,32> server_hash{0xd7,0xba,0xa7,0x68,0xeb,0x00,0xaa,0x89,
    0x5d,0x9f,0x5a,0x1c,0xd8,0x74,0x7c,0xcf,0x66,0x85,0x8c,0x7b,0x54,0x2f,0x50,0x98,
    0x30,0x92,0xb2,0xe3,0x26,0x15,0x88,0xee};
constexpr std::array<std::uint32_t,8> hashes{0x737c8f86,0x9493a8c5,0x4835c9e9,
    0xc4b5300b,0x10e7bb7a,0xe89f7410,0xe89f7410,0xe89f7410};
constexpr std::array<std::uint16_t,8> sizes{4,56,1,20,4,4,4,4};
constexpr std::array<ValueType,8> types{ValueType::EntityId,ValueType::WorldTransform,
    ValueType::TerraformingType,ValueType::DamageSetup,ValueType::MaterialId,
    ValueType::Float32,ValueType::Float32,ValueType::Float32};
static_assert(sizeof(void*)==8 && sizeof(ProgramIdentity)==24 &&
    offsetof(ProgramIdentity,program_guid)==4 && offsetof(ProgramIdentity,program_id)==20);
struct PreserveLastError {
#if defined(_WIN32)
    DWORD value{GetLastError()};
    ~PreserveLastError() { SetLastError(value); }
#endif
};
bool range(const void* pointer,std::size_t bytes) noexcept {
    const auto address=reinterpret_cast<std::uintptr_t>(pointer);
    return !bytes || (address && bytes<=UINTPTR_MAX-address);
}
bool same(const InvocationIdentity& a,const InvocationIdentity& b) noexcept {
    return a.epoch==b.epoch && a.scope_id==b.scope_id &&
        a.original.result==b.original.result && a.original.arguments==b.original.arguments &&
        a.original.context==b.original.context;
}
}

bool AcceptedNaturalFrameProfile::TryAccept(const HostProfileAttestation& evidence,
    AcceptedNaturalFrameProfile& output) noexcept {
    output.accepted_=evidence.reference_image_sha256==server_hash &&
        evidence.contract==NaturalFrameContract::DedicatedServerD7baa20261029 &&
        evidence.loaded_image_and_code_verified && evidence.natural_invocation_contract_verified;
    return output.accepted_;
}
bool NaturalInvocationBorrow::TryCreate(const AcceptedNaturalFrameProfile& profile,
    InvocationIdentity invocation,CopyBytes copy,void* copy_context,
    NaturalInvocationBorrow& output) noexcept {
    if (output.valid_ || !profile.accepted_ || !copy || !invocation.epoch ||
        !invocation.scope_id || !invocation.original.result || !invocation.original.arguments ||
        !invocation.original.context) return false;
    output.identity_=invocation; output.copy_=copy; output.copy_context_=copy_context;
    output.valid_=true;
    return true;
}
void PreparedObservationViews::Clear() noexcept {
    views_={}; operands_={}; descriptors_={}; copy_=nullptr; copy_context_=nullptr;
    variables_=nullptr; data_=nullptr; result_=nullptr;
}
bool PreparedObservationViews::Copy(void* context,const void* source,void* destination,
    std::size_t bytes) noexcept {
    const auto& prepared=*static_cast<const PreparedObservationViews*>(context);
    if (!prepared.copy_) return false;
    if (source==prepared.operands_.data() && bytes==sizeof(prepared.operands_)) {
        std::memcpy(destination,prepared.operands_.data(),bytes); return true;
    }
    const auto address=reinterpret_cast<std::uintptr_t>(source);
    for (const auto& descriptor:prepared.descriptors_) {
        if (!descriptor.operand) continue;
        const auto record=static_cast<std::size_t>(descriptor.operand&0xffffu)*24;
        if (address==reinterpret_cast<std::uintptr_t>(prepared.variables_)+record+14 && bytes==2) {
            std::memcpy(destination,&descriptor.offset,2); return true;
        }
        if (address==reinterpret_cast<std::uintptr_t>(prepared.data_)+descriptor.offset &&
            bytes==descriptor.size)
            return prepared.copy_(prepared.copy_context_,source,destination,bytes);
    }
    if (source==prepared.result_ && bytes==9)
        return prepared.copy_(prepared.copy_context_,source,destination,bytes);
    return false;
}
ViewStatus PrepareTerraformingObservationViews(NaturalInvocationBorrow& borrow,
    const InvocationIdentity& current,PreparedObservationViews& output) noexcept {
    PreserveLastError preserve;
    output.Clear();
    if (!borrow.valid_) return ViewStatus::InvalidBorrow;
    if (borrow.consumed_) return ViewStatus::AlreadyConsumed;
    borrow.consumed_=true;
    if (!same(borrow.identity_,current)) return ViewStatus::MismatchedInvocation;
    struct Header { const void* pointer; std::uint64_t count; } arguments{};
    struct Frame { Header variables,data; } frame{};
    static_assert(sizeof(Header)==16 && sizeof(Frame)==32);
    if (!range(current.original.arguments,sizeof(arguments)) ||
        !range(current.original.context,sizeof(frame)) || !range(current.original.result,9))
        return ViewStatus::Unsupported;
    if (!borrow.copy_(borrow.copy_context_,current.original.arguments,&arguments,sizeof(arguments)) ||
        !borrow.copy_(borrow.copy_context_,current.original.context,&frame,sizeof(frame)))
        return ViewStatus::Unreadable;
    if (arguments.count!=9 || frame.variables.count>PreparedObservationViews::maximum_variables ||
        frame.data.count>PreparedObservationViews::maximum_data_bytes)
        return ViewStatus::Unsupported;
    const auto variable_bytes=static_cast<std::size_t>(frame.variables.count)*24;
    const auto data_bytes=static_cast<std::size_t>(frame.data.count);
    if (!range(arguments.pointer,36) || !range(frame.variables.pointer,variable_bytes) ||
        !range(frame.data.pointer,data_bytes)) return ViewStatus::Unsupported;
    // Local staging prevents failed or partial reads from producing trusted views.
    std::array<std::uint32_t,9> operands{};
    std::array<PreparedObservationViews::Descriptor,8> descriptors{};
    if (!borrow.copy_(borrow.copy_context_,arguments.pointer,operands.data(),sizeof(operands)))
        return ViewStatus::Unreadable;
    for (std::size_t i=0;i<descriptors.size();++i) {
        const auto word=operands[i];
        if (!word) continue;
        const auto index=static_cast<std::size_t>(word&0xffffu);
        if (index>=frame.variables.count) return ViewStatus::Unsupported;
        struct Fields { std::uint32_t hash; std::uint16_t size,offset; } fields{};
        static_assert(sizeof(Fields)==8);
        const auto address=reinterpret_cast<std::uintptr_t>(frame.variables.pointer)+index*24+8;
        if (!borrow.copy_(borrow.copy_context_,reinterpret_cast<const void*>(address),&fields,8))
            return ViewStatus::Unreadable;
        if (fields.hash!=hashes[i] || fields.size!=sizes[i] || fields.offset>data_bytes ||
            fields.size>data_bytes-fields.offset) return ViewStatus::Unsupported;
        descriptors[i]={word,fields.hash,fields.size,fields.offset};
    }
    ProgramIdentity identity{};
    bool identity_present=false;
    const auto context=reinterpret_cast<std::uintptr_t>(current.original.context);
    if (0x58<=UINTPTR_MAX-context)
        identity_present=borrow.copy_(borrow.copy_context_,reinterpret_cast<const void*>(context+0x40),
            &identity,sizeof(identity));
    if (!identity_present) identity={};
    output.operands_=operands; output.descriptors_=descriptors;
    output.copy_=borrow.copy_; output.copy_context_=borrow.copy_context_;
    output.variables_=frame.variables.pointer; output.data_=frame.data.pointer;
    output.result_=current.original.result;
    auto& views=output.views_;
    views.scope_id=current.scope_id; views.arguments_identity=current.original.arguments;
    views.context_identity=current.original.context;
    views.operands={output.operands_.data(),sizeof(output.operands_),true};
    views.variables={frame.variables.pointer,variable_bytes,true};
    views.data={frame.data.pointer,data_bytes,true};
    views.result={current.original.result,9,true};
    for (std::size_t i=0;i<descriptors.size();++i)
        if (descriptors[i].operand) views.types[i]={descriptors[i].operand,types[i],sizes[i],true};
    views.copy=PreparedObservationViews::Copy; views.copy_context=&output;
    views.accepted_natural_profile=true; views.program_identity_epoch=current.epoch;
    views.program_identity_status=identity_present?FieldStatus::Present:FieldStatus::Unknown;
    views.program_identity=identity;
    return ViewStatus::Ready;
}
}
