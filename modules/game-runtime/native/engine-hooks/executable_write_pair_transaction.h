#pragma once

#include "engine-hooks/executable_write_transaction.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <utility>

namespace EarlyOwnerProbe {
class RetainedOwnerIssuer;
}

namespace EngineHooks {
namespace ExecutableWrite::Detail {
class PairReservationAccess;
template<std::size_t Capacity>
class PairReservationTable;
}

class RetainedModuleOwnerToken final {
public:
    RetainedModuleOwnerToken(const RetainedModuleOwnerToken&) = delete;
    RetainedModuleOwnerToken& operator=(const RetainedModuleOwnerToken&) = delete;

    RetainedModuleOwnerToken(RetainedModuleOwnerToken&& other) noexcept
        : admitted_module_(std::exchange(other.admitted_module_, 0)),
          load_generation_(std::exchange(other.load_generation_, 0)),
          controller_identity_(std::exchange(other.controller_identity_, nullptr)),
          validator_(std::exchange(other.validator_, nullptr)) {}

    RetainedModuleOwnerToken& operator=(RetainedModuleOwnerToken&& other) noexcept {
        if (this != &other) {
            admitted_module_ = std::exchange(other.admitted_module_, 0);
            load_generation_ = std::exchange(other.load_generation_, 0);
            controller_identity_ = std::exchange(other.controller_identity_, nullptr);
            validator_ = std::exchange(other.validator_, nullptr);
        }
        return *this;
    }

    bool IsRetained() const noexcept {
        return admitted_module_ && load_generation_ && controller_identity_ &&
            validator_ && validator_(admitted_module_, load_generation_, controller_identity_);
    }

private:
    friend class ::EarlyOwnerProbe::RetainedOwnerIssuer;
    friend class ExecutableWrite::Detail::PairReservationAccess;

    RetainedModuleOwnerToken(std::uintptr_t admitted_module,
                             std::uint64_t load_generation,
                             const void* controller_identity,
                             bool (*validator)(std::uintptr_t, std::uint64_t, const void*) noexcept) noexcept
        : admitted_module_(admitted_module),
          load_generation_(load_generation),
          controller_identity_(controller_identity),
          validator_(validator) {}

    std::uintptr_t admitted_module_{};
    std::uint64_t load_generation_{};
    const void* controller_identity_{};
    bool (*validator_)(std::uintptr_t, std::uint64_t, const void*) noexcept{};
};
}

namespace EngineHooks::ExecutableWrite {

struct Request {
    std::uintptr_t address{};
    std::size_t size{};
    const void* expected{};
    const void* replacement{};
};

using Pair = std::array<Request, 2>;

enum class PairOutcome : unsigned {
    Rejected,
    Published,
    Indeterminate,
};

enum class PairFailure : unsigned {
    None,
    InvalidRange,
    Overlap,
    InvalidHeldEntryToken,
    OwnerNotRetained,
    ReservationConflict,
    ReservationCapacity,
    InvalidClaim,
    Enumeration,
    Capacity,
    OpenThread,
    SuspendThread,
    ThreadContext,
    OccupiedPc,
    UnstableInventory,
    ExpectedBytes,
    Protect,
    Flush,
    Verification,
    RestoreProtection,
    ResumeThread,
};

struct PairResult {
    // Rejected means no write occurred or both original byte ranges and
    // protections were restored and verified. Published means both replacement
    // ranges and original protections were verified. Indeterminate is never
    // a normal rejection; the production writer fails fast on this outcome.
    PairOutcome outcome{PairOutcome::Rejected};
    PairFailure failure{PairFailure::None};
};

// Added raw diagnostic value returned by LastExecutableWriteFailure().
inline constexpr unsigned LegacyWriteBlockedByPairReservation = 0x10000U;
inline constexpr unsigned LegacyWriteBlockedByWriteSetReservation = 0x10001U;

class PairClaim final {
public:
    PairClaim() noexcept = default;
    PairClaim(const PairClaim&) = delete;
    PairClaim& operator=(const PairClaim&) = delete;

    PairClaim(PairClaim&& other) noexcept
        : table_(std::exchange(other.table_, nullptr)),
          slot_(std::exchange(other.slot_, InvalidSlot)),
          ticket_(std::exchange(other.ticket_, 0)) {}

    PairClaim& operator=(PairClaim&& other) noexcept {
        if (this != &other) {
            table_ = std::exchange(other.table_, nullptr);
            slot_ = std::exchange(other.slot_, InvalidSlot);
            ticket_ = std::exchange(other.ticket_, 0);
        }
        return *this;
    }

    explicit operator bool() const noexcept {
        return table_ && ticket_ != 0 && slot_ != InvalidSlot;
    }

private:
    template<std::size_t Capacity>
    friend class Detail::PairReservationTable;

    static constexpr std::size_t InvalidSlot =
        (std::numeric_limits<std::size_t>::max)();

    PairClaim(const void* table, std::size_t slot, std::uint64_t ticket) noexcept
        : table_(table), slot_(slot), ticket_(ticket) {}

    const void* table_{};
    std::size_t slot_{InvalidSlot};
    std::uint64_t ticket_{};
};

namespace Detail {
struct OwnerIdentity {
    std::uintptr_t admitted_module{};
    std::uint64_t load_generation{};
    const void* controller_identity{};
};

inline bool valid_write_request(const Request& request) noexcept {
    return request.address && request.size && request.size <= 32 &&
        request.address <= (std::numeric_limits<std::uintptr_t>::max)() - request.size &&
        request.expected && request.replacement;
}

template<std::size_t Capacity>
class PairReservationTable final {
public:
    PairReservationTable() noexcept = default;
    PairReservationTable(const PairReservationTable&) = delete;
    PairReservationTable& operator=(const PairReservationTable&) = delete;
    PairReservationTable(PairReservationTable&&) = delete;
    PairReservationTable& operator=(PairReservationTable&&) = delete;

    PairClaim Reserve(const OwnerIdentity& owner, const Pair& pair) noexcept {
        if (!valid_owner(owner) || !basic_pair_valid(pair)) return {};
        const auto ranges = ranges_for(pair);

        for (std::size_t i = 0; i < records_.size(); ++i) {
            const auto& record = records_[i];
            if (!record.used) continue;

            if (record.owner.admitted_module == owner.admitted_module) {
                if (same_owner(record.owner, owner) && same_ranges(record.ranges, ranges))
                    return PairClaim(this, i, record.ticket);
                // A retained module cannot rebind its ranges to another
                // controller, generation, or pair later in this process.
                return {};
            }
            if (ranges_overlap(record.ranges, ranges)) return {};
        }

        if (!next_ticket_) return {};
        std::size_t slot = records_.size();
        for (std::size_t i = 0; i < records_.size(); ++i) {
            if (!records_[i].used) {
                slot = i;
                break;
            }
        }
        if (slot == records_.size()) return {};

        auto& record = records_[slot];
        record.used = true;
        record.owner = owner;
        record.ranges = ranges;
        record.ticket = next_ticket_++;
        return PairClaim(this, slot, record.ticket);
    }

    bool Matches(const OwnerIdentity& owner, const PairClaim& claim,
                 const Pair& pair) const noexcept {
        if (!valid_owner(owner) || !basic_pair_valid(pair) ||
            !static_cast<bool>(claim) || claim.table_ != this ||
            claim.slot_ >= records_.size()) return false;
        const auto& record = records_[claim.slot_];
        return record.used && record.ticket == claim.ticket_ &&
            same_owner(record.owner, owner) &&
            same_ranges(record.ranges, ranges_for(pair));
    }

    bool OverlapsRange(std::uintptr_t address, std::size_t size) const noexcept {
        if (!valid_range(address, size)) return false;
        const Range candidate{address, address + size};
        for (const auto& record : records_) {
            if (!record.used) continue;
            for (const auto& range : record.ranges)
                if (overlaps(candidate, range)) return true;
        }
        return false;
    }

    bool OverlapsPair(const Pair& pair) const noexcept {
        if (!basic_pair_valid(pair)) return true;
        return OverlapsRange(pair[0].address, pair[0].size) ||
            OverlapsRange(pair[1].address, pair[1].size);
    }

    bool OwnsModule(std::uintptr_t admitted_module) const noexcept {
        if (!admitted_module) return false;
        for (const auto& record : records_)
            if (record.used && record.owner.admitted_module == admitted_module)
                return true;
        return false;
    }

    std::size_t Size() const noexcept {
        std::size_t count{};
        for (const auto& record : records_) count += record.used ? 1 : 0;
        return count;
    }

private:
    struct Range {
        std::uintptr_t begin{};
        std::uintptr_t end{};
    };

    struct Record {
        bool used{};
        OwnerIdentity owner{};
        std::array<Range, 2> ranges{};
        std::uint64_t ticket{};
    };

    static bool valid_owner(const OwnerIdentity& owner) noexcept {
        return owner.admitted_module && owner.load_generation &&
            owner.controller_identity;
    }

    static bool valid_range(std::uintptr_t address, std::size_t size) noexcept {
        return address && size &&
            address <= (std::numeric_limits<std::uintptr_t>::max)() - size;
    }

    static bool valid_request(const Request& request) noexcept { return valid_write_request(request); }

    static bool basic_pair_valid(const Pair& pair) noexcept {
        if (!valid_request(pair[0]) || !valid_request(pair[1])) return false;
        const Range first{pair[0].address, pair[0].address + pair[0].size};
        const Range second{pair[1].address, pair[1].address + pair[1].size};
        return !overlaps(first, second);
    }

    static bool overlaps(const Range& left, const Range& right) noexcept {
        return left.begin < right.end && right.begin < left.end;
    }

    static std::array<Range, 2> ranges_for(const Pair& pair) noexcept {
        return {{
            {pair[0].address, pair[0].address + pair[0].size},
            {pair[1].address, pair[1].address + pair[1].size},
        }};
    }

    static bool same_owner(const OwnerIdentity& left,
                           const OwnerIdentity& right) noexcept {
        return left.admitted_module == right.admitted_module &&
            left.load_generation == right.load_generation &&
            left.controller_identity == right.controller_identity;
    }

    static bool same_ranges(const std::array<Range, 2>& left,
                            const std::array<Range, 2>& right) noexcept {
        return left[0].begin == right[0].begin && left[0].end == right[0].end &&
            left[1].begin == right[1].begin && left[1].end == right[1].end;
    }

    static bool ranges_overlap(const std::array<Range, 2>& left,
                               const std::array<Range, 2>& right) noexcept {
        for (const auto& first : left)
            for (const auto& second : right)
                if (overlaps(first, second)) return true;
        return false;
    }

    std::array<Record, Capacity> records_{};
    std::uint64_t next_ticket_{1};
};

template<class Os>
PairFailure validate_pair(Os& os, const Pair& pair) noexcept {
    for (const auto& request : pair)
        if (!valid_write_request(request)) return PairFailure::InvalidRange;

    const auto first_end = pair[0].address + pair[0].size;
    const auto second_end = pair[1].address + pair[1].size;
    if (pair[0].address < second_end && pair[1].address < first_end)
        return PairFailure::Overlap;

    const auto page_size = os.PageSize();
    if (!page_size) return PairFailure::InvalidRange;
    for (const auto& request : pair) {
        if (request.address / page_size !=
                (request.address + request.size - 1) / page_size ||
            !os.ValidRange(request.address, request.size))
            return PairFailure::InvalidRange;
    }
    return PairFailure::None;
}
template<class Os>
PairResult RunPair(Os& os, const Pair& requests) noexcept {
    struct StagedRange {
        std::uintptr_t address{};
        std::size_t size{};
        std::array<std::uint8_t, 32> expected{};
        std::array<std::uint8_t, 32> replacement{};
        std::array<std::uint8_t, 32> original{};
        std::uint32_t protection{};
        bool protection_changed{};
        bool modified{};
    };

    const auto validation = validate_pair(os, requests);
    if (validation != PairFailure::None)
        return {PairOutcome::Rejected, validation};

    // All request data, thread metadata and rollback storage is fixed and
    // copied before any peer is suspended.
    std::array<StagedRange, 2> staged{};
    for (std::size_t range = 0; range < staged.size(); ++range) {
        staged[range].address = requests[range].address;
        staged[range].size = requests[range].size;
        const auto* expected = static_cast<const std::uint8_t*>(requests[range].expected);
        const auto* replacement = static_cast<const std::uint8_t*>(requests[range].replacement);
        for (std::size_t byte = 0; byte < staged[range].size; ++byte) {
            staged[range].expected[byte] = expected[byte];
            staged[range].replacement[byte] = replacement[byte];
        }
    }

    struct Peer {
        std::uint32_t id{};
        std::uintptr_t handle{};
        std::uint32_t previous{};
        bool suspended{};
    };
    std::array<Peer, MaxPeers> peers{};
    std::array<std::uint32_t, MaxPeers> ids{};
    std::size_t count{}, found{};

    auto finish = [&](PairResult result) noexcept {
        bool resumed = true;
        for (std::size_t i = count; i; --i) {
            if (peers[i - 1].suspended &&
                !os.Resume(peers[i - 1].handle, peers[i - 1].previous))
                resumed = false;
        }
        for (std::size_t i = 0; i < count; ++i) os.Close(peers[i].handle);
        return resumed ? result :
            PairResult{PairOutcome::Indeterminate, PairFailure::ResumeThread};
    };

    if (!os.Inventory(false, ids, found))
        return finish({PairOutcome::Rejected, PairFailure::Enumeration});

    for (unsigned pass = 0; pass < MaxInventories; ++pass) {
        for (std::size_t i = 0; i < found; ++i) {
            if (ids[i] == os.CurrentThread()) continue;
            std::size_t index{};
            for (; index < count && peers[index].id != ids[i]; ++index) {}
            if (index != count) continue;
            if (count == peers.size())
                return finish({PairOutcome::Rejected, PairFailure::Capacity});
            const auto handle = os.Open(ids[i]);
            if (!handle)
                return finish({PairOutcome::Rejected, PairFailure::OpenThread});
            peers[count++] = Peer{ids[i], handle};
        }

        for (std::size_t i = 0; i < count; ++i) {
            auto& peer = peers[i];
            if (peer.suspended) continue;
            if (!os.Suspend(peer.handle, peer.previous))
                return finish({PairOutcome::Rejected, PairFailure::SuspendThread});
            peer.suspended = true;

            std::uintptr_t pc{};
            if (!os.Pc(peer.handle, pc))
                return finish({PairOutcome::Rejected, PairFailure::ThreadContext});
            bool occupied{};
            for (const auto& range : staged)
                occupied |= pc >= range.address && pc < range.address + range.size;
            if (occupied)
                return finish({PairOutcome::Rejected, PairFailure::OccupiedPc});
        }

        if (!os.Inventory(true, ids, found))
            return finish({PairOutcome::Rejected, PairFailure::Enumeration});

        bool added{};
        for (std::size_t i = 0; i < found; ++i) {
            if (ids[i] == os.CurrentThread()) continue;
            std::size_t index{};
            for (; index < count && peers[index].id != ids[i]; ++index) {}
            added |= index == count;
        }
        for (std::size_t i = 0; i < count; ++i) {
            bool present{};
            for (std::size_t j = 0; j < found; ++j)
                present |= peers[i].id == ids[j];
            if (!present)
                return finish({PairOutcome::Rejected, PairFailure::UnstableInventory});
        }
        if (added) continue;

        bool expected_matches = true;
        for (auto& range : staged) {
            auto* target = reinterpret_cast<volatile std::uint8_t*>(range.address);
            for (std::size_t byte = 0; byte < range.size; ++byte) {
                range.original[byte] = target[byte];
                expected_matches &= range.original[byte] == range.expected[byte];
            }
        }
        if (!expected_matches)
            return finish({PairOutcome::Rejected, PairFailure::ExpectedBytes});

        auto rollback = [&](PairFailure cause) noexcept -> PairResult {
            bool writable = true;
            for (auto& range : staged) {
                if (!range.protection_changed) continue;
                std::uint32_t ignored{};
                if (!os.Writable(range.address, range.size, ignored)) writable = false;
            }
            if (!writable)
                return {PairOutcome::Indeterminate, PairFailure::RestoreProtection};

            for (auto& range : staged) {
                if (!range.modified) continue;
                auto* target = reinterpret_cast<volatile std::uint8_t*>(range.address);
                for (std::size_t byte = 0; byte < range.size; ++byte)
                    target[byte] = range.original[byte];
            }

            bool flushed = true;
            for (auto& range : staged) {
                if (range.modified && !os.Flush(range.address, range.size))
                    flushed = false;
            }

            bool restored = true;
            for (std::size_t i = staged.size(); i; --i) {
                auto& range = staged[i - 1];
                if (range.protection_changed &&
                    !os.Restore(range.address, range.size, range.protection))
                    restored = false;
            }
            if (!flushed || !restored)
                return {PairOutcome::Indeterminate,
                    flushed ? PairFailure::RestoreProtection : PairFailure::Flush};
            bool original_bytes_verified = true;
            for (const auto& range : staged) {
                if (!range.modified) continue;
                const auto* target = reinterpret_cast<volatile const std::uint8_t*>(range.address);
                for (std::size_t byte = 0; byte < range.size; ++byte)
                    original_bytes_verified &= target[byte] == range.original[byte];
            }
            if (!original_bytes_verified)
                return {PairOutcome::Indeterminate, PairFailure::Verification};
            return finish({PairOutcome::Rejected, cause});
        };

        for (auto& range : staged) {
            if (!os.Writable(range.address, range.size, range.protection)) {
                bool any_modified{};
                for (const auto& candidate : staged) any_modified |= candidate.modified;
                if (any_modified) return rollback(PairFailure::Protect);
                bool protection_restored = true;
                for (std::size_t i = staged.size(); i; --i) {
                    auto& prior = staged[i - 1];
                    if (prior.protection_changed &&
                        !os.Restore(prior.address, prior.size, prior.protection))
                        protection_restored = false;
                }
                if (!protection_restored)
                    return {PairOutcome::Indeterminate, PairFailure::RestoreProtection};
                return finish({PairOutcome::Rejected, PairFailure::Protect});
            }
            range.protection_changed = true;
            auto* target = reinterpret_cast<volatile std::uint8_t*>(range.address);
            for (std::size_t byte = 0; byte < range.size; ++byte)
                target[byte] = range.replacement[byte];
            range.modified = true;
        }

        bool flush_succeeded = true;
        for (auto& range : staged)
            if (!os.Flush(range.address, range.size)) flush_succeeded = false;
        if (!flush_succeeded) return rollback(PairFailure::Flush);

        bool verified = true;
        for (const auto& range : staged) {
            const auto* target = reinterpret_cast<volatile const std::uint8_t*>(range.address);
            for (std::size_t byte = 0; byte < range.size; ++byte)
                verified &= target[byte] == range.replacement[byte];
        }
        if (!verified) return rollback(PairFailure::Verification);

        bool protections_restored = true;
        for (std::size_t i = staged.size(); i; --i) {
            auto& range = staged[i - 1];
            if (range.protection_changed &&
                !os.Restore(range.address, range.size, range.protection))
                protections_restored = false;
        }
        if (!protections_restored) return rollback(PairFailure::RestoreProtection);

        bool replacements_verified = true;
        for (const auto& range : staged) {
            const auto* target = reinterpret_cast<volatile const std::uint8_t*>(range.address);
            for (std::size_t byte = 0; byte < range.size; ++byte)
                replacements_verified &= target[byte] == range.replacement[byte];
        }
        if (!replacements_verified) return rollback(PairFailure::Verification);

        return finish({PairOutcome::Published, PairFailure::None});
    }

    return finish({PairOutcome::Rejected, PairFailure::UnstableInventory});
}
}
}
