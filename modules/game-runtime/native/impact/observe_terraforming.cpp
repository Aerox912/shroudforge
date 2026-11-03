#include "observe_terraforming.h"
#include <cstring>
#include <limits>
#include <type_traits>
#if defined(_WIN32)
#include <windows.h>
#endif

namespace ImpactRuntime::TerraformingObservation {
namespace {
constexpr std::array<std::uint8_t,8> sizes{4,56,1,20,4,4,4,4};
constexpr std::array<ValueType,8> types{ValueType::EntityId,ValueType::WorldTransform,
    ValueType::TerraformingType,ValueType::DamageSetup,ValueType::MaterialId,
    ValueType::Float32,ValueType::Float32,ValueType::Float32};
static_assert(std::is_trivially_copyable_v<Observation>);
static_assert(std::atomic<bool>::is_always_lock_free &&
    std::atomic<std::uint64_t>::is_always_lock_free);

// Extent is an explicit host assertion about logical storage, not page access.
FieldStatus copy(const CheckedInvocationViews& views, const CheckedBytes& source,
    std::size_t offset, void* destination, std::size_t size) noexcept {
    if (!source.logical_extent_verified || !views.copy) return FieldStatus::Unknown;
    if (!source.data || offset > source.size || size > source.size-offset)
        return FieldStatus::Unsupported;
    auto address=reinterpret_cast<std::uintptr_t>(source.data);
    if (offset > UINTPTR_MAX-address || size > UINTPTR_MAX-(address+offset))
        return FieldStatus::Unsupported;
    return views.copy(views.copy_context,reinterpret_cast<const void*>(address+offset),
        destination,size) ? FieldStatus::Present : FieldStatus::Unreadable;
}
void entry(const CheckedInvocationViews& views, Observation& out) noexcept {
    out.operands_status=copy(views,views.operands,0,out.operands.data(),sizeof(out.operands));
    if (out.operands_status!=FieldStatus::Present) {
        out.operands={}; // A failed copier must not publish partially copied data.
        return;
    }
    for (std::size_t i=0;i<out.slots.size();++i) {
        auto& slot=out.slots[i];
        const auto operand=out.operands[i];
        if (!operand) { slot.status=FieldStatus::Absent; continue; }
        const auto& evidence=views.types[i];
        if (!evidence.verified) continue;
        if (evidence.operand_word!=operand || evidence.type!=types[i] || evidence.size!=sizes[i]) {
            slot.status=FieldStatus::Unsupported; continue;
        }
        if (!views.variables.logical_extent_verified || !views.data.logical_extent_verified)
            continue;
        const auto record=static_cast<std::size_t>(operand&0xffffu)*24;
        if (record>views.variables.size || 24>views.variables.size-record) {
            slot.status=FieldStatus::Unsupported; continue;
        }
        std::uint16_t offset{};
        slot.status=copy(views,views.variables,record+14,&offset,sizeof(offset));
        if (slot.status!=FieldStatus::Present) continue;
        slot.status=copy(views,views.data,offset,slot.bytes.data(),sizes[i]);
        if (slot.status==FieldStatus::Present) slot.size=sizes[i];
        else slot.bytes={};
    }
}
std::uint32_t last_error() noexcept {
#if defined(_WIN32)
    return GetLastError();
#else
    return 0;
#endif
}
void restore_error(std::uint32_t value) noexcept {
#if defined(_WIN32)
    SetLastError(value);
#else
    (void)value;
#endif
}
}

bool InvocationLease::TryCreate(StockCallback original, OriginalArguments arguments,
    std::uint64_t epoch, std::uint64_t thread_tag, std::uint64_t scope_id,
    InvocationLease& output) noexcept {
    if (!original || !epoch || !scope_id || output.original_) return false;
    output.original_=original;
    output.arguments_=arguments;
    output.epoch_=epoch; output.thread_tag_=thread_tag; output.scope_id_=scope_id;
    return true;
}
bool Collector::Arm(std::uint64_t epoch, std::uint32_t budget) noexcept {
    if (!epoch || !budget || budget>maximum_budget || closed_.load(std::memory_order_acquire))
        return false;
    if (gate_.test_and_set(std::memory_order_acquire)) return false;
    const bool accepted=!armed_ && !closed_.load(std::memory_order_acquire);
    if (accepted) { armed_=true; epoch_=epoch; remaining_=budget; }
    gate_.clear(std::memory_order_release);
    return accepted;
}
void Collector::Close() noexcept { closed_.store(true,std::memory_order_release); }
bool Collector::Begin(std::uint64_t epoch, std::uint64_t& ordinal) noexcept {
    if (closed_.load(std::memory_order_acquire)) return false;
    if (gate_.test_and_set(std::memory_order_acquire)) {
        busy_.fetch_add(1,std::memory_order_relaxed); return false;
    }
    const bool admitted=armed_ && epoch==epoch_ && remaining_ &&
        !closed_.load(std::memory_order_acquire);
    if (admitted) { --remaining_; ordinal=++ordinal_; }
    gate_.clear(std::memory_order_release);
    return admitted;
}
void Collector::Publish(const Observation& observation) noexcept {
    if (gate_.test_and_set(std::memory_order_acquire)) {
        busy_.fetch_add(1,std::memory_order_relaxed); return;
    }
    if (count_<capacity) { queue_[(head_+count_)%capacity]=observation; ++count_; }
    gate_.clear(std::memory_order_release);
}
DrainResult Collector::TryDrain(std::span<Observation> destination) noexcept {
    if (gate_.test_and_set(std::memory_order_acquire)) return {};
    const auto amount=destination.size()<count_?destination.size():count_;
    for (std::size_t i=0;i<amount;++i) {
        destination[i]=queue_[head_]; queue_[head_]={};
        head_=(head_+1)%capacity; --count_;
    }
    gate_.clear(std::memory_order_release);
    return {true,amount};
}
CallResult Invoke(InvocationLease& lease, Collector& collector,
    const CheckedInvocationViews& views) {
    if (!lease.original_) return {};
    if (lease.consumed_) return {CallStatus::AlreadyInvoked,nullptr};
    lease.consumed_=true;
    Observation observation;
    const auto before=last_error();
    const bool admitted=collector.Begin(lease.epoch_,observation.ordinal);
    const bool matching=views.scope_id==lease.scope_id_ &&
        views.arguments_identity==lease.arguments_.arguments &&
        views.context_identity==lease.arguments_.context;
    if (admitted) {
        observation.epoch=lease.epoch_; observation.thread_tag=lease.thread_tag_;
        observation.scope_id=lease.scope_id_;
        if (matching) {
            if (views.accepted_natural_profile &&
                views.program_identity_epoch==lease.epoch_ &&
                views.program_identity_status==FieldStatus::Present) {
                observation.program_identity=views.program_identity;
                observation.program_identity_status=FieldStatus::Present;
            }
            entry(views,observation);
        }
    }
    restore_error(before);
    // Nothing held, no catch, no cleanup obligation crosses this call. A fault
    // consumes its lease/budget but creates no false normal-return report.
    void* returned=lease.original_(lease.arguments_.result,lease.arguments_.arguments,
        lease.arguments_.context);
    const auto after=last_error();
    if (admitted) {
        observation.returned_result_matches=returned==lease.arguments_.result;
        if (matching && observation.returned_result_matches &&
            views.result.data==lease.arguments_.result) {
            std::array<std::byte,9> report{};
            observation.report_status=copy(views,views.result,0,report.data(),report.size());
            if (observation.report_status==FieldStatus::Present) {
                observation.report_tag=std::to_integer<std::uint8_t>(report[0]);
                std::memcpy(&observation.report_detail,report.data()+4,4);
                observation.report_flag=std::to_integer<std::uint8_t>(report[8]);
            }
        }
        collector.Publish(observation);
    }
    restore_error(after);
    return {CallStatus::Entered,returned};
}
}
