#include "flight_glider_gate.hpp"

#include "local_subject_context.hpp"
#include "owned_world_identity.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>

namespace shroudweave::flight {
namespace {

// Admitted actor dispatcher9A3400 supplies Q[0]=C and C[0]=current world.
// The separate object reached through C+8 has its own optional +0x30 field.
constexpr std::uintptr_t kWorldFromQuery = 0;
constexpr std::uintptr_t kRootRegistryFromRoot = 0x928;
constexpr std::uintptr_t kWorldFromRoot = 0xE179C0;
constexpr std::uintptr_t kAllocatorFromWorld = 0x30;
constexpr std::uintptr_t kEntityMapFromWorld = 0x148;
constexpr std::uintptr_t kTypeMapFromRegistry = 0x88;
constexpr std::uintptr_t kTypeKey = 0x4C234795u;
constexpr std::uint64_t kDynamicLocomotionFullKey = 0x717D8E414C234795ull;
constexpr char kDynamicLocomotionName[] = "keen::ecs::DynamicLocomotion";
constexpr std::uintptr_t kMaxMapCapacity = 1u << 16;
constexpr std::uint32_t kMaxMapProbes = 64;
constexpr std::uintptr_t kEntityRowsFromContext = 0x830;
constexpr std::uintptr_t kNativeRspFromRbp = 0x100;
constexpr std::uintptr_t kLocomotionState = 0x59;
constexpr std::uintptr_t kMaxQueryRows = 1'000'000;
constexpr std::uintptr_t kMaxEntityCapacity = 1u << 20;
constexpr std::uint32_t kMaxEntityProbes = 8;
constexpr std::uintptr_t kDynamicLocomotionSize = 0x1C8;
constexpr std::size_t kMaxCohortReads = 256;
constexpr std::size_t kMaxCohortBytes = 4096;
constexpr std::size_t kMaxCohortReadSize = 64;

constexpr std::uint32_t Fnv1a(const char* value, std::size_t length) noexcept {
    std::uint32_t hash = 0x811C9DC5u;
    for (std::size_t i = 0; i < length; ++i) {
        hash ^= static_cast<std::uint8_t>(value[i]);
        hash *= 0x01000193u;
    }
    return hash;
}
static_assert(Fnv1a(kDynamicLocomotionName, sizeof(kDynamicLocomotionName) - 1) == kTypeKey,
              "DynamicLocomotion native name hash changed");
static_assert(static_cast<std::uint32_t>(kDynamicLocomotionFullKey) == kTypeKey,
              "DynamicLocomotion full reflection key differs from the pinned native key hash");
constexpr std::uintptr_t kExpectedClientImageSize = 0x2A3F000;

std::atomic_bool g_selectorEnabled{};
std::atomic<std::uint64_t> g_selectorEpoch{1};
NativeProfileAdmission g_admittedProfile{};
std::atomic_bool g_admittedProfileReady{};

struct CountedReader final {
    BoundedNativeRead underlying{};
    void* context{};
    std::size_t calls{};
    std::size_t bytes{};
};

struct ReadObservation final {
    std::uintptr_t address{};
    std::uint8_t size{};
    std::array<std::uint8_t, kMaxCohortReadSize> value{};
};

struct CohortReader final {
    BoundedNativeRead underlying{};
    void* context{};
    std::size_t calls{};
    std::size_t bytes{};
    std::size_t observationCount{};
    bool inconsistent{};
    std::array<ReadObservation, kMaxCohortReads> observations{};
};

bool ReadCounted(std::uintptr_t address, void* destination, std::size_t size,
                 void* opaque) noexcept {
    auto* reader = static_cast<CountedReader*>(opaque);
    if (reader == nullptr || reader->underlying == nullptr || address == 0 ||
        destination == nullptr || size == 0 || size > 0x1000 ||
        reader->calls >= 256 || size > 4096 - reader->bytes ||
        address > (std::numeric_limits<std::uintptr_t>::max)() - size) return false;
    ++reader->calls;
    reader->bytes += size;
    return reader->underlying(address, destination, size, reader->context);
}

bool ReadCohort(std::uintptr_t address, void* destination, std::size_t size,
                void* opaque) noexcept {
    auto* reader = static_cast<CohortReader*>(opaque);
    if (reader == nullptr || reader->underlying == nullptr || address == 0 ||
        destination == nullptr || size == 0 || size > kMaxCohortReadSize ||
        reader->calls >= kMaxCohortReads || size > kMaxCohortBytes - reader->bytes ||
        reader->observationCount >= reader->observations.size() ||
        address > (std::numeric_limits<std::uintptr_t>::max)() - size) return false;
    ++reader->calls;
    reader->bytes += size;
    if (!reader->underlying(address, destination, size, reader->context)) return false;
    // Repeated reads are still performed and charged. Keep one final witness
    // per exact range; a changed repeat poisons the complete cohort immediately.
    for (std::size_t i=0;i<reader->observationCount;++i) {
        const auto& previous=reader->observations[i];
        if (previous.address==address && previous.size==size) {
            if (std::memcmp(previous.value.data(),destination,size)!=0) {
                reader->inconsistent=true;
                return false;
            }
            return true;
        }
    }
    auto& observation = reader->observations[reader->observationCount++];
    observation.address = address;
    observation.size = static_cast<std::uint8_t>(size);
    std::memcpy(observation.value.data(), destination, size);
    return true;
}

bool VerifyCohort(CohortReader& reader) noexcept {
    bool unchanged = !reader.inconsistent;
    for (std::size_t i = 0; i < reader.observationCount; ++i) {
        const auto& observation = reader.observations[i];
        if (reader.calls >= kMaxCohortReads ||
            observation.size > kMaxCohortBytes - reader.bytes) return false;
        std::array<std::uint8_t, kMaxCohortReadSize> current{};
        ++reader.calls;
        reader.bytes += observation.size;
        if (!reader.underlying(observation.address, current.data(), observation.size,
                               reader.context) ||
            std::memcmp(current.data(), observation.value.data(), observation.size) != 0) {
            unchanged = false;
        }
    }
    return unchanged;
}

struct LocalSnapshotContext final {
    NativeScalarFrame frame{};
    NativeQueryFrame admittedFrame{};
    NativeProfileAdmission profile{};
    const std::atomic_bool* enabled{};
    const std::atomic<std::uint64_t>* epoch{};
    std::uint64_t expectedEpoch{};
    CohortReader* reader{};
    std::uintptr_t expectedSite{};
    std::uintptr_t stackLow{};
    std::uintptr_t stackHigh{};
};

std::uint32_t EntityHash(std::uint32_t value) noexcept {
    value = ((value >> 16) ^ value) * 0x45D9F3Bu;
    value = ((value >> 16) ^ value) * 0x45D9F3Bu;
    return (value >> 16) ^ value;
}

bool LocalGuardedRead(void* opaque, std::uintptr_t address, void* output,
                      std::size_t bytes) noexcept {
    auto* context = static_cast<LocalSnapshotContext*>(opaque);
    return context != nullptr && context->reader != nullptr &&
           ReadCohort(address, output, bytes, context->reader);
}

bool LocalSnapshotCurrent(void* opaque,
                          shroudweave::client_context::CurrentEngineStamp& output) noexcept {
    auto* context = static_cast<LocalSnapshotContext*>(opaque);
    if (context == nullptr || context->enabled == nullptr || context->epoch == nullptr ||
        context->reader == nullptr || !context->enabled->load(std::memory_order_acquire) ||
        context->epoch->load(std::memory_order_acquire) != context->expectedEpoch) return false;
    NativeQueryFrame current{};
    if (!ValidateNativeQueryFrame(context->frame, context->expectedSite, context->stackLow,
                                  context->stackHigh, ReadCohort, context->reader, current) ||
        current.query != context->admittedFrame.query ||
        current.output != context->admittedFrame.output ||
        current.componentPointerSlot != context->admittedFrame.componentPointerSlot) return false;
    output.query = current.query;
    output.lifetimeGeneration = context->expectedEpoch;
    output.profile.profileId = kClientFlightProfileId;
    output.profile.generation = context->profile.generation;
    output.profile.imageBase = context->profile.imageBase;
    output.profile.imageSize = context->profile.imageSize;
    output.profile.admittedBeforePatch = context->profile.admittedBeforePatch;
    return true;
}

bool AddFits(std::uintptr_t base, std::uintptr_t amount) noexcept {
    return base <= (std::numeric_limits<std::uintptr_t>::max)() - amount;
}

bool MulFits(std::uintptr_t left, std::uintptr_t right, std::uintptr_t& result) noexcept {
    if (right != 0 && left > (std::numeric_limits<std::uintptr_t>::max)() / right) return false;
    result = left * right;
    return true;
}

template<class T>
bool ReadValue(BoundedNativeRead read, void* context, std::uintptr_t address, T& value) noexcept {
    return read != nullptr && address != 0 && AddFits(address, sizeof(T)) &&
           read(address, &value, sizeof(T), context);
}

bool ReadMapField(BoundedNativeRead read, void* context, std::uintptr_t map,
                  std::uintptr_t offset, std::uintptr_t& value) noexcept {
    return AddFits(map, offset) && ReadValue(read, context, map + offset, value);
}

enum class LookupResult : std::uint8_t { found, missing, invalid, exhausted };

LookupResult LookupNativeType(BoundedNativeRead read, void* context,
                              std::uintptr_t map, std::uint16_t& nativeIndex) noexcept {
    nativeIndex = 0;
    std::uintptr_t bitmap{};
    std::uintptr_t capacity{};
    std::uintptr_t keys{};
    std::uintptr_t values{};
    std::uint32_t occupied{};
    if (map == 0 || !ReadMapField(read, context, map, 0x08, bitmap) ||
        !ReadMapField(read, context, map, 0x10, capacity) ||
        !ReadMapField(read, context, map, 0x28, keys) ||
        !ReadMapField(read, context, map, 0x40, values) ||
        !AddFits(map, 0x54) || !ReadValue(read, context, map + 0x54, occupied) ||
        bitmap == 0 || keys == 0 || values == 0 || capacity == 0 ||
        (capacity & (capacity - 1)) != 0 || capacity > kMaxMapCapacity ||
        occupied == 0 || occupied > capacity) {
        return LookupResult::invalid;
    }

    const std::uintptr_t probeLimit = capacity < kMaxMapProbes ? capacity : kMaxMapProbes;
    std::uintptr_t slot = std::uintptr_t(kTypeKey) & (capacity - 1);
    for (std::uintptr_t probes = 0; probes < probeLimit; ++probes) {
        const auto wordIndex = slot >> 6;
        std::uintptr_t bitmapOffset{};
        if (!MulFits(wordIndex, sizeof(std::uint64_t), bitmapOffset) ||
            !AddFits(bitmap, bitmapOffset)) return LookupResult::invalid;
        std::uint64_t word{};
        if (!ReadValue(read, context, bitmap + bitmapOffset, word)) return LookupResult::invalid;
        if ((word & (std::uint64_t{1} << (slot & 63))) == 0) return LookupResult::missing;

        std::uintptr_t keyOffset{};
        if (!MulFits(slot, sizeof(std::uint32_t), keyOffset) || !AddFits(keys, keyOffset))
            return LookupResult::invalid;
        std::uint32_t key{};
        if (!ReadValue(read, context, keys + keyOffset, key)) return LookupResult::invalid;
        if (key == kTypeKey) {
            std::uintptr_t valueOffset{};
            if (!MulFits(slot, sizeof(std::uint16_t), valueOffset) ||
                !AddFits(values, valueOffset) ||
                !ReadValue(read, context, values + valueOffset, nativeIndex)) {
                return LookupResult::invalid;
            }
            return LookupResult::found;
        }
        slot = (slot + 1) & (capacity - 1);
    }
    return LookupResult::exhausted;
}

bool ResolveNativeIndex(BoundedNativeRead read, void* context,
                        std::uintptr_t registry, std::uint16_t& nativeIndex) noexcept {
    if (!AddFits(registry, kTypeMapFromRegistry)) return false;
    const auto map = registry + kTypeMapFromRegistry;
    return LookupNativeType(read, context, map, nativeIndex) == LookupResult::found;
}

bool ReadQueryIdentity(BoundedNativeRead read, void* context, const NativeQueryFrame& frame,
                       std::uintptr_t& queryContext, std::uint32_t& queryIndex) noexcept {
    std::array<std::uint8_t, 12> bytes{};
    if (frame.query == 0 || !AddFits(frame.query, sizeof(bytes)) ||
        !read(frame.query, bytes.data(), bytes.size(), context)) return false;
    queryContext = 0;
    queryIndex = 0;
    for (std::size_t i = 0; i < sizeof(queryContext); ++i)
        queryContext |= std::uintptr_t(bytes[i]) << (i * 8);
    for (std::size_t i = 0; i < sizeof(queryIndex); ++i)
        queryIndex |= std::uint32_t(bytes[sizeof(queryContext) + i]) << (i * 8);
    return queryContext != 0 && queryIndex != 0 && queryIndex <= kMaxQueryRows;
}

bool ResolveWorldRegistry(BoundedNativeRead read, void* context,
                          std::uintptr_t queryContext, std::uintptr_t& worldOut,
                          std::uintptr_t& registry, PitchDecisionPath* rejectedPath) noexcept {
    const auto reject = [rejectedPath](PitchDecisionPath path) noexcept {
        if (rejectedPath != nullptr) *rejectedPath = path;
        return false;
    };
    std::uintptr_t world{};
    if (!AddFits(queryContext, kWorldFromQuery) ||
        !ReadValue(read, context, queryContext + kWorldFromQuery, world))
        return reject(PitchDecisionPath::typed_world_query_read_failed);
    if (world == 0 || !AddFits(world, kAllocatorFromWorld) ||
        !AddFits(world, sizeof(std::uintptr_t)))
        return reject(PitchDecisionPath::typed_world_pointer_invalid);

    std::uintptr_t allocator{};
    std::uintptr_t rootRegistry{};
    if (!ReadValue(read, context, world, rootRegistry))
        return reject(PitchDecisionPath::typed_world_registry_read_failed);
    if (!ReadValue(read, context, world + kAllocatorFromWorld, allocator))
        return reject(PitchDecisionPath::typed_world_anchor_read_failed);
    if (allocator < 0x400)
        return reject(PitchDecisionPath::typed_world_root_underflow);
    const auto root = allocator - 0x400;
    std::uintptr_t expectedWorld{};
    if (!AddFits(root, kWorldFromRoot) || !AddFits(root, kRootRegistryFromRoot))
        return reject(PitchDecisionPath::typed_world_root_overflow);
    expectedWorld = root + kWorldFromRoot;
    registry = root + kRootRegistryFromRoot;
    worldOut = world;
    if (world != expectedWorld) {
        std::uint8_t nativeKind{};
        if (!ReadValue(read,context,world+8,nativeKind) ||
            shroudweave::client_context::world_identity::admit(
                root,world,allocator,nativeKind,
                [read,context](std::uintptr_t address,void* out,std::size_t size) noexcept {
                    return read(address,out,size,context);
                })==shroudweave::client_context::world_identity::Kind::unknown)
            return reject(PitchDecisionPath::typed_world_identity_mismatch);
    }
    if (rootRegistry != registry)
        return reject(PitchDecisionPath::typed_world_registry_mismatch);
    return true;
}

bool ResolveCurrentRow(BoundedNativeRead read, void* context, std::uintptr_t queryContext,
                       std::uint32_t queryIndex, std::uintptr_t& currentRow) noexcept {
    std::uintptr_t rows{};
    if (!AddFits(queryContext, kEntityRowsFromContext) ||
        !AddFits(queryContext, kEntityRowsFromContext + sizeof(std::uintptr_t)) ||
        !ReadValue(read, context, queryContext + kEntityRowsFromContext, rows) || rows == 0)
        return false;
    std::uint64_t rowCount{};
    if (!AddFits(queryContext, kEntityRowsFromContext + sizeof(std::uintptr_t)) ||
        !ReadValue(read, context, queryContext + kEntityRowsFromContext + sizeof(std::uintptr_t), rowCount) ||
        rowCount == 0 || rowCount > kMaxQueryRows || queryIndex > rowCount) return false;
    std::uintptr_t offset{};
    if (!MulFits(std::uintptr_t(queryIndex - 1), 16, offset) || !AddFits(rows, offset)) return false;
    currentRow = rows + offset;
    return true;
}

bool ResolveEntityLocation(BoundedNativeRead read, void* context, std::uintptr_t world,
                           std::uintptr_t currentRow, std::uint32_t entityId,
                           std::uint16_t selector, TypedGliderBinding& binding) noexcept {
    if (selector >= 0x280 || !AddFits(world, kEntityMapFromWorld)) return false;
    const auto map = world + kEntityMapFromWorld;
    std::uintptr_t bitmap{};
    std::uintptr_t capacity{};
    std::uintptr_t keys{};
    std::uintptr_t locations{};
    std::uint32_t occupied{};
    if (!AddFits(map, 0x54) ||
        !ReadValue(read, context, map + 0x08, bitmap) ||
        !ReadValue(read, context, map + 0x10, capacity) ||
        !ReadValue(read, context, map + 0x28, keys) ||
        !ReadValue(read, context, map + 0x40, locations) ||
        !ReadValue(read, context, map + 0x54, occupied) ||
        bitmap == 0 || keys == 0 || locations == 0 || capacity == 0 ||
        (capacity & (capacity - 1)) != 0 || capacity > kMaxEntityCapacity ||
        occupied == 0 || occupied > capacity) return false;

    std::uintptr_t slot = EntityHash(entityId) & (capacity - 1);
    bool found{};
    std::uintptr_t location{};
    for (std::uint32_t probe = 0; probe < kMaxEntityProbes; ++probe) {
        std::uintptr_t bitmapOffset{};
        std::uintptr_t keyOffset{};
        if (!MulFits(slot >> 6, sizeof(std::uint64_t), bitmapOffset) ||
            !AddFits(bitmap, bitmapOffset) || !MulFits(slot, sizeof(std::uint32_t), keyOffset) ||
            !AddFits(keys, keyOffset)) return false;
        std::uint64_t word{};
        if (!ReadValue(read, context, bitmap + bitmapOffset, word)) return false;
        if ((word & (std::uint64_t{1} << (slot & 63))) == 0) return false;
        std::uint32_t key{};
        if (!ReadValue(read, context, keys + keyOffset, key)) return false;
        if (key == entityId) {
            std::uintptr_t locationOffset{};
            if (!MulFits(slot, sizeof(std::uintptr_t), locationOffset) ||
                !AddFits(locations, locationOffset) ||
                !ReadValue(read, context, locations + locationOffset, location) || location == 0)
                return false;
            found = true;
            break;
        }
        slot = (slot + 1) & (capacity - 1);
    }
    if (!found || !AddFits(location, 0x38)) return false;

    std::array<std::uint8_t, 0x20> locationFields{};
    if (!read(location + 0x18, locationFields.data(), locationFields.size(), context)) return false;
    std::uintptr_t presenceBitmap{};
    std::uintptr_t entityChunk{};
    std::uint32_t entityRow{};
    std::memcpy(&presenceBitmap, locationFields.data(), sizeof(presenceBitmap));
    std::memcpy(&entityChunk, locationFields.data() + 8, sizeof(entityChunk));
    std::memcpy(&entityRow, locationFields.data() + 0x18, sizeof(entityRow));
    if (presenceBitmap == 0 || entityChunk == 0 || entityRow >= kMaxQueryRows ||
        !AddFits(entityChunk, 0x14)) return false;

    std::uint16_t entityRowStride{};
    if (!ReadValue(read, context, entityChunk + 0x14, entityRowStride) || entityRowStride == 0)
        return false;
    std::uintptr_t entityOffset{};
    std::uintptr_t expectedRow{};
    if (!MulFits(entityRow, 16, entityOffset) || !AddFits(entityChunk, entityRowStride) ||
        !AddFits(entityChunk + entityRowStride, entityOffset)) return false;
    expectedRow = entityChunk + entityRowStride + entityOffset;
    if (expectedRow != currentRow) return false;

    std::uintptr_t selectorOffset{};
    if (!MulFits(selector >> 6, sizeof(std::uint64_t), selectorOffset) ||
        !AddFits(presenceBitmap, selectorOffset)) return false;
    std::uint64_t selectorBits{};
    if (!ReadValue(read, context, presenceBitmap + selectorOffset, selectorBits) ||
        (selectorBits & (std::uint64_t{1} << (selector & 63))) == 0) return false;

    // Exact native entity lookup at 9A7331..9A7355: U holds the
    // component offset/stride, B the chunk, and location+30 the absolute row.
    // This types the callback pointer independently of materialized query columns.
    std::uint16_t componentOffset{};
    std::uint16_t componentStride{};
    const auto selectorBytes = std::uintptr_t(selector) * 2;
    if (!AddFits(presenceBitmap, 0xA84 + selectorBytes) ||
        !ReadValue(read, context, presenceBitmap + 0x84 + selectorBytes, componentOffset) ||
        !ReadValue(read, context, presenceBitmap + 0xA84 + selectorBytes, componentStride) ||
        componentStride < kDynamicLocomotionSize) return false;
    std::uintptr_t componentRowOffset{};
    if (!MulFits(entityRow, componentStride, componentRowOffset) ||
        !AddFits(entityChunk, componentOffset) ||
        !AddFits(entityChunk + componentOffset, componentRowOffset)) return false;
    binding.componentChunk = entityChunk + componentOffset;
    binding.component = binding.componentChunk + componentRowOffset;
    binding.componentOffset = componentOffset;
    binding.stride = componentStride;
    binding.nativeIndex = selector;

    binding.entityLocation = location;
    binding.entityPresenceBitmap = presenceBitmap;
    binding.entityChunk = entityChunk;
    binding.entityId = entityId;
    binding.entityRow = entityRow;
    binding.entityRowStride = entityRowStride;
    return true;
}

PitchDecision EvaluateGliderCandidate(
    const NativeScalarFrame& frame,
    const NativeProfileAdmission& profile,
    const std::atomic_bool& enabled,
    const std::atomic<std::uint64_t>& enableEpoch,
    std::uint64_t expectedEpoch,
    BoundedNativeRead read,
    void* readContext,
    std::uintptr_t expectedSite,
    std::uintptr_t stackLow,
    std::uintptr_t stackHigh,
    std::uint8_t initiallyObservedState,
    std::size_t prefixCalls,
    std::size_t prefixBytes) noexcept {
    PitchDecision decision{};
    decision.scalarBits = frame.originalScalarBits;
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    decision.locomotionState = initiallyObservedState;
    decision.hasLocomotionState = true;
    if (frame.nativeRbp != 0 && frame.nativeRsp != 0) {
        decision.frameRspRelationKnown = true;
        decision.frameRspRelationMatches = frame.nativeRbp >= kNativeRspFromRbp &&
            frame.nativeRsp == frame.nativeRbp - kNativeRspFromRbp;
    }
#else
    (void)initiallyObservedState;
#endif
    CohortReader cohort{read, readContext};
    auto finish = [&]() noexcept {
        decision.readCalls = prefixCalls + cohort.calls;
        decision.bytesRead = prefixBytes + cohort.bytes;
        return decision;
    };
    if (!enabled.load(std::memory_order_acquire) ||
        enableEpoch.load(std::memory_order_acquire) != expectedEpoch) {
        decision.path = PitchDecisionPath::local_subject_failed;
        return finish();
    }

    NativeQueryFrame queryFrame{};
    std::uint8_t candidateState{};
    if (!ValidateNativeQueryFrame(frame, expectedSite, stackLow, stackHigh,
                                  ReadCohort, &cohort, queryFrame) ||
        !ReadCheapLocomotionState(queryFrame, ReadCohort, &cohort, candidateState)) {
        decision.path = PitchDecisionPath::invalid_frame;
        return finish();
    }
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    decision.locomotionState = candidateState;
#endif
    if (candidateState != kHangGlidingState) {
        decision.path = PitchDecisionPath::non_gliding;
        return finish();
    }

    TypedGliderBinding firstBinding{};
    decision.path = PitchDecisionPath::typed_binding_failed;
    if (!ResolveCurrentTypedGlider(queryFrame, ReadCohort, &cohort, firstBinding, &decision.path) ||
        firstBinding.state != candidateState) {
        return finish();
    }

    shroudweave::client_context::NativeProfileStamp admittedProfile{};
    admittedProfile.profileId = kClientFlightProfileId;
    admittedProfile.generation = profile.generation;
    admittedProfile.imageBase = profile.imageBase;
    admittedProfile.imageSize = profile.imageSize;
    admittedProfile.admittedBeforePatch = profile.admittedBeforePatch;
    LocalSnapshotContext snapshotContext{
        frame, queryFrame, profile, &enabled, &enableEpoch, expectedEpoch, &cohort,
        expectedSite, stackLow, stackHigh};
    const shroudweave::client_context::CurrentQueryBorrow borrow{
        queryFrame.query, admittedProfile, &snapshotContext,
        LocalGuardedRead, LocalSnapshotCurrent};
    const auto localResult = shroudweave::client_context::evaluate_current_query(borrow);
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    decision.localSubjectReason = localResult.reason;
    decision.canonicalIndexMismatchMask = localResult.canonicalIndexMismatchMask;
    decision.diagnosticReadError = localResult.diagnosticReadError;
#endif
    if (localResult.predicate != shroudweave::client_context::Predicate::local) {
        decision.path = PitchDecisionPath::local_subject_failed;
        return finish();
    }

    // Pitch-only scope: preserve the native requested pitch. Neutral input
    // must not center XMM8 or require fields used only by deferred level flight.
    // The typed local-glider and complete query-cohort checks remain below.
    if (frame.hasPitchOperands) {
        const auto finiteBits = [](std::uint32_t bits) noexcept {
            return (bits & 0x7f800000u) != 0x7f800000u;
        };
        if (!finiteBits(frame.requestedPitchBits) || !finiteBits(frame.facingYBits)) {
            decision.path = PitchDecisionPath::local_subject_failed;
            return finish();
        }
    }
    NativeQueryFrame finalFrame{};
    TypedGliderBinding finalBinding{};
    if (!enabled.load(std::memory_order_acquire) ||
        enableEpoch.load(std::memory_order_acquire) != expectedEpoch ||
        !ValidateNativeQueryFrame(frame, expectedSite, stackLow, stackHigh,
                                  ReadCohort, &cohort, finalFrame) ||
        finalFrame.query != queryFrame.query || finalFrame.output != queryFrame.output ||
        !ResolveCurrentTypedGlider(finalFrame, ReadCohort, &cohort, finalBinding) ||
        !SameTypedGliderBinding(firstBinding, finalBinding) ||
        finalBinding.state != kHangGlidingState || !VerifyCohort(cohort) ||
        !enabled.load(std::memory_order_acquire) ||
        enableEpoch.load(std::memory_order_acquire) != expectedEpoch) {
        decision.path = PitchDecisionPath::local_subject_failed;
        return finish();
    }
    decision.scalarBits = kEnabledPitchScalarBits;
    decision.overrideRequestedPitch = false;
    decision.requestedPitchBits = frame.requestedPitchBits;
    decision.path = PitchDecisionPath::enabled_local_glider;
    return finish();
}

} // namespace

PitchDecision EvaluateCurrentPitch(
    const NativeScalarFrame& frame,
    const NativeProfileAdmission& profile,
    const std::atomic_bool& enabled,
    const std::atomic<std::uint64_t>& enableEpoch,
    BoundedNativeRead read,
    void* readContext,
    std::uintptr_t stackLow,
    std::uintptr_t stackHigh) noexcept {
    PitchDecision decision{};
    decision.scalarBits = frame.originalScalarBits;
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    if (frame.nativeRbp != 0 && frame.nativeRsp != 0) {
        decision.frameRspRelationKnown = true;
        decision.frameRspRelationMatches = frame.nativeRbp >= kNativeRspFromRbp &&
            frame.nativeRsp == frame.nativeRbp - kNativeRspFromRbp;
    }
#endif
    if (!enabled.load(std::memory_order_acquire)) {
        decision.path = PitchDecisionPath::disabled;
        return decision;
    }
    const auto expectedEpoch = enableEpoch.load(std::memory_order_acquire);
    if (read == nullptr || !profile.admittedBeforePatch || profile.imageBase == 0 ||
        profile.imageSize != kExpectedClientImageSize || profile.generation == 0 ||
        profile.imageBase > (std::numeric_limits<std::uintptr_t>::max)() - 0x26F6FE) {
        decision.path = PitchDecisionPath::invalid_profile;
        return decision;
    }

    // This small negative filter rejects every non-gliding state before the
    // typed registry and local-resource joins allocate their bounded cohort.
    CountedReader prefix{read, readContext};
    const auto expectedSite = static_cast<std::uintptr_t>(profile.imageBase + 0x26F6FE);
    NativeQueryFrame queryFrame{};
    if (!ValidateNativeQueryFrame(frame, expectedSite, stackLow, stackHigh,
                                  ReadCounted, &prefix, queryFrame)) {
        decision.path = PitchDecisionPath::invalid_frame;
        decision.readCalls = prefix.calls;
        decision.bytesRead = prefix.bytes;
        return decision;
    }
    std::uint8_t candidateState{};
    if (!ReadCheapLocomotionState(queryFrame, ReadCounted, &prefix, candidateState)) {
        decision.path = PitchDecisionPath::invalid_frame;
        decision.readCalls = prefix.calls;
        decision.bytesRead = prefix.bytes;
        return decision;
    }
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    decision.locomotionState = candidateState;
    decision.hasLocomotionState = true;
#endif
    if (candidateState != kHangGlidingState) {
        decision.path = PitchDecisionPath::non_gliding;
        decision.readCalls = prefix.calls;
        decision.bytesRead = prefix.bytes;
        return decision;
    }
    return EvaluateGliderCandidate(frame, profile, enabled, enableEpoch, expectedEpoch,
                                   read, readContext, expectedSite, stackLow, stackHigh,
                                   candidateState, prefix.calls, prefix.bytes);
}

bool AdmitCurrentClientProfile(std::uintptr_t imageBase, std::size_t imageSize,
                               std::uint64_t generation) noexcept {
    if (imageBase == 0 || imageSize != kExpectedClientImageSize || generation == 0) return false;
    if (g_admittedProfileReady.load(std::memory_order_acquire)) {
        return g_admittedProfile.imageBase == imageBase &&
               g_admittedProfile.imageSize == imageSize &&
               g_admittedProfile.generation == generation &&
               g_admittedProfile.admittedBeforePatch;
    }
    g_admittedProfile = {imageBase, imageSize, generation, true};
    g_admittedProfileReady.store(true, std::memory_order_release);
    return true;
}

void DisableCurrentPitchSelection() noexcept {
    g_selectorEnabled.store(false, std::memory_order_release);
    g_selectorEpoch.fetch_add(1, std::memory_order_acq_rel);
}

void EnableCurrentPitchSelection() noexcept {
    g_selectorEpoch.fetch_add(1, std::memory_order_acq_rel);
    g_selectorEnabled.store(true, std::memory_order_release);
}

bool CurrentPitchSelectionEnabled() noexcept {
    return g_selectorEnabled.load(std::memory_order_acquire);
}

const std::atomic_bool& CurrentPitchSelectionEnabledAtomic() noexcept {
    return g_selectorEnabled;
}

const std::atomic<std::uint64_t>& CurrentPitchSelectionEpochAtomic() noexcept {
    return g_selectorEpoch;
}

const NativeProfileAdmission& CurrentNativeProfile() noexcept {
    return g_admittedProfile;
}

bool ResolveCurrentTypedGlider(const NativeQueryFrame& frame, BoundedNativeRead read,
                               void* readContext, TypedGliderBinding& binding,
                               PitchDecisionPath* rejectedPath) noexcept {
    binding = {};
    const auto stage = [rejectedPath](PitchDecisionPath path) noexcept {
        if (rejectedPath != nullptr) *rejectedPath = path;
    };
    stage(PitchDecisionPath::typed_binding_failed);
    if (read == nullptr || frame.query == 0 || frame.output == 0 ||
        frame.componentPointerSlot != frame.output + 0xF0) return false;

    std::uintptr_t queryContext{};
    std::uint32_t queryIndex{};
    if (!ReadQueryIdentity(read, readContext, frame, queryContext, queryIndex)) return false;

    std::uintptr_t world{};
    std::uintptr_t registry{};
    stage(PitchDecisionPath::typed_world_failed);
    if (!ResolveWorldRegistry(read, readContext, queryContext, world, registry, rejectedPath)) return false;
    std::uint16_t nativeIndex{};
    stage(PitchDecisionPath::typed_index_failed);
    if (!ResolveNativeIndex(read, readContext, registry, nativeIndex)) return false;
    stage(PitchDecisionPath::typed_entity_failed);

    binding.query = frame.query;
    binding.queryContext = queryContext;
    binding.queryIndex = queryIndex;
    std::uintptr_t currentRow{};
    if (!ResolveCurrentRow(read, readContext, queryContext, queryIndex, currentRow)) return false;
    binding.currentRow = currentRow;
    std::uintptr_t callbackComponent{};
    if (!ReadValue(read, readContext, currentRow, binding.entityId) ||
        !ResolveEntityLocation(read, readContext, world, currentRow, binding.entityId,
                               nativeIndex, binding)) return false;
    stage(PitchDecisionPath::typed_component_failed);
    if (!ReadValue(read, readContext, frame.componentPointerSlot, callbackComponent) ||
        callbackComponent != binding.component ||
        !AddFits(binding.component, kLocomotionState) ||
        !ReadValue(read, readContext, binding.component + kLocomotionState, binding.state))
        return false;
    stage(PitchDecisionPath::typed_binding_failed);
    return binding.state == kHangGlidingState;
}

bool SameTypedGliderBinding(const TypedGliderBinding& left,
                            const TypedGliderBinding& right) noexcept {
    return left.query == right.query && left.queryContext == right.queryContext &&
           left.component == right.component && left.componentChunk == right.componentChunk &&
           left.currentRow == right.currentRow && left.entityLocation == right.entityLocation &&
           left.entityPresenceBitmap == right.entityPresenceBitmap &&
           left.entityChunk == right.entityChunk && left.queryIndex == right.queryIndex &&
           left.entityId == right.entityId && left.entityRow == right.entityRow &&
           left.nativeIndex == right.nativeIndex && left.stride == right.stride &&
           left.entityRowStride == right.entityRowStride &&
           left.componentOffset == right.componentOffset && left.state == right.state;
}

} // namespace shroudweave::flight
