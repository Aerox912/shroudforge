#include "engine-hooks/executable_write_set_transaction.h"

#include <array>
#include <cstdint>
#include <iostream>
#include <limits>
#include <span>
#include <utility>

namespace {
using namespace EngineHooks::ExecutableWrite;
using EngineHooks::ExecutableWrite::Detail::OwnerIdentity;
using EngineHooks::ExecutableWrite::Detail::RunWriteSet;
using SetTable = EngineHooks::ExecutableWrite::Detail::WriteSetReservationTable<8, 128>;
using PairTable = EngineHooks::ExecutableWrite::Detail::PairReservationTable<16>;

struct FakeRangeOs {
    std::size_t PageSize() const noexcept { return 0x1000; }
    bool ValidRange(std::uintptr_t, std::size_t) const noexcept { return true; }
};

unsigned checks{};
unsigned failures{};

void check(bool value, const char* label) {
    ++checks;
    if (!value) {
        ++failures;
        std::cerr << "FAIL " << label << '\n';
    }
}

OwnerIdentity owner(std::uintptr_t module, std::uint64_t generation,
                    const void* controller) {
    return {module, generation, controller};
}

Request request(std::uintptr_t address, const void* expected,
                const void* replacement, std::size_t size = 5) {
    return {address, size, expected, replacement};
}

void reservation_tests() {
    std::array<std::uint8_t, 5> expected{1, 2, 3, 4, 5};
    std::array<std::uint8_t, 5> replacement{6, 7, 8, 9, 10};
    std::array<Request, 70> requests{};
    for (std::size_t i = 0; i < requests.size(); ++i)
        requests[i] = request(0x100000 + i * 0x40, expected.data(), replacement.data());

    SetTable table;
    const auto first_owner = owner(11, 1, &requests);
    auto first = table.Reserve(first_owner, requests);
    check(static_cast<bool>(first) && table.Size() == 1,
          "fixed table reserves 70 distinct spans as one owner record");
    check(table.Matches(first_owner, first, requests),
          "claim matches its owner and exact ordered 70 ranges");

    auto idempotent = table.Reserve(first_owner, requests);
    check(static_cast<bool>(idempotent) && table.Size() == 1 &&
          table.Matches(first_owner, idempotent, requests),
          "exact owner and exact ordered range reservation is idempotent");

    auto reordered = requests;
    std::swap(reordered[3], reordered[4]);
    check(!table.Reserve(first_owner, reordered) &&
          !table.Matches(first_owner, first, reordered),
          "range reordering cannot rebind a module set");
    check(!table.Reserve(owner(11, 2, &requests), requests),
          "same module cannot rebind to another load generation");
    check(!table.Reserve(owner(11, 1, &reordered), requests),
          "same module cannot rebind to another controller identity");
    check(!table.Matches(owner(12, 1, &requests), first, requests),
          "claim rejects a different admitted module");
    check(!table.Matches(owner(11, 2, &requests), first, requests),
          "claim rejects a different owner generation");
    check(!table.Matches(first_owner, SetClaim{}, requests),
          "default stale claim cannot authorize publication");
    SetClaim moved = std::move(idempotent);
    check(static_cast<bool>(moved) && !static_cast<bool>(idempotent) &&
          !table.Matches(first_owner, idempotent, requests),
          "moved-from claim is stale and moved claim remains valid");

    auto changed_range = requests;
    changed_range[69].address += 0x10;
    check(!table.Reserve(first_owner, changed_range),
          "same module cannot extend or rebind one reserved range");

    std::array<Request, 1> disjoint_other_module{{
        request(0x200000, expected.data(), replacement.data()),
    }};
    auto disjoint_claim = table.Reserve(owner(15, 1, &disjoint_other_module),
                                        disjoint_other_module);
    check(static_cast<bool>(disjoint_claim) && table.Size() == 2,
          "another module may claim a nonoverlapping set");
    std::array<Request, 1> set_overlap{{
        request(requests[0].address + 2, expected.data(), replacement.data()),
    }};
    check(!table.Reserve(owner(16, 1, &set_overlap), set_overlap),
          "set registration rejects overlap with any other owner set");

    auto duplicate = requests;
    duplicate[10] = duplicate[9];
    check(EngineHooks::ExecutableWrite::Detail::validate_write_set_requests(duplicate) == PairFailure::Overlap &&
          !table.Reserve(owner(13, 1, &duplicate), duplicate),
          "duplicate spans are rejected before claiming");
    auto partial_overlap = requests;
    partial_overlap[10].address = partial_overlap[9].address + 4;
    check(EngineHooks::ExecutableWrite::Detail::validate_write_set_requests(partial_overlap) == PairFailure::Overlap,
          "partially overlapping spans are rejected");

    std::array<Request, MaxWriteSetSpans + 1> oversized{};
    for (std::size_t i = 0; i < oversized.size(); ++i)
        oversized[i] = request(0x400000 + i * 0x40, expected.data(), replacement.data());
    check(EngineHooks::ExecutableWrite::Detail::validate_write_set_requests(oversized) == PairFailure::ReservationCapacity &&
          !table.Reserve(owner(14, 1, &oversized), oversized),
          "span count above the fixed per-owner limit is rejected");

    auto too_wide = requests[0];
    too_wide.size = 33;
    check(EngineHooks::ExecutableWrite::Detail::validate_write_set_requests(
              std::span<const Request>(&too_wide, 1)) == PairFailure::InvalidRange,
          "each write span remains capped at 32 bytes");
    auto overflowing = request((std::numeric_limits<std::uintptr_t>::max)() - 2,
                               expected.data(), replacement.data(), 4);
    check(EngineHooks::ExecutableWrite::Detail::validate_write_set_requests(
              std::span<const Request>(&overflowing, 1)) == PairFailure::InvalidRange,
          "address-plus-size overflow is refused");
    FakeRangeOs range_os;
    auto crossing_page = request(0x1FFE, expected.data(), replacement.data(), 4);
    check(EngineHooks::ExecutableWrite::Detail::validate_write_set(range_os,
              std::span<const Request>(&crossing_page, 1)) == PairFailure::InvalidRange,
          "one span crossing a page boundary is refused");

    SetTable full;
    std::array<Request, 1> one{};
    for (std::uintptr_t i = 0; i < MaxWriteSetOwners; ++i) {
        one[0] = request(0x800000 + i * 0x40, expected.data(), replacement.data());
        auto claim = full.Reserve(owner(100 + i, 1, &one), one);
        check(static_cast<bool>(claim), "each fixed owner slot accepts a distinct module");
    }
    one[0] = request(0x900000, expected.data(), replacement.data());
    check(!full.Reserve(owner(200, 1, &one), one) && full.Size() == MaxWriteSetOwners,
          "ninth owner is refused at fixed table capacity");
}

void cross_reservation_tests() {
    std::array<std::uint8_t, 5> expected{1, 2, 3, 4, 5};
    std::array<std::uint8_t, 5> replacement{6, 7, 8, 9, 10};
    PairTable pairs;
    SetTable sets;

    const auto pair_owner = owner(31, 1, &pairs);
    const Pair pair{{
        request(0xA00000, expected.data(), replacement.data()),
        request(0xA00100, expected.data(), replacement.data()),
    }};
    auto pair_claim = pairs.Reserve(pair_owner, pair);
    check(static_cast<bool>(pair_claim), "pair table retains an existing pair registration");

    std::array<Request, 1> disjoint_same_module{{
        request(0xA10000, expected.data(), replacement.data()),
    }};
    const bool pair_blocks_same_module =
        EngineHooks::ExecutableWrite::Detail::ConflictsWithPairsForSet(
            pairs, pair_owner, disjoint_same_module);
    check(pair_blocks_same_module,
          "pair registration blocks a disjoint set for the same module");

    std::array<Request, 1> pair_overlap{{
        request(pair[0].address + 2, expected.data(), replacement.data()),
    }};
    const bool pair_blocks_range =
        EngineHooks::ExecutableWrite::Detail::ConflictsWithPairsForSet(
            pairs, owner(32, 1, &pair_overlap), pair_overlap);
    check(pair_blocks_range, "pair registration blocks an overlapping set for another module");

    const auto set_owner = owner(41, 1, &sets);
    std::array<Request, 1> set_span{{
        request(0xB00000, expected.data(), replacement.data()),
    }};
    auto set_claim = sets.Reserve(set_owner, set_span);
    check(static_cast<bool>(set_claim), "set table retains an existing set registration");

    Pair same_module_disjoint{{
        request(0xB10000, expected.data(), replacement.data()),
        request(0xB10100, expected.data(), replacement.data()),
    }};
    check(EngineHooks::ExecutableWrite::Detail::ConflictsWithSetsForPair(
              sets, set_owner, same_module_disjoint),
          "set registration blocks a disjoint pair for the same module");

    Pair overlapping_other_module{{
        request(set_span[0].address + 3, expected.data(), replacement.data()),
        request(0xB20000, expected.data(), replacement.data()),
    }};
    check(EngineHooks::ExecutableWrite::Detail::ConflictsWithSetsForPair(
              sets, owner(42, 1, &overlapping_other_module), overlapping_other_module),
          "set registration blocks an overlapping pair for another module");

    check(pairs.OverlapsRange(pair[0].address, pair[0].size) &&
          sets.OverlapsRange(set_span[0].address, set_span[0].size),
          "legacy single-range writer conflict queries see pair and set reservations");
}

void publication_tests() {
    std::array<std::uint8_t, 5> expected{1, 2, 3, 4, 5};
    std::array<std::uint8_t, 5> replacement{6, 7, 8, 9, 10};
    std::array<Request, 3> requests{{
        request(0xC00000, expected.data(), replacement.data()),
        request(0xC00100, expected.data(), replacement.data()),
        request(0xC00200, expected.data(), replacement.data()),
    }};

    for (std::size_t failed_index = 0; failed_index < requests.size(); ++failed_index) {
        std::array<bool, 3> published{};
        std::size_t calls{};
        const auto result = RunWriteSet(requests,
            []() noexcept { return PairFailure::None; },
            [&](const Request&) noexcept {
                const auto index = calls++;
                if (index == failed_index)
                    return Result{Outcome::Rejected, Failure::ExpectedBytes};
                published[index] = true;
                return Result{Outcome::Published, Failure::None};
            });
        const auto outcome = failed_index == 0
            ? WriteSetOutcome::Rejected : WriteSetOutcome::PartialPublished;
        check(result.outcome == outcome && result.published_count == failed_index &&
              result.failure == PairFailure::ExpectedBytes,
              "first, middle, and final failures report exact published prefix count");
        check(calls == failed_index + 1,
              "publication stops immediately after the first rejected range");
        bool exact_prefix = true;
        for (std::size_t i = 0; i < published.size(); ++i)
            exact_prefix &= published[i] == (i < failed_index);
        check(exact_prefix, "published ranges are exactly the reported successful prefix");
    }

    std::size_t calls{};
    const auto complete = RunWriteSet(requests,
        []() noexcept { return PairFailure::None; },
        [&](const Request&) noexcept {
            ++calls;
            return Result{Outcome::Published, Failure::None};
        });
    check(complete.outcome == WriteSetOutcome::Published &&
          complete.published_count == requests.size() && calls == requests.size(),
          "success is returned only after every requested span publishes");

    for (const auto prerequisite : {PairFailure::InvalidHeldEntryToken,
                                    PairFailure::OwnerNotRetained,
                                    PairFailure::InvalidClaim}) {
        calls = 0;
        const auto refused = RunWriteSet(requests,
            [prerequisite]() noexcept { return prerequisite; },
            [&](const Request&) noexcept {
                ++calls;
                return Result{Outcome::Published, Failure::None};
            });
        check(refused.outcome == WriteSetOutcome::Rejected &&
              refused.published_count == 0 && refused.failure == prerequisite && calls == 0,
              "invalid held token, owner, or claim cannot reach a write callback");
    }

    calls = 0;
    const auto owner_lost_after_prefix = RunWriteSet(requests,
        [&calls]() noexcept {
            return calls < 2 ? PairFailure::None : PairFailure::OwnerNotRetained;
        },
        [&calls](const Request&) noexcept {
            ++calls;
            return Result{Outcome::Published, Failure::None};
        });
    check(owner_lost_after_prefix.outcome == WriteSetOutcome::PartialPublished &&
          owner_lost_after_prefix.published_count == 2 &&
          owner_lost_after_prefix.failure == PairFailure::OwnerNotRetained,
          "per-range owner recheck preserves and reports a completed prefix");

    calls = 0;
    const auto indeterminate = RunWriteSet(requests,
        []() noexcept { return PairFailure::None; },
        [&calls](const Request&) noexcept {
            if (calls++ == 1) return Result{Outcome::Indeterminate, Failure::Flush};
            return Result{Outcome::Published, Failure::None};
        });
    check(indeterminate.outcome == WriteSetOutcome::Indeterminate &&
          indeterminate.published_count == 1 &&
          indeterminate.failure == PairFailure::Flush && calls == 2,
          "indeterminate core failure propagates with prior prefix count and exact cause");
}
}

int main() {
    reservation_tests();
    cross_reservation_tests();
    publication_tests();
    std::cout << checks << " checks, " << failures << " failures\n";
    return failures ? 1 : 0;
}
