#pragma once
#include <array>
#include <cstddef>
#include <cstdint>

namespace NamedServiceImplementation { struct BindingContract; }
namespace EngineContext { struct EcsView; }

namespace ImpactRuntime {
enum class ResolveResult : std::uint32_t {
    Found = 1,
    RegistryUnavailable = 3,
    SessionChanged = 4,
    UnsupportedContract = 5,
};

// Only a main-image code pointer and its declared arity leave the borrowed
// registry snapshot. This grants no generic right to invoke the operation.
struct RegisteredOperation {
    std::uintptr_t callback{};
    std::uint8_t arguments{};
    std::array<char, 32> name{};
};

// Passive metadata for the one supported diagnostic lookup. Unlike
// RegisteredOperation, this result contains no process address.
struct RegisteredOperationMetadata {
    std::uint32_t callback_rva{};
    std::uint32_t image_size{};
    std::uint64_t session_generation{};
    std::uint8_t arguments{};
    std::array<char, 32> name{};
    bool operator==(const RegisteredOperationMetadata&) const = default;
};

// The caller prepares the impact::ImpactSystem binding before acquiring the
// original Scene/lifetime gates and validates owner under those gates. This
// bounded reader acquires no mutex, allocates no memory, queries no session and
// invokes no engine callback. Borrowed registry data never leaves this call.
// Failure leaves output unchanged. A resolved address grants no invocation ABI.
ResolveResult ResolveRegisteredOperationPinned(const NamedServiceImplementation::BindingContract& binding,
    const EngineContext::EcsView& owner, const char* exact_name, RegisteredOperation& output);

// Resolves only the registered "terraforming" name and returns its bounded
// main-image RVA and scalar registry metadata. The caller owns and validates
// the current Scene/session gates. No callback address leaves this method and
// no engine callback is invoked.
ResolveResult ResolveRegisteredTerraformingMetadataPinned(
    const NamedServiceImplementation::BindingContract& binding,
    const EngineContext::EcsView& owner, std::uint64_t session_generation,
    RegisteredOperationMetadata& output);

// Resolves the exact original operation name in impact::ImpactSystem. Existing
// profile-backed DI/Scene ownership remains required. All Session/Scene/caller
// locks are released before this returns. No engine callback is invoked here.
// Failure leaves output unchanged.
ResolveResult ResolveRegisteredOperation(std::uint64_t expected_session,
    const char* exact_name, RegisteredOperation& output);
}
