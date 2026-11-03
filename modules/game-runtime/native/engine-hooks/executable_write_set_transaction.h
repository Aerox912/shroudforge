#pragma once

#include "engine-hooks/executable_write_pair_transaction.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>
#include <utility>

namespace EngineHooks::ExecutableWrite {

inline constexpr std::size_t MaxWriteSetOwners = 8;
inline constexpr std::size_t MaxWriteSetSpans = 128;

enum class WriteSetOutcome : unsigned {
    Rejected,
    Published,
    PartialPublished,
    Indeterminate,
};

struct WriteSetResult {
    // Rejected publishes no span. Published means every span passed its own
    // verified write. PartialPublished keeps the exact successful prefix; the
    // caller must keep attribution closed and retain all possibly reachable
    // code and owners unless the outcome is Published.
    WriteSetOutcome outcome{WriteSetOutcome::Rejected};
    std::size_t published_count{};
    PairFailure failure{PairFailure::None};
};

namespace Detail {
template<std::size_t OwnerCapacity, std::size_t MaxSpans>
class WriteSetReservationTable;
}

class SetClaim final {
public:
    SetClaim() noexcept = default;
    SetClaim(const SetClaim&) = delete;
    SetClaim& operator=(const SetClaim&) = delete;

    SetClaim(SetClaim&& other) noexcept
        : table_(std::exchange(other.table_, nullptr)),
          slot_(std::exchange(other.slot_, InvalidSlot)),
          ticket_(std::exchange(other.ticket_, 0)) {}

    SetClaim& operator=(SetClaim&& other) noexcept {
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
    template<std::size_t OwnerCapacity, std::size_t MaxSpans>
    friend class Detail::WriteSetReservationTable;

    static constexpr std::size_t InvalidSlot =
        (std::numeric_limits<std::size_t>::max)();

    SetClaim(const void* table, std::size_t slot, std::uint64_t ticket) noexcept
        : table_(table), slot_(slot), ticket_(ticket) {}

    const void* table_{};
    std::size_t slot_{InvalidSlot};
    std::uint64_t ticket_{};
};

namespace Detail {
inline PairFailure validate_write_set_requests(std::span<const Request> requests) noexcept {
    if (requests.empty()) return PairFailure::InvalidRange;
    if (requests.size() > MaxWriteSetSpans) return PairFailure::ReservationCapacity;

    for (std::size_t i = 0; i < requests.size(); ++i) {
        if (!valid_write_request(requests[i])) return PairFailure::InvalidRange;
        const auto begin = requests[i].address;
        const auto end = begin + requests[i].size;
        for (std::size_t j = 0; j < i; ++j) {
            const auto other_begin = requests[j].address;
            const auto other_end = other_begin + requests[j].size;
            if (begin < other_end && other_begin < end) return PairFailure::Overlap;
        }
    }
    return PairFailure::None;
}

template<class Os>
PairFailure validate_write_set(Os& os, std::span<const Request> requests) noexcept {
    const auto request_failure = validate_write_set_requests(requests);
    if (request_failure != PairFailure::None) return request_failure;

    const auto page_size = os.PageSize();
    if (!page_size) return PairFailure::InvalidRange;
    for (const auto& request : requests) {
        if (request.address / page_size !=
                (request.address + request.size - 1) / page_size ||
            !os.ValidRange(request.address, request.size))
            return PairFailure::InvalidRange;
    }
    return PairFailure::None;
}

inline PairFailure to_pair_failure(Failure failure) noexcept {
    switch (failure) {
    case Failure::None: return PairFailure::None;
    case Failure::InvalidRange: return PairFailure::InvalidRange;
    case Failure::Enumeration: return PairFailure::Enumeration;
    case Failure::Capacity: return PairFailure::Capacity;
    case Failure::OpenThread: return PairFailure::OpenThread;
    case Failure::SuspendThread: return PairFailure::SuspendThread;
    case Failure::ThreadContext: return PairFailure::ThreadContext;
    case Failure::OccupiedPc: return PairFailure::OccupiedPc;
    case Failure::UnstableInventory: return PairFailure::UnstableInventory;
    case Failure::ExpectedBytes: return PairFailure::ExpectedBytes;
    case Failure::Protect: return PairFailure::Protect;
    case Failure::Flush: return PairFailure::Flush;
    case Failure::RestoreProtection: return PairFailure::RestoreProtection;
    case Failure::ResumeThread: return PairFailure::ResumeThread;
    }
    return PairFailure::InvalidRange;
}

// The testable bounded publication loop does not hide a prefix write behind an
// all-or-nothing result. The production adapter supplies token checks and the
// existing single-range writer as the two callbacks.
template<class BeforeRange, class WriteRange>
WriteSetResult RunWriteSet(std::span<const Request> requests,
                           BeforeRange&& before_range,
                           WriteRange&& write_range) {
    const auto validation = validate_write_set_requests(requests);
    if (validation != PairFailure::None)
        return {WriteSetOutcome::Rejected, 0, validation};

    std::size_t published{};
    for (const auto& request : requests) {
        const auto prerequisite = before_range();
        if (prerequisite != PairFailure::None) {
            return {
                published ? WriteSetOutcome::PartialPublished : WriteSetOutcome::Rejected,
                published,
                prerequisite,
            };
        }

        const auto result = write_range(request);
        if (result.outcome == Outcome::Indeterminate)
            return {WriteSetOutcome::Indeterminate, published,
                    to_pair_failure(result.failure)};
        if (result.outcome != Outcome::Published) {
            return {
                published ? WriteSetOutcome::PartialPublished : WriteSetOutcome::Rejected,
                published,
                to_pair_failure(result.failure),
            };
        }
        ++published;
    }
    return {WriteSetOutcome::Published, published, PairFailure::None};
}
}

namespace Detail {
template<std::size_t OwnerCapacity, std::size_t MaxSpans>
class WriteSetReservationTable final {
public:
    WriteSetReservationTable() noexcept = default;
    WriteSetReservationTable(const WriteSetReservationTable&) = delete;
    WriteSetReservationTable& operator=(const WriteSetReservationTable&) = delete;
    WriteSetReservationTable(WriteSetReservationTable&&) = delete;
    WriteSetReservationTable& operator=(WriteSetReservationTable&&) = delete;

    SetClaim Reserve(const OwnerIdentity& owner,
                     std::span<const Request> requests) noexcept {
        if (!valid_owner(owner) || requests.size() > MaxSpans ||
            validate_write_set_requests(requests) != PairFailure::None)
            return {};

        const auto ranges = ranges_for(requests);
        for (std::size_t i = 0; i < records_.size(); ++i) {
            const auto& record = records_[i];
            if (!record.used) continue;

            if (record.owner.admitted_module == owner.admitted_module) {
                if (same_owner(record.owner, owner) &&
                    same_ranges(record.ranges, record.count, ranges, requests.size()))
                    return SetClaim(this, i, record.ticket);
                return {};
            }
            if (ranges_overlap(record.ranges, record.count, ranges, requests.size()))
                return {};
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
        record.count = requests.size();
        record.ticket = next_ticket_++;
        return SetClaim(this, slot, record.ticket);
    }

    bool Matches(const OwnerIdentity& owner, const SetClaim& claim,
                 std::span<const Request> requests) const noexcept {
        if (!valid_owner(owner) ||
            requests.size() > MaxSpans ||
            validate_write_set_requests(requests) != PairFailure::None ||
            !static_cast<bool>(claim) || claim.table_ != this ||
            claim.slot_ >= records_.size())
            return false;
        const auto& record = records_[claim.slot_];
        const auto ranges = ranges_for(requests);
        return record.used && record.ticket == claim.ticket_ &&
            same_owner(record.owner, owner) &&
            same_ranges(record.ranges, record.count, ranges, requests.size());
    }

    bool OwnsModule(std::uintptr_t admitted_module) const noexcept {
        if (!admitted_module) return false;
        for (const auto& record : records_)
            if (record.used && record.owner.admitted_module == admitted_module)
                return true;
        return false;
    }

    bool OverlapsRange(std::uintptr_t address, std::size_t size) const noexcept {
        if (!valid_range(address, size)) return false;
        const Range candidate{address, address + size};
        for (const auto& record : records_) {
            if (!record.used) continue;
            for (std::size_t i = 0; i < record.count; ++i)
                if (overlaps(candidate, record.ranges[i])) return true;
        }
        return false;
    }

    bool OverlapsPair(const Pair& pair) const noexcept {
        if (!valid_write_request(pair[0]) || !valid_write_request(pair[1])) return true;
        return OverlapsRange(pair[0].address, pair[0].size) ||
            OverlapsRange(pair[1].address, pair[1].size);
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
        std::array<Range, MaxSpans> ranges{};
        std::size_t count{};
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

    static std::array<Range, MaxSpans> ranges_for(
        std::span<const Request> requests) noexcept {
        std::array<Range, MaxSpans> ranges{};
        for (std::size_t i = 0; i < requests.size(); ++i)
            ranges[i] = {requests[i].address, requests[i].address + requests[i].size};
        return ranges;
    }

    static bool overlaps(const Range& left, const Range& right) noexcept {
        return left.begin < right.end && right.begin < left.end;
    }

    static bool same_owner(const OwnerIdentity& left,
                           const OwnerIdentity& right) noexcept {
        return left.admitted_module == right.admitted_module &&
            left.load_generation == right.load_generation &&
            left.controller_identity == right.controller_identity;
    }

    template<std::size_t RangeCapacity>
    static bool same_ranges(const std::array<Range, RangeCapacity>& left,
                            std::size_t left_count,
                            const std::array<Range, RangeCapacity>& right,
                            std::size_t right_count) noexcept {
        if (left_count != right_count) return false;
        for (std::size_t i = 0; i < left_count; ++i)
            if (left[i].begin != right[i].begin || left[i].end != right[i].end)
                return false;
        return true;
    }

    template<std::size_t RangeCapacity>
    static bool ranges_overlap(const std::array<Range, RangeCapacity>& left,
                               std::size_t left_count,
                               const std::array<Range, RangeCapacity>& right,
                               std::size_t right_count) noexcept {
        for (std::size_t i = 0; i < left_count; ++i)
            for (std::size_t j = 0; j < right_count; ++j)
                if (overlaps(left[i], right[j])) return true;
        return false;
    }

    std::array<Record, OwnerCapacity> records_{};
    std::uint64_t next_ticket_{1};
};

template<std::size_t PairCapacity>
bool ConflictsWithPairsForSet(
    const PairReservationTable<PairCapacity>& pairs,
    const OwnerIdentity& owner,
    std::span<const Request> requests) noexcept {
    if (pairs.OwnsModule(owner.admitted_module)) return true;
    for (const auto& request : requests)
        if (pairs.OverlapsRange(request.address, request.size)) return true;
    return false;
}

template<std::size_t OwnerCapacity, std::size_t MaxSpans>
bool ConflictsWithSetsForPair(
    const WriteSetReservationTable<OwnerCapacity, MaxSpans>& sets,
    const OwnerIdentity& owner,
    const Pair& pair) noexcept {
    return sets.OwnsModule(owner.admitted_module) || sets.OverlapsPair(pair);
}
}
}
