#pragma once
#include "observe_terraforming.h"

namespace ImpactRuntime::TerraformingObservation {
enum class NaturalFrameContract { Unknown, DedicatedServerD7baa20261029 };
struct HostProfileAttestation {
    std::array<std::uint8_t,32> reference_image_sha256{};
    NaturalFrameContract contract{};
    bool loaded_image_and_code_verified{};
    bool natural_invocation_contract_verified{};
};
// This is the trusted host's attestation, NOT runtime image authentication.
class AcceptedNaturalFrameProfile final {
public:
    static bool TryAccept(const HostProfileAttestation& evidence,
        AcceptedNaturalFrameProfile& output) noexcept;
private:
    friend class NaturalInvocationBorrow;
    bool accepted_{};
};
struct InvocationIdentity {
    OriginalArguments original;
    std::uint64_t epoch{}, scope_id{};
};
class PreparedObservationViews;
enum class ViewStatus { Ready, InvalidBorrow, MismatchedInvocation, AlreadyConsumed,
    Unreadable, Unsupported };
class NaturalInvocationBorrow final {
public:
    NaturalInvocationBorrow() noexcept = default;
    NaturalInvocationBorrow(const NaturalInvocationBorrow&) = delete;
    NaturalInvocationBorrow& operator=(const NaturalInvocationBorrow&) = delete;
    static bool TryCreate(const AcceptedNaturalFrameProfile& profile,
        InvocationIdentity invocation, CopyBytes copy, void* copy_context,
        NaturalInvocationBorrow& output) noexcept;
private:
    friend ViewStatus PrepareTerraformingObservationViews(NaturalInvocationBorrow&,
        const InvocationIdentity&, PreparedObservationViews&) noexcept;
    InvocationIdentity identity_{};
    CopyBytes copy_{};
    void* copy_context_{};
    bool valid_{}, consumed_{};
};
class PreparedObservationViews final {
public:
    static constexpr std::uint64_t maximum_variables=65536;
    static constexpr std::uint64_t maximum_data_bytes=16*1024*1024;
    PreparedObservationViews() noexcept = default;
    PreparedObservationViews(const PreparedObservationViews&) = delete;
    PreparedObservationViews& operator=(const PreparedObservationViews&) = delete;
    const CheckedInvocationViews& Views() const noexcept { return views_; }
private:
    friend ViewStatus PrepareTerraformingObservationViews(NaturalInvocationBorrow&,
        const InvocationIdentity&, PreparedObservationViews&) noexcept;
    struct Descriptor { std::uint32_t operand{}, hash{}; std::uint16_t size{}, offset{}; };
    static bool Copy(void*,const void*,void*,std::size_t) noexcept;
    void Clear() noexcept;
    CheckedInvocationViews views_{};
    std::array<std::uint32_t,9> operands_{};
    std::array<Descriptor,8> descriptors_{};
    CopyBytes copy_{};
    void* copy_context_{};
    const void* variables_{};
    const void* data_{};
    const void* result_{};
};
// Required failure clears output. Host still forwards original exactly once.
// Prepared output is one enclosing synchronous Invoke borrow, never retained.
ViewStatus PrepareTerraformingObservationViews(NaturalInvocationBorrow& borrow,
    const InvocationIdentity& current, PreparedObservationViews& output) noexcept;
}
