#ifndef SF_SEMANTIC_ATTRIBUTION_TESTING
#error "The owned ledger test target must define SF_SEMANTIC_ATTRIBUTION_TESTING for every translation unit."
#endif

#include "owned_ledger.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <memory>
#include <span>
#include <type_traits>
#include <variant>
#include <vector>

namespace SemanticAttribution::testing {

class FixtureIssuer final {
public:
    static NativeReceipt At(NativeSite site,
                            NativeThreadScalar thread,
                            NativeFrameScalar frame,
                            std::uint64_t adapter,
                            std::uint64_t profile) noexcept {
        return NativeReceipt(site, thread, frame, adapter, profile);
    }
};

} // namespace SemanticAttribution::testing

namespace {

using namespace SemanticAttribution;

constexpr NativeThreadScalar kThread = 0x51;
constexpr std::uint64_t kAdapter = 0xA11CE;
constexpr std::uint64_t kProfile = 0x260B02;
constexpr ScalarAddress kRow = 0x100000;
constexpr ScalarAddress kContext = 0x200000;
constexpr ScalarAddress kWorld = 0x300000;
constexpr ScalarAddress kOwner = kWorld + 0x928;
constexpr ScalarAddress kTypeDescriptor = 0x500000;
constexpr ScalarAddress kCallback = 0x600000;
constexpr ScalarAddress kSchema = 0x700000;
constexpr ScalarAddress kQuerySlot = 0x800000;
constexpr ScalarAddress kQuerySlotTwo = kQuerySlot + 8;
constexpr ScalarAddress kParent = 0x900000;
constexpr ScalarAddress kChild = 0xA00000;
constexpr ScalarAddress kRootSlot = 0xB00000;
constexpr ScalarAddress kDispatch = 0xC00000;
constexpr ScalarAddress kManager = kWorld + 0xE15DC0;
constexpr ScalarAddress kDispatchRecordAddress = 0x34000000;
constexpr ScalarAddress kDispatchRecordAddressTwo = kDispatchRecordAddress + 0x38;
constexpr SemanticHash kHash = 0xA1234567U;
constexpr SemanticKey kSemanticKey = 0xFEED1234ULL;
constexpr std::uint32_t kNativeOrdinal = 7;
constexpr std::uint32_t kNativeGroup = 0;
constexpr std::uint32_t kNativeFlags = 0x00000120;
constexpr std::uint32_t kOutputIndex = 3;
constexpr ScalarAddress kEmittedComponentPointer = 0xD00000;
constexpr std::uint32_t kOutputStride = 0x40;
constexpr ScalarAddress kNativeOutputRecordAddress = 0xD00180;
constexpr ScalarAddress kEmittedComponentPointerTwo = 0;
constexpr ScalarAddress kNativeOutputRecordAddressTwo = 0xD00280;
constexpr std::uint32_t kOutputIndexTwo = 5;
constexpr ScalarAddress kGateOffset = 0x148;

constexpr std::array<DescriptorDefinition, 1> kCatalog{{
    DescriptorDefinition{kHash, kSemanticKey},
}};

std::array<DispatchRecordId, kMaxDispatchRecords> DispatchRecordList(
    DispatchRecordId first = {}, DispatchRecordId second = {}) {
    std::array<DispatchRecordId, kMaxDispatchRecords> records{};
    records[0] = first;
    records[1] = second;
    return records;
}

const char* g_current_test = "not started";

[[noreturn]] void Fail(const char* expression, int line) {
    std::fprintf(stderr, "owned ledger test failure in %s at line %d: %s\n",
                 g_current_test, line, expression);
    std::abort();
}

#define REQUIRE(expression) \
    do { \
        if (!(expression)) { \
            Fail(#expression, __LINE__); \
        } \
    } while (false)

template <class Id>
Id Issued(const EventReply& reply, EventStatus expected) {
    if (reply.status != expected) {
        std::fprintf(stderr, "issued status mismatch in %s: got %u expected %u\n",
                     g_current_test, static_cast<unsigned>(reply.status),
                     static_cast<unsigned>(expected));
    }
    REQUIRE(reply.status == expected);
    const auto* value = std::get_if<Id>(&reply.issued);
    REQUIRE(value != nullptr);
    return *value;
}

template <class Id>
Id Related(const EventReply& reply, EventStatus expected) {
    if (reply.status != expected) {
        std::fprintf(stderr, "related status mismatch in %s: got %u expected %u\n",
                     g_current_test, static_cast<unsigned>(reply.status),
                     static_cast<unsigned>(expected));
    }
    REQUIRE(reply.status == expected);
    const auto* value = std::get_if<Id>(&reply.related);
    REQUIRE(value != nullptr);
    return *value;
}

void Status(const EventReply& reply, EventStatus expected) {
    if (reply.status != expected) {
        std::fprintf(stderr, "status mismatch in %s: got %u expected %u\n",
                     g_current_test, static_cast<unsigned>(reply.status),
                     static_cast<unsigned>(expected));
    }
    REQUIRE(reply.status == expected);
}

Limits DefaultLimits() {
    return Limits{
        .max_records = 96,
        .max_evidence_entries = 48,
        .max_observations = 8,
        .max_registration_entries = 8,
        .max_compile_entries = 8,
        .expected_adapter_incarnation = kAdapter,
        .expected_profile_identity = kProfile,
    };
}

class Fixture final {
public:
    explicit Fixture(Limits limits = DefaultLimits()) : limits_(limits) {
        const auto required = Ledger::RequiredStorage(limits_);
        REQUIRE(required != 0);
        storage_.resize(required + alignof(std::max_align_t));
        const auto result = Ledger::Construct(
            std::span<std::byte>(storage_.data(), storage_.size()),
            limits_,
            DescriptorCatalog{std::span<const DescriptorDefinition>(kCatalog)});
        REQUIRE(result.status == ConstructStatus::Ready);
        ledger_ = result.ledger;
        REQUIRE(ledger_ != nullptr);
    }

    Fixture(const Fixture&) = delete;
    Fixture& operator=(const Fixture&) = delete;

    ~Fixture() {
        if (ledger_ != nullptr) {
            ledger_->CloseAll(CloseReason::Shutdown);
            std::destroy_at(ledger_);
        }
    }

    template <class E>
    EventReply Send(NativeSite site,
                    NativeFrameScalar frame,
                    const E& event,
                    NativeThreadScalar thread = kThread,
                    std::uint64_t adapter = kAdapter,
                    std::uint64_t profile = kProfile) {
        const auto receipt = testing::FixtureIssuer::At(site, thread, frame, adapter, profile);
        return ledger_->Observe(receipt, Event{event});
    }

    bool Copy(ObservationId id, OwnedObservation& output) const {
        return ledger_->TryCopy(id, output);
    }

    void CloseAll(CloseReason reason = CloseReason::Explicit) {
        ledger_->CloseAll(reason);
    }

    Ledger* Get() noexcept { return ledger_; }

private:
    Limits limits_;
    std::vector<std::byte> storage_;
    Ledger* ledger_{};
};

struct Trace final {
    Fixture fixture;
    ScalarAddress manager{kManager};
    RowPassId pass;
    RowIterationId iteration;
    ContextSetupId context_setup;
    ContextBirthId context_birth;
    ContextPublicationId context_publication;
    WorldBirthId world_birth;
    WorldPublicationId world_publication;
    RegistrationOpId registration;
    SystemBindingId binding;
    RegistrationOpId registration_two;
    SystemBindingId binding_two;
    CompileOpId compile_operation;
    CompileId compile;
    RootAttributionId root;
    DispatchGroupId dispatch_group;
    DispatchRecordId dispatch_record;
    DispatchRecordId dispatch_record_second;
    CreateOpId parent_create;
    NodeBirthId parent_birth;
    TargetEdgeId target_edge;
    CreateOpId child_create;
    NodeBirthId child_birth;
    OriginalSubmitId submit;
    PublicationId child_publication;
    GateHoldId gate_hold;
    ClaimId source_claim;
    ClaimId early_claim;
    ClaimKind source_claim_kind{ClaimKind::Cached};
    LinkAttemptId delayed_attempt;
    PublicationId parent_publication;

    explicit Trace(Limits limits = DefaultLimits()) : fixture(limits) {}

    void BuildContextWorldRegistrationCompile(ScalarAddress selected_manager = kManager) {
        manager = selected_manager;
        pass = Issued<RowPassId>(
            fixture.Send(NativeSite::RowPassBegin, 0x100, RowPassBegin{}),
            EventStatus::Pending);

        const auto initial_null_row = fixture.Send(
            NativeSite::RowReservationBegin, 0x130,
            RowReservationBegin{pass, kRow});
        Status(initial_null_row, EventStatus::Pending);
        const auto initial_null_reservation = Issued<ReservationId>(
            initial_null_row, EventStatus::Pending);

        context_setup = Issued<ContextSetupId>(
            fixture.Send(NativeSite::ContextSetupBegin, 0x110,
                         ContextSetupBegin{pass, kRow}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::ContextConstructorBegin, 0x120,
                            ContextConstructBegin{context_setup, kContext}),
               EventStatus::Accepted);
        context_birth = Issued<ContextBirthId>(
            fixture.Send(NativeSite::ContextBirthCaptured, 0x120,
                         ContextBirthCaptured{context_setup, kContext, true}),
            EventStatus::Accepted);

        const auto context_reservation = Issued<ReservationId>(
            fixture.Send(NativeSite::ContextPublicationStore, 0x110,
                         ContextPublicationBegin{context_setup, context_birth, kRow}),
            EventStatus::Pending);
        context_publication = Issued<ContextPublicationId>(
            fixture.Send(NativeSite::ContextPublicationComplete, 0x110,
                         ContextPublicationComplete{
                             context_reservation, kRow, kContext, true}),
            EventStatus::Committed);
        Status(fixture.Send(NativeSite::RowReservationComplete, 0x130,
                            RowReservationComplete{
                                initial_null_reservation, kRow, kContext}),
               EventStatus::Refused);
        world_birth = Issued<WorldBirthId>(
            fixture.Send(NativeSite::WorldSetupBegin, 0x140,
                         WorldSetupBegin{context_publication, manager, kOwner}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::WorldBirthCaptured, 0x150,
                            WorldBirthCaptured{world_birth, kWorld, true}),
               EventStatus::Accepted);
        const auto world_reservation = Issued<ReservationId>(
            fixture.Send(NativeSite::WorldPublicationStore, 0x140,
                         WorldPublicationBegin{world_birth, context_publication, kContext}),
            EventStatus::Pending);
        world_publication = Issued<WorldPublicationId>(
            fixture.Send(NativeSite::WorldPublicationComplete, 0x140,
                         WorldPublicationComplete{
                             world_reservation, kContext, kWorld, true}),
            EventStatus::Committed);

        registration = Issued<RegistrationOpId>(
            fixture.Send(NativeSite::RegistrationBegin, 0x160,
                         RegistrationBegin{
                             world_publication, kWorld, kTypeDescriptor, 0}),
            EventStatus::Pending);
        binding = Issued<SystemBindingId>(
            fixture.Send(NativeSite::RegistrationTupleStore, 0x160,
                         RegistrationStore{
                             registration,
                             kWorld + 0x746350,
                             kTypeDescriptor,
                             kCallback,
                             0,
                             0x24,
                             0x80,
                             1}),
            EventStatus::Pending);
        registration_two = Issued<RegistrationOpId>(
            fixture.Send(NativeSite::RegistrationBegin, 0x160,
                         RegistrationBegin{
                             world_publication, kWorld, kTypeDescriptor + 0x100, 0}),
            EventStatus::Pending);
        binding_two = Issued<SystemBindingId>(
            fixture.Send(NativeSite::RegistrationTupleStore, 0x160,
                         RegistrationStore{
                             registration_two,
                             kWorld + 0x746350 + 0x1F20,
                             kTypeDescriptor + 0x100,
                             0,
                             0,
                             0x28,
                             0x90,
                             2}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::WorldRegistrationBatchComplete, 0x162,
                            WorldRegistrationBatchComplete{
                                world_publication, kWorld, 2, true}),
               EventStatus::Committed);

        // The initial NULL row path never posts. A later real row pre consumes
        // the already published context/world and freezes its compile snapshot.
        const auto row_start = fixture.Send(
            NativeSite::RowReservationBegin, 0x163,
            RowReservationBegin{pass, kRow});
        const auto row_reservation = Issued<ReservationId>(row_start, EventStatus::Pending);
        iteration = Related<RowIterationId>(row_start, EventStatus::Pending);
        const auto row_result = fixture.Send(
            NativeSite::RowReservationComplete, 0x163,
            RowReservationComplete{row_reservation, kRow, kContext});
        Status(row_result, EventStatus::Accepted);
        const auto* row_context = std::get_if<ContextPublicationId>(&row_result.issued);
        REQUIRE(row_context != nullptr);
        REQUIRE(*row_context == context_publication);

        compile_operation = Issued<CompileOpId>(
            fixture.Send(NativeSite::CompileBegin, 0x170,
                         CompileBegin{world_publication, binding, iteration,
                                      kSchema, std::nullopt, std::nullopt}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::CompileReadHash, 0x171,
                            CompileHashCaptured{
                                compile_operation,
                                HashDirection::Read,
                                kHash,
                                kQuerySlot,
                                kNativeOrdinal,
                                kNativeGroup,
                                kNativeFlags,
                                true}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::CompileReadHash, 0x171,
                            CompileHashCaptured{
                                compile_operation,
                                HashDirection::Read,
                                kHash,
                                kQuerySlotTwo,
                                kNativeOrdinal,
                                kNativeGroup,
                                kNativeFlags + 1,
                                true}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::CompileReadCount, 0x171,
                            CompileCountCaptured{
                                compile_operation, HashDirection::Read, 2}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::CompileWriteCount, 0x171,
                            CompileCountCaptured{
                                compile_operation, HashDirection::Write, 0}),
               EventStatus::Accepted);
        compile = Issued<CompileId>(
            fixture.Send(NativeSite::CompileComplete, 0x170,
                         CompileComplete{compile_operation, 2, 0, true}),
            EventStatus::Committed);
    }

    void BeginRootAndDispatch(bool group_first = false,
                              CompileId selected_compile = {}) {
        root = Issued<RootAttributionId>(
            fixture.Send(NativeSite::RootWorldLoaded, 0x190,
                         RootAttributionBegin{
                             pass,
                             iteration,
                             context_publication,
                             world_publication,
                             manager,
                             kWorld,
                             0x190}),
            EventStatus::Pending);
        if (group_first) {
            dispatch_group = Issued<DispatchGroupId>(
                fixture.Send(NativeSite::DispatchGroupBegin, 0x190,
                             DispatchGroupBegin{root, kDispatch, 0x190}),
                EventStatus::Pending);
        }
        const auto captured_compile = selected_compile == CompileId{}
            ? compile : selected_compile;
        dispatch_record = Issued<DispatchRecordId>(
            fixture.Send(NativeSite::DispatchRecordCaptured, 0x190,
                         DispatchRecordCaptured{
                             root, captured_compile, binding, kTypeDescriptor,
                             kDispatchRecordAddress, 0}),
            EventStatus::Pending);
        if (!group_first) {
            dispatch_group = Issued<DispatchGroupId>(
                fixture.Send(NativeSite::DispatchGroupBegin, 0x190,
                             DispatchGroupBegin{root, kDispatch, 0x190}),
                EventStatus::Pending);
        }
        dispatch_record_second = Issued<DispatchRecordId>(
            fixture.Send(NativeSite::DispatchRecordCaptured, 0x190,
                         DispatchRecordCaptured{
                             root, captured_compile, binding, kTypeDescriptor,
                             kDispatchRecordAddressTwo, 1}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::DispatchRecordAssociated, 0x190,
                            DispatchRecordAssociated{
                                dispatch_group, dispatch_record, manager,
                                kDispatchRecordAddress, kTypeDescriptor}),
               EventStatus::Committed);
        Status(fixture.Send(NativeSite::DispatchRecordAssociated, 0x190,
                            DispatchRecordAssociated{
                                dispatch_group, dispatch_record_second, manager,
                                kDispatchRecordAddressTwo, kTypeDescriptor}),
               EventStatus::Committed);
    }

    void StartNextPassWithRoot(CompileId selected_compile = {}) {
        pass = Issued<RowPassId>(
            fixture.Send(NativeSite::RowPassBegin, 0x100, RowPassBegin{}),
            EventStatus::Pending);
        const auto row_start = fixture.Send(
            NativeSite::RowReservationBegin, 0x130,
            RowReservationBegin{pass, kRow});
        const auto reservation = Issued<ReservationId>(row_start, EventStatus::Pending);
        iteration = Related<RowIterationId>(row_start, EventStatus::Pending);
        Status(fixture.Send(NativeSite::RowReservationComplete, 0x130,
                            RowReservationComplete{reservation, kRow, kContext}),
               EventStatus::Accepted);
        BeginRootAndDispatch(false, selected_compile);
    }

    void EndCurrentPass() {
        Status(fixture.Send(NativeSite::RowIterationEnd, 0x1F0,
                            RowIterationEnd{iteration}),
               EventStatus::Revoked);
        Status(fixture.Send(NativeSite::RowPassEnd, 0x1F0,
                            RowPassEnd{pass}),
               EventStatus::Revoked);
    }

    void CreateParentAndChild(ScalarAddress parent_callback = kApprovedParentCreator,
                              ScalarAddress parent_owner = kOwner,
                              ScalarAddress child_owner = kOwner,
                              bool check_dispatch_membership_rejections = false) {
        parent_create = Issued<CreateOpId>(
            fixture.Send(NativeSite::ParentCreateBegin, 0x1A0,
                         ParentCreateBegin{
                             root, dispatch_group, 0xF00000,
                             parent_callback, 0}),
            EventStatus::Pending);
        parent_birth = Issued<NodeBirthId>(
            fixture.Send(NativeSite::NodeAllocationReturn, 0x1A1,
                         NodeBirthCaptured{
                             parent_create, kParent, parent_owner, true}),
            EventStatus::Accepted);
        Status(fixture.Send(NativeSite::NodePayloadComplete, 0x1A1,
                            NodePayloadCompleted{parent_birth, true}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::ParentBirthReturn, 0x1A0,
                            ParentCreatorReturned{
                                parent_create, kParent, parent_callback, 0}),
               EventStatus::Accepted);

        target_edge = Issued<TargetEdgeId>(
            fixture.Send(NativeSite::DescriptorTargetAssign, 0x190,
                         DescriptorTargetAssigned{
                             dispatch_group, parent_birth, kParent}),
            EventStatus::Pending);
        child_create = Issued<CreateOpId>(
            fixture.Send(NativeSite::ChildCreateBegin, 0x1A0,
                         NodeCreateBegin{
                             root, dispatch_group, parent_birth, 0xF00018,
                             kWorkloadCallback, target_edge}),
            EventStatus::Pending);
        const auto child_birth_reply = fixture.Send(
            NativeSite::NodeAllocationReturn, 0x1A1,
            NodeBirthCaptured{child_create, kChild, child_owner, true});
        child_birth = Issued<NodeBirthId>(child_birth_reply, EventStatus::Accepted);
        submit = Related<OriginalSubmitId>(child_birth_reply, EventStatus::Accepted);

        if (check_dispatch_membership_rejections) {
            const auto foreign_group = Issued<DispatchGroupId>(
                fixture.Send(NativeSite::DispatchGroupBegin, 0x190,
                             DispatchGroupBegin{root, kDispatch + 0x1000, 0x190}),
                EventStatus::Pending);
            const auto foreign_record = Issued<DispatchRecordId>(
                fixture.Send(NativeSite::DispatchRecordCaptured, 0x190,
                             DispatchRecordCaptured{
                                 root, compile, binding, kTypeDescriptor,
                                 kDispatchRecordAddress + 0x1000, 2}),
                EventStatus::Pending);
            Status(fixture.Send(NativeSite::DispatchRecordAssociated, 0x190,
                                DispatchRecordAssociated{
                                    foreign_group, foreign_record, kManager,
                                    kDispatchRecordAddress + 0x1000, kTypeDescriptor}),
                   EventStatus::Committed);

            const auto invalid_payload = [&](std::uint64_t base, std::uint32_t count,
                                             std::array<DispatchRecordId,
                                                        kMaxDispatchRecords> records) {
                Status(fixture.Send(NativeSite::ChildPayloadCopy, 0x1A1,
                                    ChildPayloadCaptured{child_birth, child_owner, kManager,
                                                         base, count, records}),
                       EventStatus::Refused);
            };
            invalid_payload(kDispatchRecordAddress, 0,
                            DispatchRecordList());
            invalid_payload(kDispatchRecordAddress, 1,
                            DispatchRecordList(dispatch_record));
            invalid_payload(kDispatchRecordAddress, 2,
                            DispatchRecordList(dispatch_record, dispatch_record));
            invalid_payload(kDispatchRecordAddress, 2,
                            DispatchRecordList(dispatch_record, foreign_record));
            invalid_payload(kDispatchRecordAddress, 2,
                            DispatchRecordList(dispatch_record_second, dispatch_record));
            invalid_payload(kDispatchRecordAddress, kMaxDispatchRecords + 1,
                            DispatchRecordList(dispatch_record, dispatch_record_second));
            invalid_payload(std::numeric_limits<ScalarAddress>::max() - 0x10, 2,
                            DispatchRecordList(dispatch_record, dispatch_record_second));
            auto trailing_record = DispatchRecordList(dispatch_record, dispatch_record_second);
            trailing_record[2] = foreign_record;
            invalid_payload(kDispatchRecordAddress, 2, trailing_record);
        }

        Status(fixture.Send(NativeSite::ChildPayloadCopy, 0x1A1,
                            ChildPayloadCaptured{
                                child_birth, child_owner,
                                kManager, kDispatchRecordAddress + 8,
                                2, DispatchRecordList(dispatch_record, dispatch_record_second)}),
               EventStatus::Refused);
        Status(fixture.Send(NativeSite::ChildPayloadCopy, 0x1A1,
                            ChildPayloadCaptured{
                                child_birth, child_owner, kManager,
                                kDispatchRecordAddress, 2,
                                DispatchRecordList(dispatch_record, dispatch_record_second)}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::ChildTargetStore, 0x1A1,
                            ChildTargetEdgeCaptured{
                                child_create, target_edge, parent_birth, child_birth,
                                kParent, kParent, kParent}),
               EventStatus::Committed);
        Status(fixture.Send(NativeSite::NodePayloadComplete, 0x1A1,
                            NodePayloadCompleted{child_birth, true}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::ChildCreatorReturn, 0x1A0,
                            ChildCreatorReturned{child_create, child_birth, kChild}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::RootKickAssignment, 0x190,
                            RootKickAssigned{
                                root, dispatch_group, submit, child_birth,
                                kRootSlot, kChild, kWorkloadCallback,
                                0, 1, 2, true}),
               EventStatus::Committed);
    }

    LinkAttemptId BeginRootLink(ScalarAddress actual_root_slot = kRootSlot,
                                ScalarAddress actual_child = kChild) {
        return Issued<LinkAttemptId>(
            fixture.Send(NativeSite::RootLinkAttemptBegin, 0x1B0,
                         RootLinkAttemptBegin{
                             submit,
                             actual_root_slot,
                             actual_child,
                             kOwner,
                             LinkMode::AcquiredGate,
                             kOwner + kGateOffset,
                             std::nullopt}),
            EventStatus::Pending);
    }

    PublicationId PrevalidateAndCommit(LinkAttemptId attempt,
                                       ScalarAddress target,
                                       ScalarAddress owner,
                                       bool native_acquired,
                                       bool native_increment_completed = true) {
        Status(fixture.Send(NativeSite::LinkPrevalidate, 0x1B0,
                            LinkPrevalidated{
                                attempt, target, owner, owner + kGateOffset,
                                native_acquired}),
               EventStatus::Pending);
        return Issued<PublicationId>(
            fixture.Send(NativeSite::LinkCommitted, 0x1B0,
                         LinkCommitted{
                             attempt, target, owner, owner + kGateOffset,
                             native_increment_completed, true, true}),
            EventStatus::Committed);
    }

    PublicationId PublishChild() {
        const auto attempt = BeginRootLink();
        child_publication = PrevalidateAndCommit(
            attempt, kChild, kOwner, true);
        Status(fixture.Send(NativeSite::RootSubmitTailReturn, 0x190,
                            RootSubmitTailReturned{submit, attempt}),
               EventStatus::Accepted);
        return child_publication;
    }

    ClaimId ClaimWork(ClaimKind kind = ClaimKind::Cached) {
        source_claim_kind = kind;
        gate_hold = Issued<GateHoldId>(
            fixture.Send(NativeSite::GateHoldBegin, 0x1C0,
                         GateHoldBegin{kOwner, kOwner + kGateOffset}),
            EventStatus::Pending);
        if (kind == ClaimKind::Cached) {
            Status(fixture.Send(NativeSite::CachedSourceReserved, 0x1C0,
                                CachedSourceReserved{gate_hold, kChild}),
                   EventStatus::Accepted);
        }
        source_claim = Issued<ClaimId>(
            fixture.Send(kind == ClaimKind::Cached
                             ? NativeSite::CachedClaim : NativeSite::FreshClaim,
                         0x1C0,
                         QueueClaimCaptured{
                             kind,
                             gate_hold,
                             kChild,
                             true}),
            EventStatus::Committed);
        Status(fixture.Send(NativeSite::ClaimTransfer, 0x1C0,
                            QueueClaimTransfer{source_claim, child_birth}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::GateHoldRelease, 0x1C0,
                            GateHoldRelease{gate_hold, kOwner}),
               EventStatus::Revoked);
        Status(fixture.Send(NativeSite::WorkCallbackBegin, 0x1C1,
                            WorkCallbackBegin{
                                source_claim, child_birth, kWorkloadCallback}),
               EventStatus::Accepted);
        return source_claim;
    }

    ClaimId EarlyCachedThenFreshWork() {
        gate_hold = Issued<GateHoldId>(
            fixture.Send(NativeSite::GateHoldBegin, 0x1C0,
                         GateHoldBegin{kOwner, kOwner + kGateOffset}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::CachedSourceReserved, 0x1C0,
                            CachedSourceReserved{gate_hold, kChild}),
               EventStatus::Accepted);
        early_claim = Issued<ClaimId>(
            fixture.Send(NativeSite::CachedClaim, 0x1C0,
                         QueueClaimCaptured{ClaimKind::Cached, gate_hold, kChild, true}),
            EventStatus::Committed);
        Status(fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                            CompletionBegin{early_claim, child_birth}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::CompletionRelease, 0x1C1,
                            CompletionRelease{early_claim, child_birth}),
               EventStatus::Revoked);
        Status(fixture.Send(NativeSite::ClaimTransfer, 0x1C1,
                            QueueClaimTransfer{early_claim, child_birth}),
               EventStatus::Refused);

        source_claim_kind = ClaimKind::Fresh;
        source_claim = Issued<ClaimId>(
            fixture.Send(NativeSite::FreshClaim, 0x1C2,
                         QueueClaimCaptured{ClaimKind::Fresh, gate_hold, kChild, true}),
            EventStatus::Committed);
        REQUIRE(source_claim != early_claim);
        Status(fixture.Send(NativeSite::ClaimTransfer, 0x1C2,
                            QueueClaimTransfer{source_claim, child_birth}),
               EventStatus::Accepted);
        Status(fixture.Send(NativeSite::GateHoldRelease, 0x1C2,
                            GateHoldRelease{gate_hold, kOwner}),
               EventStatus::Revoked);
        Status(fixture.Send(NativeSite::WorkCallbackBegin, 0x1C3,
                            WorkCallbackBegin{
                                source_claim, child_birth, kWorkloadCallback}),
               EventStatus::Accepted);
        return source_claim;
    }

    void CompleteWorkToParent(bool actual_last_run = true,
                              ScalarAddress held_owner = kOwner,
                              ScalarAddress actual_target_owner = kOwner,
                              ScalarAddress actual_gate = kOwner + kGateOffset,
                              bool claim_already_completing = false) {
        if (!claim_already_completing) {
            Status(fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                                CompletionBegin{source_claim, child_birth}),
                   EventStatus::Accepted);
        }

        gate_hold = Issued<GateHoldId>(
            fixture.Send(NativeSite::GateHoldBegin, 0x1C2,
                         GateHoldBegin{held_owner, held_owner + kGateOffset}),
            EventStatus::Pending);

        delayed_attempt = Issued<LinkAttemptId>(
            fixture.Send(NativeSite::DelayedCompletionTarget, 0x1C3,
                         DelayedLinkAttemptBegin{
                             source_claim,
                             submit,
                             child_birth,
                             target_edge,
                             parent_birth,
                             kParent,
                             actual_target_owner,
                             LinkMode::CallerHeldGate,
                             actual_gate,
                             gate_hold,
                             source_claim_kind == ClaimKind::Cached
                                 ? CompletionOrigin::CachedReturn
                                 : CompletionOrigin::WorkerReturn,
                             actual_last_run,
                             kChild,
                             kParent}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::DelayedLinkRequestEntered, 0x1B0,
                            DelayedLinkRequestEntered{
                                delayed_attempt, kChild, kParent}),
               EventStatus::Accepted);
        parent_publication = PrevalidateAndCommit(
            delayed_attempt, kParent, actual_target_owner, false);
        Status(fixture.Send(NativeSite::CompletionRelease, 0x1C3,
                            CompletionRelease{source_claim, child_birth}),
               EventStatus::Revoked);
        Status(fixture.Send(NativeSite::GateHoldRelease, 0x1C2,
                            GateHoldRelease{gate_hold, held_owner}),
               EventStatus::Revoked);
    }

    void CompleteWorkToParentDl0() {
        Status(fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                            CompletionBegin{source_claim, child_birth}),
               EventStatus::Accepted);
        delayed_attempt = Issued<LinkAttemptId>(
            fixture.Send(NativeSite::DelayedCompletionTarget, 0x1C3,
                         DelayedLinkAttemptBegin{
                             source_claim, submit, child_birth, target_edge, parent_birth,
                             kParent, kOwner, LinkMode::AcquiredGate,
                             kOwner + kGateOffset, std::nullopt,
                             CompletionOrigin::WorkerReturn, true, kChild, kParent}),
            EventStatus::Pending);
        Status(fixture.Send(NativeSite::LinkPrevalidate, 0x1B0,
                            LinkPrevalidated{
                                delayed_attempt, kParent, kOwner,
                                kOwner + kGateOffset, true}),
               EventStatus::Pending);
        parent_publication = Issued<PublicationId>(
            fixture.Send(NativeSite::LinkCommitted, 0x1B0,
                         LinkCommitted{delayed_attempt, kParent, kOwner,
                                       kOwner + kGateOffset, true, true, true}),
            EventStatus::Committed);
        Status(fixture.Send(NativeSite::CompletionRelease, 0x1C3,
                            CompletionRelease{source_claim, child_birth}),
               EventStatus::Revoked);
    }

    MaterializeOpId BeginMaterializeQueued(DispatchRecordId selected_dispatch = {}) {
        return Issued<MaterializeOpId>(
            fixture.Send(NativeSite::MaterializeBegin, 0x1D0,
                         MaterializeBegin{
                             source_claim,
                             child_publication,
                             compile,
                             binding,
                             selected_dispatch == DispatchRecordId{}
                                 ? dispatch_record : selected_dispatch,
                             parent_birth,
                             child_birth}),
            EventStatus::Pending);
    }

    MaterializeOpId BeginMaterializeRootDirect(CompileId selected_compile = {},
                                               DispatchRecordId selected_dispatch = {}) {
        return Issued<MaterializeOpId>(
            fixture.Send(NativeSite::MaterializeBegin, 0x1D0,
                         MaterializeBegin{
                             root,
                             std::nullopt,
                             selected_compile == CompileId{} ? compile : selected_compile,
                             binding,
                             selected_dispatch == DispatchRecordId{}
                                 ? dispatch_record : selected_dispatch,
                             std::nullopt,
                             std::nullopt}),
            EventStatus::Pending);
    }

    EventReply CaptureMaterializeEntry(MaterializeOpId operation,
                                       ScalarAddress source_entry,
                                       std::uint32_t first_ordinal,
                                       std::uint32_t final_ordinal,
                                       ScalarAddress component_pointer,
                                       std::uint32_t component_stride,
                                       std::uint32_t row_index,
                                       ScalarAddress output_record) {
        return fixture.Send(NativeSite::MaterializeOutput, 0x1D0,
                            MaterializeEntryCaptured{
                                operation, source_entry, first_ordinal, final_ordinal,
                                HashDirection::Read, kNativeGroup, component_pointer,
                                component_stride, row_index, output_record});
    }

    EventReply CompleteMaterialize(MaterializeOpId operation,
                                   std::uint32_t query_count,
                                   std::uint32_t completed_count) {
        return fixture.Send(NativeSite::MaterializeComplete, 0x1D0,
                            MaterializeComplete{operation, query_count, completed_count});
    }

    ObservationId MaterializeQueued(DispatchRecordId selected_dispatch = {}) {
        const auto operation = BeginMaterializeQueued(selected_dispatch);
        Status(CaptureMaterializeEntry(operation, kQuerySlot, kNativeOrdinal,
                                       kNativeOrdinal, kEmittedComponentPointer,
                                       kOutputStride, kOutputIndex,
                                       kNativeOutputRecordAddress),
               EventStatus::Accepted);
        Status(CaptureMaterializeEntry(operation, kQuerySlot, kNativeOrdinal,
                                       kNativeOrdinal + 1, kEmittedComponentPointer,
                                       kOutputStride, kOutputIndex,
                                       kNativeOutputRecordAddress),
               EventStatus::Refused);
        Status(CaptureMaterializeEntry(operation, kQuerySlotTwo, kNativeOrdinal,
                                       kNativeOrdinal + 1, kEmittedComponentPointerTwo,
                                       0, kOutputIndexTwo,
                                       kNativeOutputRecordAddressTwo),
               EventStatus::Accepted);
        return Issued<ObservationId>(CompleteMaterialize(operation, 2, 2),
                                    EventStatus::Committed);
    }

    ObservationId MaterializeRootDirect(CompileId selected_compile = {},
                                        DispatchRecordId selected_dispatch = {}) {
        const auto operation = BeginMaterializeRootDirect(selected_compile, selected_dispatch);
        Status(CaptureMaterializeEntry(operation, kQuerySlot, kNativeOrdinal,
                                       kNativeOrdinal, kEmittedComponentPointer,
                                       kOutputStride, kOutputIndex,
                                       kNativeOutputRecordAddress),
               EventStatus::Accepted);
        Status(CaptureMaterializeEntry(operation, kQuerySlotTwo, kNativeOrdinal,
                                       kNativeOrdinal + 1, kEmittedComponentPointerTwo,
                                       0, kOutputIndexTwo,
                                       kNativeOutputRecordAddressTwo),
               EventStatus::Accepted);
        return Issued<ObservationId>(CompleteMaterialize(operation, 2, 2),
                                    EventStatus::Committed);
    }
};

CompileId CompleteTwoReadCompile(Trace& trace,
                                 std::variant<RowIterationId, RootAttributionId> owner_scope,
                                 ScalarAddress schema,
                                 NativeFrameScalar frame) {
    const auto operation = Issued<CompileOpId>(
        trace.fixture.Send(NativeSite::CompileBegin, frame,
                           CompileBegin{trace.world_publication, trace.binding,
                                        owner_scope, schema,
                                        std::nullopt, std::nullopt}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::CompileReadHash, frame + 1,
                              CompileHashCaptured{
                                  operation, HashDirection::Read, kHash, kQuerySlot,
                                  kNativeOrdinal, kNativeGroup, kNativeFlags, true}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileReadHash, frame + 1,
                              CompileHashCaptured{
                                  operation, HashDirection::Read, kHash, kQuerySlotTwo,
                                  kNativeOrdinal, kNativeGroup, kNativeFlags + 1, true}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileReadCount, frame + 1,
                              CompileCountCaptured{operation, HashDirection::Read, 2}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileWriteCount, frame + 1,
                              CompileCountCaptured{operation, HashDirection::Write, 0}),
           EventStatus::Accepted);
    return Issued<CompileId>(
        trace.fixture.Send(NativeSite::CompileComplete, frame,
                           CompileComplete{operation, 2, 0, true}),
        EventStatus::Committed);
}

void TestClosedLifecycleReusesSmallStorageAcrossPassesAndPreservesRootCompile() {
    auto limits = DefaultLimits();
    limits.max_records = 24;
    limits.max_evidence_entries = 8;
    limits.max_observations = 1;
    limits.max_registration_entries = 2;
    limits.max_compile_entries = 4;
    Trace trace(limits);
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    const auto root_owned_compile = CompleteTwoReadCompile(
        trace, trace.root, kSchema + 0x100, 0x1A0);

    trace.EndCurrentPass();
    trace.StartNextPassWithRoot(root_owned_compile);
    const auto materialize = trace.BeginMaterializeRootDirect(root_owned_compile);
    trace.EndCurrentPass();
    Status(trace.CaptureMaterializeEntry(materialize, kQuerySlot, kNativeOrdinal,
                                         kNativeOrdinal, kEmittedComponentPointer,
                                         kOutputStride, kOutputIndex,
                                         kNativeOutputRecordAddress),
           EventStatus::Accepted);
    Status(trace.CaptureMaterializeEntry(materialize, kQuerySlotTwo, kNativeOrdinal,
                                         kNativeOrdinal + 1, kEmittedComponentPointerTwo,
                                         0, kOutputIndexTwo,
                                         kNativeOutputRecordAddressTwo),
           EventStatus::Accepted);
    const auto observation = Issued<ObservationId>(
        trace.CompleteMaterialize(materialize, 2, 2), EventStatus::Committed);
    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(observation, copied));
    REQUIRE(copied.compile == root_owned_compile);
    REQUIRE(copied.entry_count == 2);

    for (std::uint32_t cycle = 0; cycle < 40; ++cycle) {
        trace.StartNextPassWithRoot(root_owned_compile);
        trace.EndCurrentPass();
    }
    REQUIRE(trace.fixture.Copy(observation, copied));
}

void TestInflightChildAndParentPublicationSurvivePassEndThenReleaseRootStorage() {
    auto limits = DefaultLimits();
    limits.max_records = 48;
    limits.max_evidence_entries = 8;
    limits.max_observations = 2;
    limits.max_registration_entries = 2;
    limits.max_compile_entries = 4;
    Trace trace(limits);
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork(ClaimKind::Fresh);

    const auto closed_root = trace.root;
    trace.EndCurrentPass();
    Status(trace.fixture.Send(NativeSite::DispatchGroupBegin, 0x190,
                              DispatchGroupBegin{closed_root, kDispatch + 0x100, 0x190}),
           EventStatus::Refused);

    // This is the delayed worker path: the original W claim remains live
    // across the row/pass close, completes into P, and keeps P claimable after
    // W itself is retired.
    trace.CompleteWorkToParentDl0();
    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E0,
                              NodeRetire{trace.child_birth, kChild}),
           EventStatus::Revoked);
    const auto parent_hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1E1,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::CachedSourceReserved, 0x1E1,
                              CachedSourceReserved{parent_hold, kParent}),
           EventStatus::Accepted);
    const auto parent_claim = Issued<ClaimId>(
        trace.fixture.Send(NativeSite::CachedClaim, 0x1E1,
                           QueueClaimCaptured{ClaimKind::Cached, parent_hold,
                                              kParent, true}),
        EventStatus::Committed);
    Status(trace.fixture.Send(NativeSite::CachedClaimAbandon, 0x1E1,
                              CachedClaimAbandon{parent_claim}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::GateHoldRelease, 0x1E1,
                              GateHoldRelease{parent_hold, kOwner}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E2,
                              NodeRetire{trace.parent_birth, kParent}),
           EventStatus::Revoked);

    for (std::uint32_t cycle = 0; cycle < 24; ++cycle) {
        trace.StartNextPassWithRoot(trace.compile);
        trace.EndCurrentPass();
    }
}

void TestPositiveContextWorldRegistrationCompileAndQueueHandoff() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();

    const auto direct_observation = trace.MaterializeRootDirect();
    OwnedObservation direct_copy{};
    REQUIRE(trace.fixture.Copy(direct_observation, direct_copy));
    REQUIRE(direct_copy.context == trace.context_publication);
    REQUIRE(direct_copy.world == trace.world_publication);
    REQUIRE(direct_copy.publication == PublicationId{});
    REQUIRE(direct_copy.parent == NodeBirthId{});
    REQUIRE(direct_copy.child == NodeBirthId{});

    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork();
    const auto observation = trace.MaterializeQueued();

    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(observation, copied));
    REQUIRE(copied.context == trace.context_publication);
    REQUIRE(copied.world == trace.world_publication);
    REQUIRE(copied.binding == trace.binding);
    REQUIRE(copied.compile == trace.compile);
    REQUIRE(copied.dispatch == trace.dispatch_record);
    REQUIRE(copied.publication == trace.child_publication);
    REQUIRE(copied.parent == trace.parent_birth);
    REQUIRE(copied.child == trace.child_birth);
    REQUIRE(copied.parent_birth.identity == trace.parent_birth);
    REQUIRE(copied.child_birth.identity == trace.child_birth);
    REQUIRE(copied.entry_count == 2);
    REQUIRE(copied.entries[0].semantic_hash == kHash);
    REQUIRE(copied.entries[0].semantic_key == kSemanticKey);
    REQUIRE(copied.entries[0].first_ordinal == kNativeOrdinal);
    REQUIRE(copied.entries[0].final_ordinal == kNativeOrdinal);
    REQUIRE(copied.entries[0].original_row_index == kOutputIndex);
    REQUIRE(copied.entries[0].group == kNativeGroup);
    REQUIRE(copied.entries[0].flags == kNativeFlags);
    REQUIRE(copied.entries[0].optional);
    REQUIRE(copied.entries[0].source_entry == kQuerySlot);
    REQUIRE(copied.entries[0].emitted_component_pointer == kEmittedComponentPointer);
    REQUIRE(copied.entries[0].component_stride == kOutputStride);
    REQUIRE(copied.entries[0].native_output_record_address == kNativeOutputRecordAddress);
    REQUIRE(copied.entries[0].emitted_component_pointer
            != copied.entries[0].native_output_record_address);
    REQUIRE(copied.entries[0].native_output_record_address
            != copied.entries[0].emitted_component_pointer
                + copied.entries[0].component_stride * copied.entries[0].original_row_index);
    REQUIRE(copied.entries[1].semantic_hash == kHash);
    REQUIRE(copied.entries[1].first_ordinal == kNativeOrdinal);
    REQUIRE(copied.entries[1].final_ordinal == kNativeOrdinal + 1);
    REQUIRE(copied.entries[1].source_entry == kQuerySlotTwo);
    REQUIRE(copied.entries[1].optional);
    REQUIRE(copied.entries[1].emitted_component_pointer == 0);
    REQUIRE(copied.entries[1].component_stride == 0);
    REQUIRE(copied.entries[1].native_output_record_address == kNativeOutputRecordAddressTwo);

    trace.CompleteWorkToParent();
    REQUIRE(trace.parent_publication != PublicationId{});
    REQUIRE(trace.fixture.Copy(observation, copied));

    // The committed P publication keeps its own birth and copied W provenance.
    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E0,
                              NodeRetire{trace.child_birth, kChild}),
           EventStatus::Revoked);
    REQUIRE(!trace.fixture.Copy(observation, copied));
    REQUIRE(trace.fixture.Copy(direct_observation, direct_copy));

    const auto parent_hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1E1,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::CachedSourceReserved, 0x1E1,
                              CachedSourceReserved{parent_hold, kParent}),
           EventStatus::Accepted);
    const auto parent_claim = Issued<ClaimId>(
        trace.fixture.Send(NativeSite::CachedClaim, 0x1E1,
                           QueueClaimCaptured{
                               ClaimKind::Cached, parent_hold, kParent, true}),
        EventStatus::Committed);
    Status(trace.fixture.Send(NativeSite::ClaimTransfer, 0x1E1,
                              QueueClaimTransfer{parent_claim, trace.parent_birth}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::GateHoldRelease, 0x1E1,
                              GateHoldRelease{parent_hold, kOwner}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::WorkCallbackBegin, 0x1E2,
                              WorkCallbackBegin{
                                  parent_claim, trace.parent_birth, kApprovedParentCreator}),
           EventStatus::Refused);
    Status(trace.fixture.Send(NativeSite::MaterializeBegin, 0x1E2,
                              MaterializeBegin{
                                  parent_claim, trace.parent_publication,
                                  trace.compile, trace.binding, trace.dispatch_record,
                                  trace.parent_birth, trace.child_birth}),
           EventStatus::Refused);
    Status(trace.fixture.Send(NativeSite::CachedClaimAbandon, 0x1E2,
                              CachedClaimAbandon{parent_claim}),
           EventStatus::Revoked);

    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E1,
                              NodeRetire{trace.parent_birth, kParent}),
           EventStatus::Revoked);
    REQUIRE(!trace.fixture.Copy(observation, copied));
}

void TestDispatchMembershipAndSequentialMaterializationOnWorkingClaim() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild(kApprovedParentCreator, kOwner, kOwner, true);
    trace.PublishChild();
    trace.ClaimWork(ClaimKind::Fresh);

    const auto first_observation = trace.MaterializeQueued(trace.dispatch_record);
    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(first_observation, copied));
    REQUIRE(copied.dispatch == trace.dispatch_record);

    const auto second_observation = trace.MaterializeQueued(trace.dispatch_record_second);
    REQUIRE(trace.fixture.Copy(second_observation, copied));
    REQUIRE(copied.dispatch == trace.dispatch_record_second);
    REQUIRE(copied.entry_count == 2);

    Status(trace.fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                              CompletionBegin{trace.source_claim, trace.child_birth}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::MaterializeBegin, 0x1D0,
                              MaterializeBegin{
                                  trace.source_claim, trace.child_publication,
                                  trace.compile, trace.binding, trace.dispatch_record,
                                  trace.parent_birth, trace.child_birth}),
           EventStatus::Refused);
    trace.CompleteWorkToParent(true, kOwner, kOwner, kOwner + kGateOffset, true);
    REQUIRE(trace.parent_publication != PublicationId{});
}

void TestCopiedParentMembershipSurvivesChildFreeUntilRelocation() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork(ClaimKind::Fresh);
    trace.CompleteWorkToParentDl0();

    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E0,
                              NodeRetire{trace.child_birth, kChild}),
           EventStatus::Revoked);
    const auto hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1E1,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::CachedSourceReserved, 0x1E1,
                              CachedSourceReserved{hold, kParent}),
           EventStatus::Accepted);

    Status(trace.fixture.Send(NativeSite::RRelocationBegin, 0x1E2,
                              RRelocationBegin{kManager + 0x4A4A8}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::CachedSourceReserved, 0x1E2,
                              CachedSourceReserved{hold, kParent}),
           EventStatus::Refused);
}

void TestMaterializeCommitRequiresWholeNativeOperation() {
    Trace prefix;
    prefix.BuildContextWorldRegistrationCompile();
    prefix.BeginRootAndDispatch();
    prefix.CreateParentAndChild();
    prefix.PublishChild();
    prefix.ClaimWork(ClaimKind::Fresh);
    const auto prefix_operation = prefix.BeginMaterializeQueued();
    Status(prefix.CaptureMaterializeEntry(
               prefix_operation, kQuerySlot, kNativeOrdinal, kNativeOrdinal,
               kEmittedComponentPointer, kOutputStride, kOutputIndex,
               kNativeOutputRecordAddress),
           EventStatus::Accepted);
    const auto prefix_reply = prefix.CompleteMaterialize(prefix_operation, 2, 2);
    Status(prefix_reply, EventStatus::Incomplete);
    REQUIRE(std::holds_alternative<std::monostate>(prefix_reply.issued));

    Trace mismatched_counts;
    mismatched_counts.BuildContextWorldRegistrationCompile();
    mismatched_counts.BeginRootAndDispatch();
    const auto mismatch_operation = mismatched_counts.BeginMaterializeRootDirect();
    Status(mismatched_counts.CaptureMaterializeEntry(
               mismatch_operation, kQuerySlot, kNativeOrdinal, kNativeOrdinal,
               kEmittedComponentPointer, kOutputStride, kOutputIndex,
               kNativeOutputRecordAddress),
           EventStatus::Accepted);
    Status(mismatched_counts.CaptureMaterializeEntry(
               mismatch_operation, kQuerySlotTwo, kNativeOrdinal, kNativeOrdinal + 1,
               0, 0, kOutputIndexTwo, kNativeOutputRecordAddressTwo),
           EventStatus::Accepted);
    const auto mismatch_reply = mismatched_counts.CompleteMaterialize(mismatch_operation, 2, 1);
    Status(mismatch_reply, EventStatus::Incomplete);
    REQUIRE(std::holds_alternative<std::monostate>(mismatch_reply.issued));

    Trace empty_operation;
    empty_operation.BuildContextWorldRegistrationCompile();
    empty_operation.BeginRootAndDispatch();
    const auto empty_id = empty_operation.BeginMaterializeRootDirect();
    const auto empty_reply = empty_operation.CompleteMaterialize(empty_id, 0, 0);
    Status(empty_reply, EventStatus::Accepted);
    REQUIRE(std::holds_alternative<std::monostate>(empty_reply.issued));
}

void TestCachedEarlyClaimIsNotReusedForWork() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();

    trace.EarlyCachedThenFreshWork();
    const auto observation = trace.MaterializeQueued();
    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(observation, copied));
    REQUIRE(copied.publication == trace.child_publication);
    REQUIRE(copied.child == trace.child_birth);

    trace.CompleteWorkToParent();
    REQUIRE(trace.parent_publication != PublicationId{});
}

void TestRegistrationRetirementUsesExactSAndPreservesOtherTuples() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    const auto second_slot = kWorld + 0x746350 + 0x1F20;
    Status(trace.fixture.Send(NativeSite::RegistrationRetire, 0x1E0,
                              RegistrationRetire{second_slot}),
           EventStatus::Revoked);

    // Retiring S2 leaves the distinct S1 compile lineage usable.
    trace.BeginRootAndDispatch();

    Trace first_slot_trace;
    first_slot_trace.BuildContextWorldRegistrationCompile();
    Status(first_slot_trace.fixture.Send(
               NativeSite::RegistrationRetire, 0x1E0,
               RegistrationRetire{kWorld + 0x746350}),
           EventStatus::Revoked);
    Status(first_slot_trace.fixture.Send(
               NativeSite::CompileBegin, 0x1E1,
               CompileBegin{first_slot_trace.world_publication,
                           first_slot_trace.binding,
                           first_slot_trace.iteration,
                           kSchema + 4,
                           std::nullopt,
                           std::nullopt}),
           EventStatus::Refused);
}

void TestNullCallbackRegistrationIsMetadataOnly() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    REQUIRE(trace.binding_two != SystemBindingId{});
    trace.BeginRootAndDispatch();
    Status(trace.fixture.Send(
               NativeSite::DispatchRecordCaptured, 0x1E0,
               DispatchRecordCaptured{
                   trace.root, trace.compile, trace.binding_two,
                   kTypeDescriptor + 0x100, kDispatchRecordAddress + 0x100, 0}),
           EventStatus::Refused);
}

void TestCompileQueryPreservesNativeGroupAndPackedFlags() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    const auto operation = Issued<CompileOpId>(
        trace.fixture.Send(NativeSite::CompileBegin, 0x1E0,
                           CompileBegin{trace.world_publication, trace.binding,
                                        trace.iteration, kSchema + 4,
                                        std::nullopt, std::nullopt}),
        EventStatus::Pending);
    Status(trace.fixture.Send(
               NativeSite::CompileReadHash, 0x1E1,
               CompileHashCaptured{operation, HashDirection::Read, kHash,
                                   kQuerySlot, kNativeOrdinal, 1,
                                   kNativeFlags, true}),
           EventStatus::Refused);
    Status(trace.fixture.Send(
               NativeSite::CompileReadHash, 0x1E1,
               CompileHashCaptured{operation, HashDirection::Read, kHash,
                                   kQuerySlot, kNativeOrdinal, 0,
                                   0x00010000U | kNativeFlags, true}),
           EventStatus::Refused);
    Status(trace.fixture.Send(
               NativeSite::CompileReadHash, 0x1E1,
               CompileHashCaptured{operation, HashDirection::Read, kHash,
                                   kQuerySlot, kNativeOrdinal, 0,
                                   kNativeFlags, true}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileReadCount, 0x1E1,
                              CompileCountCaptured{operation, HashDirection::Read, 1}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileWriteCount, 0x1E1,
                              CompileCountCaptured{operation, HashDirection::Write, 0}),
           EventStatus::Accepted);
    REQUIRE(Issued<CompileId>(
                trace.fixture.Send(NativeSite::CompileComplete, 0x1E0,
                                   CompileComplete{operation, 1, 0, true}),
                EventStatus::Committed) != CompileId{});
}

void TestCommonPrelinkRetiresOnlyMatchingOwnedTarget() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();

    Status(trace.fixture.Send(
               NativeSite::LinkTargetRetire, 0x1B0,
               LinkTargetRetire{kChild, kOwner + 8}),
           EventStatus::Accepted);
    const auto hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1C0,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::FreshClaim, 0x1C0,
                              QueueClaimCaptured{
                                  ClaimKind::Fresh, hold, kChild, true}),
           EventStatus::Committed);

    Status(trace.fixture.Send(
               NativeSite::LinkTargetRetire, 0x1B1,
               LinkTargetRetire{kChild, kOwner}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::FreshClaim, 0x1C1,
                              QueueClaimCaptured{
                                  ClaimKind::Fresh, hold, kChild, true}),
           EventStatus::Refused);
}

void TestRelocationClosesManagerRootsAndCopiedQueueLineage() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork(ClaimKind::Fresh);

    Status(trace.fixture.Send(
               NativeSite::RRelocationBegin, 0x1E0,
               RRelocationBegin{kManager + 0x4A4A8}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(
               NativeSite::MaterializeBegin, 0x1E1,
               MaterializeBegin{trace.source_claim, trace.child_publication,
                                trace.compile, trace.binding, trace.dispatch_record,
                                trace.parent_birth, trace.child_birth}),
           EventStatus::Refused);

    trace.BeginRootAndDispatch();
}

void TestPostSweepRelocationUsesWorldProvenanceAndCheckedMath() {
    Trace matching;
    matching.BuildContextWorldRegistrationCompile();
    matching.BeginRootAndDispatch();
    const auto matching_observation = matching.MaterializeRootDirect();
    OwnedObservation copied{};
    REQUIRE(matching.fixture.Copy(matching_observation, copied));
    matching.EndCurrentPass();
    REQUIRE(matching.fixture.Copy(matching_observation, copied));
    Status(matching.fixture.Send(
               NativeSite::RRelocationBegin, 0x1E0,
               RRelocationBegin{kManager + 0x4A4A8}),
           EventStatus::Revoked);
    REQUIRE(!matching.fixture.Copy(matching_observation, copied));

    // World and compile lineage remain available after the negative fence.
    matching.StartNextPassWithRoot(matching.compile);

    Trace foreign;
    foreign.BuildContextWorldRegistrationCompile();
    foreign.BeginRootAndDispatch();
    const auto foreign_observation = foreign.MaterializeRootDirect();
    REQUIRE(foreign.fixture.Copy(foreign_observation, copied));
    foreign.EndCurrentPass();
    Status(foreign.fixture.Send(
               NativeSite::RRelocationBegin, 0x1E0,
               RRelocationBegin{kManager + 0x4A4A8 + 8}),
           EventStatus::Accepted);
    REQUIRE(foreign.fixture.Copy(foreign_observation, copied));

    constexpr ScalarAddress kRelocationOffset = 0x4A4A8;
    const auto overflowing_manager =
        std::numeric_limits<ScalarAddress>::max() - kRelocationOffset + 2;
    Trace overflow;
    overflow.BuildContextWorldRegistrationCompile(overflowing_manager);
    overflow.BeginRootAndDispatch();
    const auto overflow_observation = overflow.MaterializeRootDirect();
    REQUIRE(overflow.fixture.Copy(overflow_observation, copied));
    overflow.EndCurrentPass();
    // Unchecked addition would wrap this manager plus the offset to 1.
    Status(overflow.fixture.Send(
               NativeSite::RRelocationBegin, 0x1E0,
               RRelocationBegin{1}),
           EventStatus::Accepted);
    REQUIRE(overflow.fixture.Copy(overflow_observation, copied));
}

void TestNormalWorkingCompletionUsesDl0WithoutHeldGateId() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork(ClaimKind::Fresh);
    trace.CompleteWorkToParentDl0();
    REQUIRE(trace.parent_publication != PublicationId{});
}

void TestDl1RequiresItsExactHeldGate() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork(ClaimKind::Fresh);
    Status(trace.fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                              CompletionBegin{trace.source_claim, trace.child_birth}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(
               NativeSite::DelayedCompletionTarget, 0x1C3,
               DelayedLinkAttemptBegin{
                   trace.source_claim, trace.submit, trace.child_birth,
                   trace.target_edge, trace.parent_birth, kParent, kOwner,
                   LinkMode::CallerHeldGate, kOwner + kGateOffset, std::nullopt,
                   CompletionOrigin::WorkerReturn, true, kChild, kParent}),
           EventStatus::Refused);
}

void TestUnknownOverwritesCloseOnlyMatchingPriorPublications() {
    Trace context_trace;
    context_trace.BuildContextWorldRegistrationCompile();
    context_trace.BeginRootAndDispatch();
    const auto context_observation = context_trace.MaterializeRootDirect();
    OwnedObservation copied{};
    REQUIRE(context_trace.fixture.Copy(context_observation, copied));
    Status(context_trace.fixture.Send(
               NativeSite::ContextPublicationStore, 0x1E0,
               ContextPublicationBegin{ContextSetupId{}, ContextBirthId{}, kRow}),
           EventStatus::Refused);
    REQUIRE(!context_trace.fixture.Copy(context_observation, copied));
    const auto context_row_start = context_trace.fixture.Send(
        NativeSite::RowReservationBegin, 0x1E1,
        RowReservationBegin{context_trace.pass, kRow});
    const auto context_reservation = Issued<ReservationId>(
        context_row_start, EventStatus::Pending);
    Status(context_trace.fixture.Send(
               NativeSite::RowReservationComplete, 0x1E1,
               RowReservationComplete{context_reservation, kRow, kContext}),
           EventStatus::Refused);

    Trace world_trace;
    world_trace.BuildContextWorldRegistrationCompile();
    world_trace.BeginRootAndDispatch();
    const auto world_observation = world_trace.MaterializeRootDirect();
    REQUIRE(world_trace.fixture.Copy(world_observation, copied));
    Status(world_trace.fixture.Send(
               NativeSite::WorldPublicationStore, 0x1E2,
               WorldPublicationBegin{WorldBirthId{}, ContextPublicationId{}, kContext}),
           EventStatus::Refused);
    REQUIRE(!world_trace.fixture.Copy(world_observation, copied));
    const auto world_row_start = world_trace.fixture.Send(
        NativeSite::RowReservationBegin, 0x1E3,
        RowReservationBegin{world_trace.pass, kRow});
    const auto world_reservation = Issued<ReservationId>(
        world_row_start, EventStatus::Pending);
    const auto world_row_result = world_trace.fixture.Send(
        NativeSite::RowReservationComplete, 0x1E3,
        RowReservationComplete{world_reservation, kRow, kContext});
    Status(world_row_result, EventStatus::Accepted);
    const auto* preserved_context = std::get_if<ContextPublicationId>(&world_row_result.issued);
    REQUIRE(preserved_context != nullptr);
    REQUIRE(*preserved_context == world_trace.context_publication);
}

void TestMaterializeOperationRejectsMixedThreadOrFrame() {
    const auto prepare_two_entries = [](Trace& trace, MaterializeOpId operation) {
        Status(trace.CaptureMaterializeEntry(operation, kQuerySlot, kNativeOrdinal,
                                             kNativeOrdinal, kEmittedComponentPointer,
                                             kOutputStride, kOutputIndex,
                                             kNativeOutputRecordAddress),
               EventStatus::Accepted);
        Status(trace.CaptureMaterializeEntry(operation, kQuerySlotTwo, kNativeOrdinal,
                                             kNativeOrdinal + 1, kEmittedComponentPointerTwo,
                                             0, kOutputIndexTwo,
                                             kNativeOutputRecordAddressTwo),
               EventStatus::Accepted);
    };

    Trace entry_frame;
    entry_frame.BuildContextWorldRegistrationCompile();
    entry_frame.BeginRootAndDispatch();
    const auto frame_operation = entry_frame.BeginMaterializeRootDirect();
    Status(entry_frame.fixture.Send(
               NativeSite::MaterializeOutput, 0x1D1,
               MaterializeEntryCaptured{
                   frame_operation, kQuerySlot, kNativeOrdinal, kNativeOrdinal,
                   HashDirection::Read, kNativeGroup, kEmittedComponentPointer,
                   kOutputStride, kOutputIndex, kNativeOutputRecordAddress}),
           EventStatus::Refused);
    Status(entry_frame.fixture.Send(NativeSite::MaterializeComplete, 0x1D0,
                                    MaterializeComplete{frame_operation, 2, 2}),
           EventStatus::Incomplete);

    Trace entry_thread;
    entry_thread.BuildContextWorldRegistrationCompile();
    entry_thread.BeginRootAndDispatch();
    const auto thread_operation = entry_thread.BeginMaterializeRootDirect();
    Status(entry_thread.fixture.Send(
               NativeSite::MaterializeOutput, 0x1D0,
               MaterializeEntryCaptured{
                   thread_operation, kQuerySlot, kNativeOrdinal, kNativeOrdinal,
                   HashDirection::Read, kNativeGroup, kEmittedComponentPointer,
                   kOutputStride, kOutputIndex, kNativeOutputRecordAddress},
               kThread + 1),
           EventStatus::Refused);
    Status(entry_thread.fixture.Send(NativeSite::MaterializeComplete, 0x1D0,
                                     MaterializeComplete{thread_operation, 2, 2}),
           EventStatus::Incomplete);

    Trace completion_frame;
    completion_frame.BuildContextWorldRegistrationCompile();
    completion_frame.BeginRootAndDispatch();
    const auto completion_frame_operation = completion_frame.BeginMaterializeRootDirect();
    prepare_two_entries(completion_frame, completion_frame_operation);
    Status(completion_frame.fixture.Send(
               NativeSite::MaterializeComplete, 0x1D2,
               MaterializeComplete{completion_frame_operation, 2, 2}),
           EventStatus::Refused);
    Status(completion_frame.fixture.Send(
               NativeSite::MaterializeComplete, 0x1D0,
               MaterializeComplete{completion_frame_operation, 2, 2}),
           EventStatus::Incomplete);

    Trace completion_thread;
    completion_thread.BuildContextWorldRegistrationCompile();
    completion_thread.BeginRootAndDispatch();
    const auto completion_thread_operation = completion_thread.BeginMaterializeRootDirect();
    prepare_two_entries(completion_thread, completion_thread_operation);
    Status(completion_thread.fixture.Send(
               NativeSite::MaterializeComplete, 0x1D0,
               MaterializeComplete{completion_thread_operation, 2, 2},
               kThread + 1),
           EventStatus::Refused);
    Status(completion_thread.fixture.Send(
               NativeSite::MaterializeComplete, 0x1D0,
               MaterializeComplete{completion_thread_operation, 2, 2}),
           EventStatus::Incomplete);
}

void TestRootOwnedCompileRequiresPriorRowSnapshot() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    const auto original_root = trace.root;

    const auto snapshot_compile = CompleteTwoReadCompile(
        trace, original_root, kSchema + 0x100, 0x1E0);
    const auto late_operation = Issued<CompileOpId>(
        trace.fixture.Send(NativeSite::CompileBegin, 0x1F0,
                           CompileBegin{trace.world_publication, trace.binding,
                                        original_root, kSchema + 0x200,
                                        std::nullopt, std::nullopt}),
        EventStatus::Pending);

    const auto row_start = trace.fixture.Send(
        NativeSite::RowReservationBegin, 0x200,
        RowReservationBegin{trace.pass, kRow});
    const auto reservation = Issued<ReservationId>(row_start, EventStatus::Pending);
    const auto later_iteration = Related<RowIterationId>(row_start, EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::RowReservationComplete, 0x200,
                              RowReservationComplete{reservation, kRow, kContext}),
           EventStatus::Accepted);

    Status(trace.fixture.Send(NativeSite::CompileReadHash, 0x1F1,
                              CompileHashCaptured{
                                  late_operation, HashDirection::Read, kHash, kQuerySlot,
                                  kNativeOrdinal, kNativeGroup, kNativeFlags, true}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileReadHash, 0x1F1,
                              CompileHashCaptured{
                                  late_operation, HashDirection::Read, kHash, kQuerySlotTwo,
                                  kNativeOrdinal, kNativeGroup, kNativeFlags + 1, true}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileReadCount, 0x1F1,
                              CompileCountCaptured{
                                  late_operation, HashDirection::Read, 2}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::CompileWriteCount, 0x1F1,
                              CompileCountCaptured{
                                  late_operation, HashDirection::Write, 0}),
           EventStatus::Accepted);
    const auto late_compile = Issued<CompileId>(
        trace.fixture.Send(NativeSite::CompileComplete, 0x1F0,
                           CompileComplete{late_operation, 2, 0, true}),
        EventStatus::Committed);

    trace.iteration = later_iteration;
    trace.BeginRootAndDispatch(false, snapshot_compile);
    const auto snapshot_observation = trace.MaterializeRootDirect(snapshot_compile);
    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(snapshot_observation, copied));
    REQUIRE(copied.compile == snapshot_compile);

    const auto later_root = Issued<RootAttributionId>(
        trace.fixture.Send(NativeSite::RootWorldLoaded, 0x202,
                           RootAttributionBegin{
                               trace.pass, later_iteration,
                               trace.context_publication, trace.world_publication,
                               kManager, kWorld, 0x202}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::DispatchRecordCaptured, 0x202,
                              DispatchRecordCaptured{
                                  later_root, late_compile, trace.binding,
                                  kTypeDescriptor, kDispatchRecordAddress + 0x2000, 0}),
           EventStatus::Refused);
}

void TestSchemaResetIsCloseOnlyAndUnknownWorldSetupStillRevokes() {
    Trace compile_trace;
    compile_trace.BuildContextWorldRegistrationCompile();
    compile_trace.BeginRootAndDispatch();
    Status(compile_trace.fixture.Send(
               NativeSite::CompileAssociationsRetire, 0x1E0,
               CompileAssociationsRetire{kSchema}),
           EventStatus::Revoked);
    Status(compile_trace.fixture.Send(
               NativeSite::DispatchRecordCaptured, 0x1E1,
               DispatchRecordCaptured{
                   compile_trace.root, compile_trace.compile, compile_trace.binding,
                   kTypeDescriptor, kDispatchRecordAddress, 0}),
           EventStatus::Refused);

    Trace world_trace;
    world_trace.BuildContextWorldRegistrationCompile();
    Status(world_trace.fixture.Send(
               NativeSite::WorldSetupBegin, 0x1E0,
               WorldSetupBegin{std::nullopt, kManager, kOwner}),
           EventStatus::Revoked);
    Status(world_trace.fixture.Send(
               NativeSite::CompileBegin, 0x1E1,
               CompileBegin{world_trace.world_publication,
                           world_trace.binding,
                           world_trace.iteration,
                           kSchema + 8,
                           std::nullopt,
                           std::nullopt}),
           EventStatus::Refused);
}

void TestReceiptAndRootWorldScalarAreRequired() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();

    const auto wrong_adapter = trace.fixture.Send(
        NativeSite::RootWorldLoaded, 0x190,
        RootAttributionBegin{
            trace.pass, trace.iteration, trace.context_publication,
            trace.world_publication,
            kManager, kWorld, 0x190},
        kThread, kAdapter + 1, kProfile);
    Status(wrong_adapter, EventStatus::InvalidReceipt);

    const auto wrong_world = trace.fixture.Send(
        NativeSite::RootWorldLoaded, 0x190,
        RootAttributionBegin{
            trace.pass, trace.iteration, trace.context_publication,
            trace.world_publication,
            kManager, kWorld + 8, 0x190});
    Status(wrong_world, EventStatus::Refused);
    REQUIRE(std::holds_alternative<std::monostate>(wrong_world.issued));
}

void TestParentCallbackCannotBeReclassifiedAsWorkload() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();

    const auto reply = trace.fixture.Send(
        NativeSite::ParentCreateBegin, 0x1A0,
        ParentCreateBegin{
            trace.root, trace.dispatch_group, 0xF00000,
            kWorkloadCallback, 0});
    Status(reply, EventStatus::Refused);
    REQUIRE(std::holds_alternative<std::monostate>(reply.issued));
}

void TestRootSubmitRequiresExactChildAndOriginalStackSlot() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();

    Status(trace.fixture.Send(
               NativeSite::RootLinkAttemptBegin, 0x1B0,
               RootLinkAttemptBegin{
                   trace.submit, kRootSlot + 8, kChild, kOwner,
                   LinkMode::AcquiredGate, kOwner + kGateOffset, std::nullopt}),
           EventStatus::Refused);
    Status(trace.fixture.Send(
               NativeSite::RootLinkAttemptBegin, 0x1B0,
               RootLinkAttemptBegin{
                   OriginalSubmitId{}, kRootSlot, kChild, kOwner,
                   LinkMode::AcquiredGate, kOwner + kGateOffset, std::nullopt}),
           EventStatus::Refused);
}

void TestPendingLinkNeverClaimsAndMissingPostCannotReopen() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    const auto attempt = trace.BeginRootLink();
    Status(trace.fixture.Send(NativeSite::LinkPrevalidate, 0x1B0,
                              LinkPrevalidated{
                                  attempt, kChild, kOwner,
                                  kOwner + kGateOffset, true}),
           EventStatus::Pending);

    Status(trace.fixture.Send(
               NativeSite::FreshClaim, 0x1C0,
               QueueClaimCaptured{
                   ClaimKind::Fresh, GateHoldId{}, kChild, true}),
           EventStatus::Refused);
    Status(trace.fixture.Send(NativeSite::RootSubmitTailReturn, 0x190,
                              RootSubmitTailReturned{trace.submit, attempt}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::LinkCommitted, 0x1B0,
                              LinkCommitted{
                                  attempt, kChild, kOwner, kOwner + kGateOffset,
                                  true, true, true}),
           EventStatus::Refused);
}

void TestNonLastCompletionAndWrongDl1HeldOwnerRefuse() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork();
    Status(trace.fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                              CompletionBegin{trace.source_claim, trace.child_birth}),
           EventStatus::Accepted);
    trace.gate_hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1C2,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);

    Status(trace.fixture.Send(
               NativeSite::DelayedCompletionTarget, 0x1C3,
               DelayedLinkAttemptBegin{
                   trace.source_claim, trace.submit, trace.child_birth,
                   trace.target_edge, trace.parent_birth, kParent, kOwner,
                   LinkMode::CallerHeldGate, kOwner + kGateOffset,
                   trace.gate_hold, CompletionOrigin::CachedReturn,
                   false, kChild, kParent}),
           EventStatus::Refused);
    Status(trace.fixture.Send(
               NativeSite::DelayedCompletionTarget, 0x1C3,
               DelayedLinkAttemptBegin{
                   trace.source_claim, trace.submit, trace.child_birth,
                   trace.target_edge, trace.parent_birth, kParent, kOwner + 0x100,
                   LinkMode::CallerHeldGate, kOwner + kGateOffset,
                   trace.gate_hold, CompletionOrigin::CachedReturn,
                   true, kChild, kParent}),
           EventStatus::Refused);
}

void TestSourceRetireBeforeCommitAbortsPendingHandoff() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork();
    Status(trace.fixture.Send(NativeSite::CompletionBegin, 0x1C1,
                              CompletionBegin{trace.source_claim, trace.child_birth}),
           EventStatus::Accepted);
    trace.gate_hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1C2,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);
    trace.delayed_attempt = Issued<LinkAttemptId>(
        trace.fixture.Send(NativeSite::DelayedCompletionTarget, 0x1C3,
                           DelayedLinkAttemptBegin{
                               trace.source_claim, trace.submit, trace.child_birth,
                               trace.target_edge, trace.parent_birth, kParent, kOwner,
                               LinkMode::CallerHeldGate, kOwner + kGateOffset,
                               trace.gate_hold, CompletionOrigin::CachedReturn,
                               true, kChild, kParent}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::DelayedLinkRequestEntered, 0x1B0,
                              DelayedLinkRequestEntered{
                                  trace.delayed_attempt, kChild, kParent}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::LinkPrevalidate, 0x1B0,
                              LinkPrevalidated{
                                  trace.delayed_attempt, kParent, kOwner,
                                  kOwner + kGateOffset, false}),
           EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E0,
                              NodeRetire{trace.child_birth, kChild}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::LinkCommitted, 0x1B0,
                              LinkCommitted{
                                  trace.delayed_attempt, kParent, kOwner,
                                  kOwner + kGateOffset, true, true, true}),
           EventStatus::Refused);
}

void TestUnboundRetirementAndScopeInvalidationCloseCopies() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork();
    const auto observation = trace.MaterializeQueued();
    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(observation, copied));

    Status(trace.fixture.Send(NativeSite::ManagerRetireDispatch, 0x1E0,
                              ManagerRetire{kManager, ManagerRetireKind::DispatchOwner}),
           EventStatus::Revoked);
    REQUIRE(!trace.fixture.Copy(observation, copied));

    Trace schema_trace;
    schema_trace.BuildContextWorldRegistrationCompile();
    schema_trace.BeginRootAndDispatch();
    schema_trace.CreateParentAndChild();
    schema_trace.PublishChild();
    schema_trace.ClaimWork();
    const auto schema_observation = schema_trace.MaterializeQueued();
    REQUIRE(schema_trace.fixture.Copy(schema_observation, copied));
    Status(schema_trace.fixture.Send(
               NativeSite::SchemaMutateAppend, 0x1E0,
               SchemaMutate{schema_trace.world_publication, kSchema,
                            SchemaMutationKind::Append}),
           EventStatus::Revoked);
    REQUIRE(!schema_trace.fixture.Copy(schema_observation, copied));
}

void TestContextAndWorldRetirementCloseObservation() {
    Trace context_trace;
    context_trace.BuildContextWorldRegistrationCompile();
    context_trace.BeginRootAndDispatch();
    context_trace.CreateParentAndChild();
    context_trace.PublishChild();
    context_trace.ClaimWork();
    const auto context_observation = context_trace.MaterializeQueued();
    OwnedObservation copied{};
    REQUIRE(context_trace.fixture.Copy(context_observation, copied));
    Status(context_trace.fixture.Send(NativeSite::ContextRetire, 0x1E0,
                                      ContextRetire{kContext}),
           EventStatus::Revoked);
    REQUIRE(!context_trace.fixture.Copy(context_observation, copied));

    Trace world_trace;
    world_trace.BuildContextWorldRegistrationCompile();
    world_trace.BeginRootAndDispatch();
    world_trace.CreateParentAndChild();
    world_trace.PublishChild();
    world_trace.ClaimWork();
    const auto world_observation = world_trace.MaterializeQueued();
    REQUIRE(world_trace.fixture.Copy(world_observation, copied));
    Status(world_trace.fixture.Send(NativeSite::WorldRetireScene, 0x1E0,
                                    WorldRetire{kWorld, WorldRetireKind::SceneDisposer}),
           EventStatus::Revoked);
    REQUIRE(!world_trace.fixture.Copy(world_observation, copied));

    Trace owner_trace;
    owner_trace.BuildContextWorldRegistrationCompile();
    owner_trace.BeginRootAndDispatch();
    const auto owner_observation = owner_trace.MaterializeRootDirect();
    REQUIRE(owner_trace.fixture.Copy(owner_observation, copied));
    Status(owner_trace.fixture.Send(NativeSite::WorldRetireCommon, 0x1E0,
                                   WorldRetire{kOwner, WorldRetireKind::CommonOwnerDisposer}),
           EventStatus::Revoked);
    REQUIRE(!owner_trace.fixture.Copy(owner_observation, copied));
}

void TestSameAddressRebirthDoesNotReviveOriginalSubmit() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    trace.CreateParentAndChild();
    const auto old_child = trace.child_birth;
    const auto old_submit = trace.submit;

    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E0,
                              NodeRetire{old_child, kChild}),
           EventStatus::Revoked);
    const auto new_edge = Issued<TargetEdgeId>(
        trace.fixture.Send(NativeSite::DescriptorTargetAssign, 0x190,
                           DescriptorTargetAssigned{
                               trace.dispatch_group, trace.parent_birth, kParent}),
        EventStatus::Pending);
    const auto new_create = Issued<CreateOpId>(
        trace.fixture.Send(NativeSite::ChildCreateBegin, 0x1A0,
                           NodeCreateBegin{
                               trace.root, trace.dispatch_group, trace.parent_birth,
                               0xF00020, kWorkloadCallback, new_edge}),
        EventStatus::Pending);
    const auto reborn_reply = trace.fixture.Send(
        NativeSite::NodeAllocationReturn, 0x1A1,
        NodeBirthCaptured{new_create, kChild, kOwner, true});
    const auto new_child = Issued<NodeBirthId>(reborn_reply, EventStatus::Accepted);
    const auto new_submit = Related<OriginalSubmitId>(reborn_reply, EventStatus::Accepted);
    REQUIRE(new_child != old_child);
    REQUIRE(new_submit != old_submit);

    Status(trace.fixture.Send(NativeSite::ChildPayloadCopy, 0x1A1,
                              ChildPayloadCaptured{
                                  new_child, kOwner, kManager, kDispatchRecordAddress,
                                  2, DispatchRecordList(trace.dispatch_record,
                                                        trace.dispatch_record_second)}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::ChildTargetStore, 0x1A1,
                              ChildTargetEdgeCaptured{
                                  new_create, new_edge, trace.parent_birth,
                                  new_child, kParent, kParent, kParent}),
           EventStatus::Committed);
    Status(trace.fixture.Send(NativeSite::NodePayloadComplete, 0x1A1,
                              NodePayloadCompleted{new_child, true}),
           EventStatus::Accepted);
    Status(trace.fixture.Send(NativeSite::ChildCreatorReturn, 0x1A0,
                              ChildCreatorReturned{new_create, new_child, kChild}),
           EventStatus::Accepted);

    Status(trace.fixture.Send(NativeSite::RootKickAssignment, 0x190,
                              RootKickAssigned{
                                  trace.root, trace.dispatch_group, old_submit,
                                  old_child, kRootSlot, kChild,
                                  kWorkloadCallback, 0, 1, 2, true}),
           EventStatus::Refused);
    Status(trace.fixture.Send(NativeSite::RootKickAssignment, 0x190,
                              RootKickAssigned{
                                  trace.root, trace.dispatch_group, new_submit,
                                  new_child, kRootSlot, kChild,
                                  kWorkloadCallback, 0, 1, 2, true}),
           EventStatus::Committed);
}

void TestStorageAndPendingRowReservationsStayBounded() {
    REQUIRE(Ledger::RequiredStorage(Limits{}) == 0);
    auto small_limits = DefaultLimits();
    std::array<std::byte, 8> too_small{};
    const auto small = Ledger::Construct(
        std::span<std::byte>(too_small), small_limits,
        DescriptorCatalog{std::span<const DescriptorDefinition>(kCatalog)});
    REQUIRE(small.ledger == nullptr);
    REQUIRE(small.status == ConstructStatus::InsufficientStorage);

    auto one_slot_limits = DefaultLimits();
    one_slot_limits.max_records = 1;
    Fixture one_slot(one_slot_limits);
    const auto only_pass = Issued<RowPassId>(
        one_slot.Send(NativeSite::RowPassBegin, 0x100, RowPassBegin{}),
        EventStatus::Pending);
    Status(one_slot.Send(NativeSite::RowReservationBegin, 0x110,
                         RowReservationBegin{only_pass, kRow}),
           EventStatus::CapacityExhausted);

    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    auto& fixture = trace.fixture;
    const auto pass = trace.pass;
    ReservationId previous;
    RowIterationId previous_iteration;
    for (ScalarAddress index = 0; index != 40; ++index) {
        const auto start = fixture.Send(
            NativeSite::RowReservationBegin, 0x100,
            RowReservationBegin{pass, kRow});
        const auto reservation = Issued<ReservationId>(start, EventStatus::Pending);
        const auto iteration = Related<RowIterationId>(start, EventStatus::Pending);
        if (index != 0) {
            Status(fixture.Send(NativeSite::RowReservationComplete, 0x100,
                                RowReservationComplete{previous, kRow, kContext}),
                   EventStatus::Refused);
            REQUIRE(previous_iteration != iteration);
        }
        previous = reservation;
        previous_iteration = iteration;
    }
    Status(fixture.Send(NativeSite::RowPassEnd, 0x100, RowPassEnd{pass}),
           EventStatus::Revoked);
}

void TestReceiptSiteAndProfileMismatchesFailClosed() {
    Fixture fixture;
    const auto wrong_site = fixture.Send(
        NativeSite::ContextSetupBegin, 0x100,
        RowPassBegin{}, kThread, kAdapter, kProfile);
    Status(wrong_site, EventStatus::InvalidReceipt);

    const auto pass = Issued<RowPassId>(
        fixture.Send(NativeSite::RowPassBegin, 0x100, RowPassBegin{}),
        EventStatus::Pending);
    const auto wrong_profile = fixture.Send(
        NativeSite::ContextSetupBegin, 0x110,
        ContextSetupBegin{pass, kRow}, kThread, kAdapter, kProfile + 1);
    Status(wrong_profile, EventStatus::InvalidReceipt);
}

void TestCopiedBirthStampDoesNotGuardParentAndCloseAllInvalidates() {
    Trace trace;
    trace.BuildContextWorldRegistrationCompile();
    trace.BeginRootAndDispatch();
    const auto direct_observation = trace.MaterializeRootDirect();
    trace.CreateParentAndChild();
    trace.PublishChild();
    trace.ClaimWork();
    const auto observation = trace.MaterializeQueued();
    trace.CompleteWorkToParent();

    OwnedObservation copied{};
    REQUIRE(trace.fixture.Copy(observation, copied));
    Status(trace.fixture.Send(NativeSite::NodeRetire, 0x1E0,
                              NodeRetire{std::nullopt, kChild}),
           EventStatus::Revoked);
    REQUIRE(!trace.fixture.Copy(observation, copied));

    // P retains its own publication and the copied W birth provenance after W
    // retires. A witnessed acquisition can still resolve P under the gate.
    const auto parent_hold = Issued<GateHoldId>(
        trace.fixture.Send(NativeSite::GateHoldBegin, 0x1E1,
                           GateHoldBegin{kOwner, kOwner + kGateOffset}),
        EventStatus::Pending);
    Status(trace.fixture.Send(NativeSite::CachedSourceReserved, 0x1E1,
                              CachedSourceReserved{parent_hold, kParent}),
           EventStatus::Accepted);
    const auto parent_claim = Issued<ClaimId>(
        trace.fixture.Send(NativeSite::CachedClaim, 0x1E1,
                           QueueClaimCaptured{
                               ClaimKind::Cached, parent_hold, kParent, true}),
        EventStatus::Committed);
    Status(trace.fixture.Send(NativeSite::CachedClaimAbandon, 0x1E1,
                              CachedClaimAbandon{parent_claim}),
           EventStatus::Revoked);
    Status(trace.fixture.Send(NativeSite::GateHoldRelease, 0x1E1,
                              GateHoldRelease{parent_hold, kOwner}),
           EventStatus::Revoked);

    trace.fixture.CloseAll(CloseReason::Shutdown);
    REQUIRE(!trace.fixture.Copy(direct_observation, copied));
}

static_assert(std::is_default_constructible_v<RowPassId>);
static_assert(!std::is_convertible_v<RowPassId, std::uint64_t>);
static_assert(!std::is_constructible_v<NativeReceipt,
                                       NativeSite,
                                       NativeThreadScalar,
                                       NativeFrameScalar,
                                       std::uint64_t,
                                       std::uint64_t>);

} // namespace

int main() {
#define RUN_TEST(test) do { g_current_test = #test; test(); } while (false)
    RUN_TEST(TestPositiveContextWorldRegistrationCompileAndQueueHandoff);
    RUN_TEST(TestDispatchMembershipAndSequentialMaterializationOnWorkingClaim);
    RUN_TEST(TestCopiedParentMembershipSurvivesChildFreeUntilRelocation);
    RUN_TEST(TestMaterializeCommitRequiresWholeNativeOperation);
    RUN_TEST(TestCachedEarlyClaimIsNotReusedForWork);
    RUN_TEST(TestRegistrationRetirementUsesExactSAndPreservesOtherTuples);
    RUN_TEST(TestNullCallbackRegistrationIsMetadataOnly);
    RUN_TEST(TestCompileQueryPreservesNativeGroupAndPackedFlags);
    RUN_TEST(TestCommonPrelinkRetiresOnlyMatchingOwnedTarget);
    RUN_TEST(TestRelocationClosesManagerRootsAndCopiedQueueLineage);
    RUN_TEST(TestPostSweepRelocationUsesWorldProvenanceAndCheckedMath);
    RUN_TEST(TestNormalWorkingCompletionUsesDl0WithoutHeldGateId);
    RUN_TEST(TestDl1RequiresItsExactHeldGate);
    RUN_TEST(TestUnknownOverwritesCloseOnlyMatchingPriorPublications);
    RUN_TEST(TestMaterializeOperationRejectsMixedThreadOrFrame);
    RUN_TEST(TestRootOwnedCompileRequiresPriorRowSnapshot);
    RUN_TEST(TestSchemaResetIsCloseOnlyAndUnknownWorldSetupStillRevokes);
    RUN_TEST(TestReceiptAndRootWorldScalarAreRequired);
    RUN_TEST(TestParentCallbackCannotBeReclassifiedAsWorkload);
    RUN_TEST(TestRootSubmitRequiresExactChildAndOriginalStackSlot);
    RUN_TEST(TestPendingLinkNeverClaimsAndMissingPostCannotReopen);
    RUN_TEST(TestNonLastCompletionAndWrongDl1HeldOwnerRefuse);
    RUN_TEST(TestSourceRetireBeforeCommitAbortsPendingHandoff);
    RUN_TEST(TestUnboundRetirementAndScopeInvalidationCloseCopies);
    RUN_TEST(TestContextAndWorldRetirementCloseObservation);
    RUN_TEST(TestSameAddressRebirthDoesNotReviveOriginalSubmit);
    RUN_TEST(TestStorageAndPendingRowReservationsStayBounded);
    RUN_TEST(TestClosedLifecycleReusesSmallStorageAcrossPassesAndPreservesRootCompile);
    RUN_TEST(TestInflightChildAndParentPublicationSurvivePassEndThenReleaseRootStorage);
    RUN_TEST(TestReceiptSiteAndProfileMismatchesFailClosed);
    RUN_TEST(TestCopiedBirthStampDoesNotGuardParentAndCloseAllInvalidates);
#undef RUN_TEST
    return 0;
}
