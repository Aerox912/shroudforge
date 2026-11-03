#pragma once
#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <span>

// Private, passive call instrumentation. No engine lookup, hook, or authority API.
namespace ImpactRuntime::TerraformingObservation {
#if defined(_MSC_VER)
using StockCallback = void* (__fastcall*)(void*, const void*, const void*);
#else
using StockCallback = void* (*)(void*, const void*, const void*);
#endif
using CopyBytes = bool (*)(void*, const void*, void*, std::size_t) noexcept;
enum class FieldStatus : std::uint8_t { Unknown, Present, Absent, Unsupported, Unreadable };
enum class ValueType : std::uint8_t {
    Unknown, EntityId, WorldTransform, TerraformingType, DamageSetup, MaterialId, Float32
};
struct CheckedBytes {
    const void* data{};
    std::size_t size{};
    bool logical_extent_verified{};
};
struct TypeEvidence {
    // Host attestation for this exact operand in these invocation-scoped views.
    std::uint32_t operand_word{};
    ValueType type{};
    std::uint16_t size{};
    bool verified{};
};
struct ProgramIdentity {
    std::uint32_t opaque_state_word{};
    std::array<std::byte,16> program_guid{};
    std::uint32_t program_id{};
};
struct CheckedInvocationViews {
    std::uint64_t scope_id{};
    const void* arguments_identity{};
    const void* context_identity{};
    CheckedBytes operands, variables, data, result;
    std::array<TypeEvidence, 8> types{};
    CopyBytes copy{};
    void* copy_context{};
    bool accepted_natural_profile{};
    std::uint64_t program_identity_epoch{};
    FieldStatus program_identity_status{};
    ProgramIdentity program_identity{};
};
struct SlotSnapshot {
    FieldStatus status{};
    std::uint8_t size{};
    std::array<std::byte, 56> bytes{};
};
struct Observation {
    std::uint64_t epoch{}, thread_tag{}, ordinal{};
    std::uint64_t scope_id{};
    FieldStatus program_identity_status{};
    ProgramIdentity program_identity{};
    FieldStatus operands_status{};
    std::array<std::uint32_t, 9> operands{};
    std::array<SlotSnapshot, 8> slots{};
    FieldStatus ninth_converted_status{FieldStatus::Unknown};
    FieldStatus report_status{};
    std::uint32_t report_detail{};
    std::uint8_t report_tag{}, report_flag{};
    bool returned_result_matches{};
    // No native pointers, actor authority, tool/strike identity or settlement.
};
struct OriginalArguments { void* result{}; const void* arguments{}; const void* context{}; };
class Collector;
enum class CallStatus { Entered, InvalidLease, AlreadyInvoked };
struct CallResult { CallStatus status{CallStatus::InvalidLease}; void* value{}; };
class InvocationLease final {
public:
    InvocationLease() noexcept = default;
    InvocationLease(const InvocationLease&) = delete;
    InvocationLease& operator=(const InvocationLease&) = delete;
    // One thread owns each lease. Creation never overwrites a prior lease.
    static bool TryCreate(StockCallback original, OriginalArguments arguments,
        std::uint64_t epoch, std::uint64_t thread_tag, std::uint64_t scope_id,
        InvocationLease& output) noexcept;
private:
    friend CallResult Invoke(InvocationLease&, Collector&, const CheckedInvocationViews&);
    StockCallback original_{};
    OriginalArguments arguments_{};
    std::uint64_t epoch_{}, thread_tag_{}, scope_id_{};
    bool consumed_{};
};
struct DrainResult { bool acquired{}; std::size_t count{}; };
class Collector final {
public:
    static constexpr std::size_t capacity = 16;
    static constexpr std::uint32_t maximum_budget = 256;
    Collector() noexcept = default;
    Collector(const Collector&) = delete;
    Collector& operator=(const Collector&) = delete;
    // One-shot, explicit arm; invalid/busy attempts do not arm. Close is final.
    bool Arm(std::uint64_t epoch, std::uint32_t sample_budget) noexcept;
    void Close() noexcept;
    DrainResult TryDrain(std::span<Observation> destination) noexcept;
    std::uint64_t BusyDrops() const noexcept { return busy_.load(std::memory_order_relaxed); }
private:
    friend CallResult Invoke(InvocationLease&, Collector&, const CheckedInvocationViews&);
    bool Begin(std::uint64_t epoch, std::uint64_t& ordinal) noexcept;
    void Publish(const Observation& observation) noexcept;
    std::atomic_flag gate_ = ATOMIC_FLAG_INIT;
    std::atomic<bool> closed_{};
    std::atomic<std::uint64_t> busy_{};
    bool armed_{};
    std::uint64_t epoch_{}, ordinal_{};
    std::uint32_t remaining_{};
    std::size_t head_{}, count_{};
    std::array<Observation, capacity> queue_{};
};
// No noexcept or catch around stock: its C++/SEH behavior propagates unchanged.
// Host retains collector, original code and checked views for the whole call.
// Close stops new admission; previously admitted normal returns may publish.
// Drain/empty is NOT hook quiescence. Host owns hook retirement and lifetime.
CallResult Invoke(InvocationLease& lease, Collector& collector,
    const CheckedInvocationViews& views);
}
