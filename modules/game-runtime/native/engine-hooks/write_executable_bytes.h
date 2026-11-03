#pragma once

#include "engine-hooks/executable_write_pair_transaction.h"
#include "engine-hooks/executable_write_set_transaction.h"
#include "engine-hooks/held_entry_preparation.h"

#include <cstddef>
#include <cstdint>
#include <span>

namespace EngineHooks {
// False guarantees no publication, or verified original-byte/protection rollback.
// An indeterminate mutation/cleanup fails fast rather than returning to a caller
// that could free a potentially reachable relay. Writes are process-serialized.
bool WriteExecutableBytes(std::uintptr_t address, const void* bytes, std::size_t size,
                          const void* expected = nullptr);
bool ReplaceExecutableBytes(std::uintptr_t address, const void* expected,
                            const void* replacement, std::size_t size);
unsigned LastExecutableWriteFailure() noexcept;

// Set claims permanently reserve the exact ordered ranges for one retained
// owner generation. Publication reports any completed prefix truthfully.
ExecutableWrite::SetClaim ReserveExecutableWriteSet(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    std::span<const ExecutableWrite::Request> requests) noexcept;
ExecutableWrite::WriteSetResult PublishExecutableWriteSet(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    const ExecutableWrite::SetClaim& claim,
    std::span<const ExecutableWrite::Request> requests) noexcept;

// Pair claims reserve both exact ranges for the retained module owner and
// generation. They have no release path and are required for pair publication.
ExecutableWrite::PairClaim ReserveExecutableWritePair(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    const ExecutableWrite::Pair& pair) noexcept;
ExecutableWrite::PairResult PublishExecutableWritePair(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    const ExecutableWrite::PairClaim& claim,
    const ExecutableWrite::Pair& pair) noexcept;
}
