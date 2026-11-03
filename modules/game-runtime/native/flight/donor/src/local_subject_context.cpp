#include "local_subject_context.hpp"
#include "owned_world_identity.hpp"

#include <array>
#include <cstring>
#include <limits>

namespace shroudweave::client_context {
namespace {

constexpr std::uint64_t kMaxProfileImageSize = 0x40000000ull;
constexpr std::uint64_t kMaxQueryRows = 1'000'000ull;
constexpr std::uint64_t kMaxEntityCapacity = 1ull << 20;
constexpr std::uint64_t kMaxLocalRecords = 4096;
constexpr std::uint64_t kMaxLocalHashCapacity = 4096;
constexpr std::uint32_t kLocalNameHash = 0x9117771Du;
constexpr char kLocalName[] = "LocalPlayerData";
constexpr std::size_t kLocalNameLength = sizeof(kLocalName) - 1;

bool checked_add(Address base, std::uint64_t offset, Address& out) noexcept {
    if (offset > static_cast<std::uint64_t>(std::numeric_limits<Address>::max() - base)) return false;
    out = base + static_cast<Address>(offset);
    return true;
}

bool checked_read_end(Address at, std::size_t size) noexcept {
    return size != 0 && size <= std::numeric_limits<Address>::max() - at;
}

bool is_power_of_two(std::uint64_t value) noexcept {
    return value != 0 && (value & (value - 1)) == 0;
}

bool valid_profile(const NativeProfileStamp& profile) noexcept {
    return profile.profileId != 0 && profile.generation != 0 && profile.imageBase != 0 &&
           profile.imageSize != 0 && profile.imageSize <= kMaxProfileImageSize &&
           profile.imageSize <= std::numeric_limits<Address>::max() - profile.imageBase &&
           profile.admittedBeforePatch;
}

bool same_profile(const NativeProfileStamp& a, const NativeProfileStamp& b) noexcept {
    return a.profileId == b.profileId && a.generation == b.generation &&
           a.imageBase == b.imageBase && a.imageSize == b.imageSize &&
           a.admittedBeforePatch == b.admittedBeforePatch;
}

bool same_snapshot(const CurrentEngineStamp& a, const CurrentEngineStamp& b) noexcept {
    return a.query == b.query && a.lifetimeGeneration == b.lifetimeGeneration &&
           same_profile(a.profile, b.profile);
}

class ReadJournal {
public:
    struct Observation {
        Address address{};
        std::uint8_t size{};
        std::array<std::uint8_t, kMaxRangeBytes> value{};
    };

    explicit ReadJournal(const CurrentQueryBorrow& borrow) noexcept : borrow_(borrow) {}

    bool read(Address at, void* output, std::size_t size) noexcept {
        if (!borrow_.guardedRead || !borrow_.context || !output || !checked_read_end(at, size) ||
            size > kMaxRangeBytes) {
            fail(Reason::guarded_read_failed);
            return false;
        }
        if (readCalls_ >= kMaxReadCallbacks || size > kMaxTotalReadBytes - bytesRead_) {
            fail(Reason::read_budget_exceeded);
            return false;
        }
        if (observationCount_ >= observations_.size() ||
            size > kMaxObservedBytes - observedBytes_) {
            fail(Reason::observation_budget_exceeded);
            return false;
        }

        ++readCalls_;
        bytesRead_ += size;
        auto& observation = observations_[observationCount_];
        observation.address = at;
        observation.size = static_cast<std::uint8_t>(size);
        if (!borrow_.guardedRead(borrow_.context, at, observation.value.data(), size)) {
            fail(Reason::guarded_read_failed);
            return false;
        }
        std::memcpy(output, observation.value.data(), size);
        ++observationCount_;
        observedBytes_ += size;
        return true;
    }

    template<class T>
    bool read_value(Address at, T& output) noexcept {
        static_assert(sizeof(T) <= kMaxRangeBytes);
        return read(at, &output, sizeof(T));
    }

    bool verify_all() noexcept {
        bool unchanged = true;
        bool readFailed = false;
        for (std::size_t i = 0; i < observationCount_; ++i) {
            if (readCalls_ >= kMaxReadCallbacks ||
                observations_[i].size > kMaxTotalReadBytes - bytesRead_) {
                fail(Reason::read_budget_exceeded);
                unchanged = false;
                break;
            }
            std::array<std::uint8_t, kMaxRangeBytes> current{};
            const auto& observation = observations_[i];
            ++readCalls_;
            bytesRead_ += observation.size;
            if (!borrow_.guardedRead(borrow_.context, observation.address,
                                     current.data(), observation.size)) {
                readFailed = true;
                unchanged = false;
                continue;
            }
            if (std::memcmp(current.data(), observation.value.data(), observation.size) != 0) {
                unchanged = false;
            }
        }
        if (readFailed) fail(Reason::guarded_read_failed);
        else if (!unchanged && error_ == Reason::none) fail(Reason::memory_changed);
        return unchanged;
    }

    bool take_final_snapshot(const CurrentEngineStamp& initial) noexcept {
        if (!borrow_.snapshotCurrent) {
            fail(Reason::snapshot_unavailable);
            return false;
        }
        CurrentEngineStamp current{};
        ++snapshotCalls_;
        if (!borrow_.snapshotCurrent(borrow_.context, current)) {
            fail(Reason::snapshot_unavailable);
            return false;
        }
        if (!same_snapshot(initial, current) || current.query != borrow_.query ||
            !same_profile(current.profile, borrow_.admittedProfile) ||
            current.lifetimeGeneration == 0) {
            fail(Reason::profile_or_lifetime_changed);
            return false;
        }
        return true;
    }

    void fail(Reason reason) noexcept {
        if (error_ == Reason::none) error_ = reason;
    }

    Reason error() const noexcept { return error_; }
    std::uint32_t read_calls() const noexcept { return static_cast<std::uint32_t>(readCalls_); }
    std::uint32_t bytes_read() const noexcept { return static_cast<std::uint32_t>(bytesRead_); }
    std::uint16_t observations() const noexcept { return static_cast<std::uint16_t>(observationCount_); }
    std::uint8_t snapshot_calls() const noexcept { return static_cast<std::uint8_t>(snapshotCalls_); }

private:
    const CurrentQueryBorrow& borrow_;
    std::array<Observation, kMaxObservedRanges> observations_{};
    std::size_t observationCount_{};
    std::size_t observedBytes_{};
    std::size_t readCalls_{};
    std::size_t bytesRead_{};
    std::uint8_t snapshotCalls_{1}; // The successful admission snapshot is taken before this journal.
    Reason error_{Reason::none};
};

struct HashProbe {
    std::uint64_t slot{};
    std::uint8_t probes{};
    bool found{};
    bool exhausted{};
};

std::uint32_t entity_hash(std::uint32_t entity) noexcept {
    auto value = ((entity >> 16) ^ entity) * 0x45D9F3Bu;
    value = ((value >> 16) ^ value) * 0x45D9F3Bu;
    return (value >> 16) ^ value;
}

constexpr std::uint32_t fnv1a_name_hash(const char* text, std::size_t size) noexcept {
    std::uint32_t hash = 0x811C9DC5u;
    for (std::size_t i = 0; i < size; ++i) {
        hash ^= static_cast<std::uint8_t>(text[i]);
        hash *= 0x01000193u;
    }
    return hash;
}
static_assert(fnv1a_name_hash(kLocalName, kLocalNameLength) == kLocalNameHash,
              "canonical LocalPlayerData FNV pin must match native evidence");

bool read_wrapped_values(ReadJournal& journal, Address base, std::uint64_t capacity,
                         std::uint64_t start, std::uint64_t count,
                         std::size_t stride, std::uint8_t* output) noexcept {
    if (!base || !capacity || !count || count > kMaxHashProbes || start >= capacity ||
        stride == 0 || count > capacity) return false;
    if (count > std::numeric_limits<std::uint64_t>::max() / stride ||
        start > std::numeric_limits<std::uint64_t>::max() / stride) return false;
    const auto firstCount = (count < capacity - start) ? count : capacity - start;
    const auto secondCount = count - firstCount;
    const auto firstBytes = static_cast<std::size_t>(firstCount * stride);
    Address firstAddress{};
    if (!checked_add(base, start * stride, firstAddress) ||
        !journal.read(firstAddress, output, firstBytes)) return false;
    if (secondCount) {
        Address secondAddress{};
        const auto secondBytes = static_cast<std::size_t>(secondCount * stride);
        if (!checked_add(base, 0, secondAddress) ||
            !journal.read(secondAddress, output + firstBytes, secondBytes)) return false;
    }
    return true;
}

struct BitmapWord {
    std::uint64_t index{};
    std::uint64_t value{};
};

bool load_probe_bits(ReadJournal& journal, Address bitmap, std::uint64_t capacity,
                     std::uint64_t start, std::uint64_t count,
                     std::array<BitmapWord, 3>& words, std::uint8_t& wordCount) noexcept {
    wordCount = 0;
    if (!bitmap || !capacity || !count || count > kMaxHashProbes) return false;
    for (std::uint64_t i = 0; i < count; ++i) {
        const auto slot = (start + i) & (capacity - 1);
        const auto wordIndex = slot >> 6;
        bool alreadyRead = false;
        for (std::uint8_t j = 0; j < wordCount; ++j) {
            if (words[j].index == wordIndex) {
                alreadyRead = true;
                break;
            }
        }
        if (alreadyRead) continue;
        if (wordCount >= words.size()) return false;
        Address at{};
        if (wordIndex > std::numeric_limits<std::uint64_t>::max() / sizeof(std::uint64_t) ||
            !checked_add(bitmap, wordIndex * sizeof(std::uint64_t), at) ||
            !journal.read_value(at, words[wordCount].value)) return false;
        words[wordCount].index = wordIndex;
        ++wordCount;
    }
    return true;
}

bool probe_bit(const std::array<BitmapWord, 3>& words, std::uint8_t count,
              std::uint64_t slot) noexcept {
    const auto wordIndex = slot >> 6;
    for (std::uint8_t i = 0; i < count; ++i) {
        if (words[i].index == wordIndex)
            return (words[i].value & (std::uint64_t{1} << (slot & 63))) != 0;
    }
    return false;
}

bool find_hash_slot(ReadJournal& journal, Address bitmap, Address keys,
                    std::uint64_t capacity, std::uint64_t maxCapacity,
                    std::uint32_t wanted, std::uint32_t hashInput,
                    std::uint8_t& probeCounter, HashProbe& result) noexcept {
    if (!bitmap || !keys || !is_power_of_two(capacity) || capacity > maxCapacity) return false;
    const auto start = std::uint64_t(hashInput) & (capacity - 1);
    const auto count = capacity < kMaxHashProbes ? capacity : kMaxHashProbes;
    std::array<std::uint8_t, kMaxHashProbes * sizeof(std::uint32_t)> keyBytes{};
    std::array<BitmapWord, 3> words{};
    std::uint8_t wordCount{};
    if (!read_wrapped_values(journal, keys, capacity, start, count,
                             sizeof(std::uint32_t), keyBytes.data()) ||
        !load_probe_bits(journal, bitmap, capacity, start, count, words, wordCount)) return false;

    for (std::uint64_t i = 0; i < count; ++i) {
        const auto slot = (start + i) & (capacity - 1);
        ++probeCounter;
        if (!probe_bit(words, wordCount, slot)) {
            result = {slot, probeCounter, false, false};
            return true;
        }
        std::uint32_t key{};
        std::memcpy(&key, keyBytes.data() + i * sizeof(key), sizeof(key));
        if (key == wanted) {
            result = {slot, probeCounter, true, false};
            return true;
        }
    }
    result = {0, probeCounter, false, true};
    return true;
}

bool finish_result(ReadJournal& journal, const CurrentEngineStamp& initial, Result& result) noexcept {
    const bool memoryStable = journal.verify_all();
    const bool snapshotStable = journal.take_final_snapshot(initial);
    if (!memoryStable || !snapshotStable) {
        result.predicate = Predicate::unknown;
        result.reason = journal.error() == Reason::none ? Reason::memory_changed : journal.error();
    }
    result.readCalls = journal.read_calls();
    result.bytesRead = journal.bytes_read();
    result.observations = journal.observations();
    result.snapshotCalls = journal.snapshot_calls();
    return memoryStable && snapshotStable;
}

} // namespace

Result evaluate_current_query(const CurrentQueryBorrow& borrow) noexcept {
    Result result{};
    if (!borrow.query || !borrow.context || !borrow.guardedRead || !borrow.snapshotCurrent) {
        result.reason = Reason::invalid_borrow;
        return result;
    }

    CurrentEngineStamp initial{};
    ++result.snapshotCalls;
    if (!borrow.snapshotCurrent(borrow.context, initial)) {
        result.reason = Reason::snapshot_unavailable;
        return result;
    }
    if (!valid_profile(borrow.admittedProfile) || !valid_profile(initial.profile) ||
        !same_profile(initial.profile, borrow.admittedProfile) || initial.query != borrow.query ||
        initial.lifetimeGeneration == 0) {
        result.reason = Reason::profile_not_admitted;
        return result;
    }

    ReadJournal journal(borrow);
    auto unknown = [&](Reason reason) noexcept {
        result.predicate = Predicate::unknown;
        result.reason = journal.error() == Reason::none ? reason : journal.error();
        result.readCalls = journal.read_calls();
        result.bytesRead = journal.bytes_read();
        result.observations = journal.observations();
        result.snapshotCalls = journal.snapshot_calls();
        return result;
    };

    std::array<std::uint8_t, 12> queryBytes{};
    if (!journal.read(borrow.query, queryBytes.data(), queryBytes.size())) return unknown(Reason::guarded_read_failed);
    Address context{};
    std::uint32_t index{};
    std::memcpy(&context, queryBytes.data(), sizeof(context));
    std::memcpy(&index, queryBytes.data() + 8, sizeof(index));
    if (!context || !index) return unknown(Reason::malformed_query);

    Address rowsAddress{};
    std::uint32_t rowCount{};
    Address queryRowsField{};
    if (!checked_add(context, 0x830, queryRowsField)) return unknown(Reason::malformed_query);
    std::array<std::uint8_t, 12> rowsBytes{};
    if (!journal.read(queryRowsField, rowsBytes.data(), rowsBytes.size())) return unknown(Reason::guarded_read_failed);
    std::memcpy(&rowsAddress, rowsBytes.data(), sizeof(rowsAddress));
    std::memcpy(&rowCount, rowsBytes.data() + 8, sizeof(rowCount));
    if (!rowsAddress || !rowCount || rowCount > kMaxQueryRows) return unknown(Reason::malformed_query);
    if (index > rowCount) return unknown(Reason::query_index_out_of_range);
    const auto rowOrdinal = std::uint64_t(index - 1);
    if (rowOrdinal > std::numeric_limits<std::uint64_t>::max() / 16) return unknown(Reason::malformed_query);
    Address currentRow{};
    if (!checked_add(rowsAddress, rowOrdinal * 16, currentRow)) return unknown(Reason::malformed_query);
    if (!journal.read_value(currentRow, result.entity)) return unknown(Reason::guarded_read_failed);

    Address worldField{};
    // The admitted actor-query dispatcher stores its owning world at C[0].
    // World/root/registry and exact current-row checks below still establish
    // main-world membership; the leading pointer alone grants no authority.
    if (!checked_add(context, 0, worldField)) return unknown(Reason::malformed_query);
    Address world{};
    if (!journal.read_value(worldField, world) || !world) return unknown(Reason::guarded_read_failed);
    std::array<std::uint8_t, 0x38> worldBytes{};
    if (!journal.read(world, worldBytes.data(), worldBytes.size())) return unknown(Reason::guarded_read_failed);
    Address rootRegistry{};
    Address allocator{};
    std::memcpy(&rootRegistry, worldBytes.data(), sizeof(rootRegistry));
    std::memcpy(&allocator, worldBytes.data() + 0x30, sizeof(allocator));
    if (allocator < 0x400) return unknown(Reason::invalid_world_root);
    const Address root = allocator - 0x400;
    if (!root) return unknown(Reason::invalid_world_root);
    Address expectedWorld{};
    Address expectedRegistry{};
    if (!checked_add(root, 0xE179C0, expectedWorld) ||
        !checked_add(root, 0x928, expectedRegistry)) return unknown(Reason::invalid_world_root);
    const auto worldKind=world_identity::admit(root,world,allocator,worldBytes[8],
        [&journal](Address address,void* out,std::size_t bytes) noexcept {
            return journal.read(address,out,bytes);
        });
    if (worldKind==world_identity::Kind::unknown) return unknown(Reason::invalid_world_root);
    if (rootRegistry != expectedRegistry) return unknown(Reason::world_registry_mismatch);

    Address entityMap{};
    if (!checked_add(world, 0x148, entityMap)) return unknown(Reason::invalid_world_root);
    std::array<std::uint8_t, 0x28> entityMapFirst{};
    Address entityBitmap{};
    std::uint64_t entityCapacity{};
    Address entityKeys{};
    if (!checked_add(entityMap, 8, expectedWorld) ||
        !journal.read(expectedWorld, entityMapFirst.data(), entityMapFirst.size())) {
        return unknown(Reason::invalid_entity_map);
    }
    std::memcpy(&entityBitmap, entityMapFirst.data(), sizeof(entityBitmap));
    std::memcpy(&entityCapacity, entityMapFirst.data() + 8, sizeof(entityCapacity));
    std::memcpy(&entityKeys, entityMapFirst.data() + 0x20, sizeof(entityKeys));
    Address locations{};
    std::uint32_t entityLive{};
    Address entityLocationsField{};
    std::array<std::uint8_t, 0x18> entityMapTail{};
    if (!checked_add(entityMap, 0x40, entityLocationsField) ||
        !journal.read(entityLocationsField, entityMapTail.data(), entityMapTail.size()) ||
        !entityBitmap || !entityKeys || !is_power_of_two(entityCapacity) ||
        entityCapacity > kMaxEntityCapacity) {
        return unknown(Reason::invalid_entity_map);
    }
    std::memcpy(&locations, entityMapTail.data(), sizeof(locations));
    std::memcpy(&entityLive, entityMapTail.data() + 0x14, sizeof(entityLive));
    if (!locations || !entityLive || entityLive > entityCapacity) {
        return unknown(Reason::invalid_entity_map);
    }
    if (worldKind==world_identity::Kind::owned_secondary) {
        // Root ownership alone cannot qualify a player across separate maps.
        // Require the exact canonical backing index, including occupancy,
        // capacity, keys, locations and live count. All fields are confirmed.
        Address canonicalMap{}, canonicalFirst{}, canonicalKeys{}, canonicalTail{};
        std::array<std::uint8_t,16> sharedFirst{};
        std::array<std::uint8_t,24> sharedTail{};
        Address canonicalBitmap{}, canonicalKeyPointer{}, canonicalLocations{};
        std::uint64_t canonicalCapacity{};
        std::uint32_t canonicalLive{};
        if (!checked_add(root,0xE179C0+0x148,canonicalMap) ||
            !checked_add(canonicalMap,8,canonicalFirst) ||
            !checked_add(canonicalMap,0x28,canonicalKeys) ||
            !checked_add(canonicalMap,0x40,canonicalTail) ||
            !journal.read(canonicalFirst,sharedFirst.data(),sharedFirst.size()) ||
            !journal.read_value(canonicalKeys,canonicalKeyPointer) ||
            !journal.read(canonicalTail,sharedTail.data(),sharedTail.size())) {
            auto refusal=unknown(Reason::canonical_index_witness_failed);
            refusal.diagnosticReadError=journal.error();
            refusal.reason=Reason::canonical_index_witness_failed;
            return refusal;
        }
        std::memcpy(&canonicalBitmap,sharedFirst.data(),8);
        std::memcpy(&canonicalCapacity,sharedFirst.data()+8,8);
        std::memcpy(&canonicalLocations,sharedTail.data(),8);
        std::memcpy(&canonicalLive,sharedTail.data()+0x14,4);
        const auto mismatch=static_cast<std::uint8_t>(
            (canonicalBitmap!=entityBitmap ? 1u : 0u) |
            (canonicalCapacity!=entityCapacity ? 2u : 0u) |
            (canonicalKeyPointer!=entityKeys ? 4u : 0u) |
            (canonicalLocations!=locations ? 8u : 0u) |
            (canonicalLive!=entityLive ? 16u : 0u));
        if (mismatch!=0) {
            auto refusal=unknown(Reason::canonical_index_mismatch);
            // Early refusal observations are not final-confirmed alias proof.
            refusal.canonicalIndexMismatchMask=mismatch;
            return refusal;
        }
    }


    HashProbe entityLookup{};
    if (!find_hash_slot(journal, entityBitmap, entityKeys, entityCapacity, kMaxEntityCapacity,
                        result.entity, entity_hash(result.entity), result.entityProbes, entityLookup)) {
        return unknown(journal.error() == Reason::none ? Reason::invalid_entity_map : journal.error());
    }
    // The native helper hashes the full entity DWORD, then compares that same
    // full DWORD in the selected key slot.
    if (!entityLookup.found || entityLookup.exhausted) {
        result.entityProbes = entityLookup.probes;
        return unknown(entityLookup.exhausted ? Reason::entity_probe_exhausted : Reason::entity_not_indexed);
    }
    const auto entitySlot = entityLookup.slot;
    Address locationEntry{};
    if (entitySlot >= entityCapacity || entitySlot > std::numeric_limits<std::uint64_t>::max() / 8 ||
        !checked_add(locations, entitySlot * 8, locationEntry)) return unknown(Reason::invalid_location);
    Address location{};
    if (!journal.read_value(locationEntry, location) || !location) return unknown(Reason::invalid_location);
    std::array<std::uint8_t, 0x20> locationFields{};
    Address locationFieldsAddress{};
    if (!checked_add(location, 0x18, locationFieldsAddress) ||
        !journal.read(locationFieldsAddress, locationFields.data(), locationFields.size()))
        return unknown(Reason::invalid_location);
    Address locationBitmap{};
    Address chunk{};
    std::uint32_t row{};
    std::memcpy(&locationBitmap, locationFields.data(), sizeof(locationBitmap));
    std::memcpy(&chunk, locationFields.data() + 8, sizeof(chunk));
    std::memcpy(&row, locationFields.data() + 0x18, sizeof(row));
    if (!locationBitmap || !chunk || row >= kMaxQueryRows) return unknown(Reason::invalid_location);
    Address chunkRowOffsetField{};
    if (!checked_add(chunk, 0x14, chunkRowOffsetField)) return unknown(Reason::invalid_location);
    std::uint16_t chunkRowOffset{};
    if (!journal.read_value(chunkRowOffsetField, chunkRowOffset)) return unknown(Reason::invalid_location);
    // Native 9A72A0 indexes U by the component selector supplied in RSI.
    // This evaluator has no admitted selector, so U is retained as an observed
    // non-null dependency but is never interpreted as a row-occupancy bitmap.
    Address expectedRow{};
    if (row > std::numeric_limits<std::uint64_t>::max() / 16 ||
        !checked_add(chunk, chunkRowOffset, expectedRow) ||
        !checked_add(expectedRow, std::uint64_t(row) * 16, expectedRow) ||
        expectedRow != currentRow) return unknown(Reason::current_row_mismatch);

    Address localMap{};
    if (!checked_add(root, 0x7462D0, localMap)) return unknown(Reason::invalid_world_root);
    std::array<std::uint8_t, 0x18> localMapHeader{};
    std::array<std::uint8_t, 0x10> localMapOccupancy{};
    std::array<std::uint8_t, 0x30> localMapTail{};
    Address records{};
    std::uint64_t recordCount{};
    std::uint64_t recordCapacity{};
    Address localBitmap{};
    std::uint64_t localCapacity{};
    Address localKeys{};
    Address localIndices{};
    std::uint32_t localOccupied{};
    Address localMapTailAddress{};
    Address localMapOccupancyAddress{};
    if (!checked_add(localMap, 0x50, localMapTailAddress) ||
        !checked_add(localMap,0x30,localMapOccupancyAddress) ||
        !journal.read(localMap, localMapHeader.data(), localMapHeader.size()) ||
        !journal.read(localMapOccupancyAddress,localMapOccupancy.data(),localMapOccupancy.size()) ||
        !journal.read(localMapTailAddress, localMapTail.data(), localMapTail.size())) {
        return unknown(Reason::invalid_local_map);
    }
    std::memcpy(&records, localMapHeader.data(), sizeof(records));
    std::memcpy(&recordCount, localMapHeader.data() + 8, sizeof(recordCount));
    std::memcpy(&recordCapacity, localMapHeader.data() + 0x10, sizeof(recordCapacity));
    std::memcpy(&localBitmap, localMapOccupancy.data(), sizeof(localBitmap));
    std::memcpy(&localCapacity, localMapOccupancy.data() + 8, sizeof(localCapacity));
    std::memcpy(&localKeys, localMapTail.data(), sizeof(localKeys));
    std::memcpy(&localIndices, localMapTail.data() + 0x18, sizeof(localIndices));
    std::memcpy(&localOccupied, localMapTail.data() + 0x2C, sizeof(localOccupied));
    if (!records || !localBitmap ||
        !localKeys || !localIndices || !recordCount || recordCount > kMaxLocalRecords ||
        recordCapacity < recordCount || recordCapacity > kMaxLocalRecords ||
        !is_power_of_two(localCapacity) || localCapacity > kMaxLocalHashCapacity ||
        !localOccupied || localOccupied != recordCount || localOccupied > localCapacity) {
        return unknown(Reason::invalid_local_map);
    }

    constexpr auto localNameHash = fnv1a_name_hash(kLocalName, kLocalNameLength);
    HashProbe localLookup{};
    if (!find_hash_slot(journal, localBitmap, localKeys, localCapacity, kMaxLocalHashCapacity,
                        localNameHash, localNameHash, result.localProbes, localLookup)) {
        return unknown(journal.error() == Reason::none ? Reason::invalid_local_map : journal.error());
    }
    result.localProbes = localLookup.probes;
    if (!localLookup.found || localLookup.exhausted) {
        return unknown(localLookup.exhausted ? Reason::local_probe_exhausted : Reason::canonical_resource_missing);
    }

    // The native registration map uses 16-bit record indices.
    Address recordIndexAddress{};
    if (localLookup.slot > std::numeric_limits<std::uint64_t>::max() / sizeof(std::uint16_t) ||
        !checked_add(localIndices, localLookup.slot * sizeof(std::uint16_t), recordIndexAddress)) {
        return unknown(Reason::invalid_local_map);
    }
    std::uint16_t recordIndex{};
    if (!journal.read_value(recordIndexAddress, recordIndex) || recordIndex >= recordCount) {
        return unknown(Reason::invalid_local_map);
    }
    if (recordIndex > std::numeric_limits<std::uint64_t>::max() / 0xE8) return unknown(Reason::invalid_local_map);
    Address record{};
    if (!checked_add(records, std::uint64_t(recordIndex) * 0xE8, record)) return unknown(Reason::invalid_local_map);

    std::array<std::uint8_t, 0x18> recordHead{};
    std::array<std::uint8_t, 0x10> recordIdentity{};
    Address name{};
    std::uint64_t nameLength{};
    std::uint64_t nameCapacity{};
    std::uint32_t recordHash{};
    Address registration{};
    Address selected{};
    Address recordIdentityAddress{};
    Address recordHashAddress{};
    if (!journal.read(record, recordHead.data(), recordHead.size()) ||
        !checked_add(record,0xa0,recordHashAddress) ||
        !journal.read_value(recordHashAddress,recordHash) ||
        !checked_add(record, 0xA8, recordIdentityAddress) ||
        !journal.read(recordIdentityAddress, recordIdentity.data(), recordIdentity.size())) {
        return unknown(Reason::invalid_canonical_resource);
    }
    std::memcpy(&name, recordHead.data(), sizeof(name));
    std::memcpy(&nameLength, recordHead.data() + 8, sizeof(nameLength));
    std::memcpy(&nameCapacity, recordHead.data() + 0x10, sizeof(nameCapacity));
    std::memcpy(&registration, recordIdentity.data(), sizeof(registration));
    std::memcpy(&selected, recordIdentity.data() + 8, sizeof(selected));
    if (!name || nameLength != kLocalNameLength || nameCapacity < nameLength ||
        nameCapacity > kMaxLocalRecords || recordHash != localNameHash ||
        !registration || !selected || registration != selected) {
        return unknown(Reason::invalid_canonical_resource);
    }
    std::array<char, kLocalNameLength> observedName{};
    if (!journal.read(name, observedName.data(), observedName.size()) ||
        std::memcmp(observedName.data(), kLocalName, kLocalNameLength) != 0) {
        return unknown(Reason::invalid_canonical_resource);
    }
    Address selectedValueAddress{};
    if (!checked_add(selected, 4, selectedValueAddress) ||
        !journal.read_value(selectedValueAddress, result.selectedEntity) || !result.selectedEntity) {
        return unknown(Reason::invalid_selected_value);
    }

    result.predicate = result.selectedEntity == result.entity ? Predicate::local : Predicate::remote;
    result.reason = result.predicate == Predicate::local ? Reason::local_value_matches : Reason::local_value_differs;
    if (!finish_result(journal, initial, result)) return result;
    return result;
}

} // namespace shroudweave::client_context
