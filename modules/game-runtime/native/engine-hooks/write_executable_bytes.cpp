#include "engine-hooks/write_executable_bytes.h"
#include "engine-hooks/executable_write_transaction.h"
#include "engine-hooks/held_entry_preparation.h"
#include <windows.h>
#include <tlhelp32.h>
#include <atomic>
#include <mutex>
#include <intrin.h>

namespace EngineHooks::ExecutableWrite::Detail {
class PairReservationAccess final {
public:
    static OwnerIdentity Identity(const EngineHooks::RetainedModuleOwnerToken& owner) noexcept {
        return OwnerIdentity{owner.admitted_module_, owner.load_generation_,
                             owner.controller_identity_};
    }
};
}

namespace {
std::mutex executable_writer_mutex;
EngineHooks::ExecutableWrite::Detail::PairReservationTable<64> pair_reservations;
EngineHooks::ExecutableWrite::Detail::WriteSetReservationTable<
    EngineHooks::ExecutableWrite::MaxWriteSetOwners,
    EngineHooks::ExecutableWrite::MaxWriteSetSpans> write_set_reservations;
std::atomic<std::uint32_t> held_entry_worker{};
std::atomic<unsigned> last_failure{};
using NextThread = LONG (NTAPI*)(HANDLE,HANDLE,ACCESS_MASK,ULONG,ULONG,PHANDLE);
constexpr LONG NoMoreThreads = static_cast<LONG>(0x8000001aU);

struct WindowsThreads {
    NextThread next{};
    DWORD current{GetCurrentThreadId()};
    DWORD process{GetCurrentProcessId()};
    DWORD page_size{};
    WindowsThreads() {
        // Resolve before suspending peers. GetProcAddress/loader operations are
        // forbidden inside the frozen window.
        const auto module = GetModuleHandleW(L"ntdll.dll");
        next = module ? reinterpret_cast<NextThread>(GetProcAddress(module,"NtGetNextThread")) : nullptr;
        SYSTEM_INFO system{}; GetSystemInfo(&system); page_size = system.dwPageSize;
    }
    std::uint32_t CurrentThread() const { return current; }
    std::size_t PageSize() const { return page_size; }
    bool ValidRange(std::uintptr_t address, std::size_t size) const {
        if (!next || !page_size || address/page_size != (address+size-1)/page_size) return false;
        MEMORY_BASIC_INFORMATION info{};
        if (VirtualQuery(reinterpret_cast<void*>(address),&info,sizeof(info)) != sizeof(info) ||
            info.State != MEM_COMMIT || (info.Protect & (PAGE_GUARD|PAGE_NOACCESS))) return false;
        const auto access = info.Protect & 0xff;
        return access == PAGE_EXECUTE_READ || access == PAGE_EXECUTE_READWRITE || access == PAGE_EXECUTE_WRITECOPY;
    }
    bool Inventory(bool frozen, std::array<std::uint32_t,EngineHooks::ExecutableWrite::MaxPeers>& ids,
                   std::size_t& count) const {
        count = 0;
        bool saw_current{};
        if (!frozen) {
            const auto snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD,0);
            if (snapshot == INVALID_HANDLE_VALUE) return false;
            THREADENTRY32 entry{sizeof(entry)};
            SetLastError(ERROR_SUCCESS);
            // An initial failure is never a valid empty process: the calling
            // thread necessarily exists in a complete thread snapshot.
            if (!Thread32First(snapshot,&entry)) { CloseHandle(snapshot); return false; }
            for (unsigned inspected = 0; inspected < 65536; ++inspected) {
                if (entry.dwSize < offsetof(THREADENTRY32,th32OwnerProcessID)+sizeof(entry.th32OwnerProcessID)) {
                    CloseHandle(snapshot); return false;
                }
                if (entry.th32OwnerProcessID == process) {
                    if (!entry.th32ThreadID) { CloseHandle(snapshot); return false; }
                    if (entry.th32ThreadID == current) saw_current = true;
                    else {
                        if (count == ids.size()) { CloseHandle(snapshot); return false; }
                        ids[count++] = entry.th32ThreadID;
                    }
                }
                entry.dwSize = sizeof(entry); SetLastError(ERROR_SUCCESS);
                if (Thread32Next(snapshot,&entry)) continue;
                const auto error = GetLastError();
                const bool closed = CloseHandle(snapshot) != FALSE;
                return error == ERROR_NO_MORE_FILES && closed && saw_current;
            }
            CloseHandle(snapshot); return false;
        }
        // Toolhelp can allocate from the application heap. The fresh fixed-point
        // inventory instead walks kernel thread handles into preallocated storage.
        // Missing/failed native iteration is a refusal, never an empty inventory.
        if (!next) return false;
        HANDLE previous{};
        for (std::size_t inspected = 0; inspected < ids.size()+2; ++inspected) {
            HANDLE thread{};
            const auto status = next(GetCurrentProcess(),previous,
                THREAD_SUSPEND_RESUME|THREAD_GET_CONTEXT|THREAD_QUERY_INFORMATION,0,0,&thread);
            if (previous) CloseHandle(previous);
            previous = nullptr;
            if (status == NoMoreThreads) {
                if (thread) { CloseHandle(thread); return false; }
                return saw_current;
            }
            if (status < 0 || !thread) { if (thread) CloseHandle(thread); return false; }
            const auto id = GetThreadId(thread);
            if (!id) { CloseHandle(thread); return false; }
            if (id == current) saw_current = true;
            else {
                if (count == ids.size()) { CloseHandle(thread); return false; }
                ids[count++] = id;
            }
            previous = thread;
        }
        if (previous) CloseHandle(previous);
        return false;
    }
    std::uintptr_t Open(std::uint32_t id) const {
        const auto thread = OpenThread(
            THREAD_SUSPEND_RESUME|THREAD_GET_CONTEXT|THREAD_QUERY_INFORMATION,FALSE,id);
        if (!thread) return 0;
        // Snapshot IDs may become stale before opening. Never suspend a reused
        // identifier belonging to another process or a different thread object.
        if (GetProcessIdOfThread(thread) != process || GetThreadId(thread) != id) {
            CloseHandle(thread); return 0;
        }
        return reinterpret_cast<std::uintptr_t>(thread);
    }
    bool Suspend(std::uintptr_t handle, std::uint32_t& previous) const {
        previous = SuspendThread(reinterpret_cast<HANDLE>(handle));
        return previous != DWORD(-1);
    }
    bool Pc(std::uintptr_t handle, std::uintptr_t& pc) const {
        CONTEXT context{}; context.ContextFlags = CONTEXT_CONTROL;
        if (!GetThreadContext(reinterpret_cast<HANDLE>(handle),&context)) return false;
        pc = context.Rip; return true;
    }
    bool Resume(std::uintptr_t handle, std::uint32_t previous) const {
        const auto count = ResumeThread(reinterpret_cast<HANDLE>(handle));
        return count != DWORD(-1) && count == previous+1;
    }
    void Close(std::uintptr_t handle) const { CloseHandle(reinterpret_cast<HANDLE>(handle)); }
    bool Writable(std::uintptr_t address, std::size_t size, std::uint32_t& previous) const {
        DWORD protection{};
        const auto result = VirtualProtect(reinterpret_cast<void*>(address),size,PAGE_EXECUTE_READWRITE,&protection);
        previous = protection; return result != FALSE;
    }
    bool Restore(std::uintptr_t address, std::size_t size, std::uint32_t previous) const {
        DWORD ignored{};
        return VirtualProtect(reinterpret_cast<void*>(address),size,previous,&ignored) != FALSE;
    }
    bool Flush(std::uintptr_t address, std::size_t size) const {
        return FlushInstructionCache(GetCurrentProcess(),reinterpret_cast<void*>(address),size) != FALSE;
    }
};

[[noreturn]] void uncertain_write() {
    // Returning false could make an existing caller free a published relay.
    // Do not unwind, log, allocate, or resume uncertain execution. Process exit
    // retains all potentially reachable code until the address space is gone.
    TerminateProcess(GetCurrentProcess(),ERROR_WRITE_FAULT);
    __fastfail(FAST_FAIL_FATAL_APP_EXIT);
}
EngineHooks::ExecutableWrite::Result write_locked(std::uintptr_t address,const void* bytes,
        std::size_t size,const void* expected) {
    WindowsThreads os;
    const auto result = EngineHooks::ExecutableWrite::Run(os,address,bytes,size,expected);
    last_failure.store(static_cast<unsigned>(result.failure),std::memory_order_relaxed);
    if (result.outcome == EngineHooks::ExecutableWrite::Outcome::Indeterminate) uncertain_write();
    return result;
}

bool legacy_request_range_valid(std::uintptr_t address, std::size_t size) noexcept {
    return address && size && size <= 32 &&
        address <= (std::numeric_limits<std::uintptr_t>::max)() - size;
}

void record_pair_failure(EngineHooks::ExecutableWrite::PairFailure failure) noexcept {
    last_failure.store(static_cast<unsigned>(failure), std::memory_order_relaxed);
}
}

namespace EngineHooks {
bool WriteExecutableBytes(std::uintptr_t address,const void* bytes,std::size_t size,const void* expected) {
    std::lock_guard lock(executable_writer_mutex);
    if (bytes && legacy_request_range_valid(address, size) &&
        pair_reservations.OverlapsRange(address, size)) {
        last_failure.store(ExecutableWrite::LegacyWriteBlockedByPairReservation,
                           std::memory_order_relaxed);
        return false;
    }
    if (bytes && legacy_request_range_valid(address, size) &&
        write_set_reservations.OverlapsRange(address, size)) {
        last_failure.store(ExecutableWrite::LegacyWriteBlockedByWriteSetReservation,
                           std::memory_order_relaxed);
        return false;
    }
    return write_locked(address,bytes,size,expected).outcome == ExecutableWrite::Outcome::Published;
}
bool ReplaceExecutableBytes(std::uintptr_t address,const void* expected,const void* replacement,std::size_t size) {
    if (!expected) return false;
    std::lock_guard lock(executable_writer_mutex);
    if (replacement && legacy_request_range_valid(address, size) &&
        pair_reservations.OverlapsRange(address, size)) {
        last_failure.store(ExecutableWrite::LegacyWriteBlockedByPairReservation,
                           std::memory_order_relaxed);
        return false;
    }
    if (replacement && legacy_request_range_valid(address, size) &&
        write_set_reservations.OverlapsRange(address, size)) {
        last_failure.store(ExecutableWrite::LegacyWriteBlockedByWriteSetReservation,
                           std::memory_order_relaxed);
        return false;
    }
    for (unsigned attempt=0; attempt<8; ++attempt) {
        const auto result = write_locked(address,replacement,size,expected);
        if (result.outcome == ExecutableWrite::Outcome::Published) return true;
        if (result.failure != ExecutableWrite::Failure::OccupiedPc &&
            result.failure != ExecutableWrite::Failure::UnstableInventory &&
            result.failure != ExecutableWrite::Failure::OpenThread &&
            result.failure != ExecutableWrite::Failure::SuspendThread) return false;
        Sleep(2); // Peers have been resumed before this bounded retry.
    }
    return false;
}
unsigned LastExecutableWriteFailure() noexcept { return last_failure.load(std::memory_order_relaxed); }

ExecutableWrite::SetClaim ReserveExecutableWriteSet(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    std::span<const ExecutableWrite::Request> requests) noexcept {
    using namespace ExecutableWrite;
    if (!held.ValidForCurrentThread()) {
        record_pair_failure(PairFailure::InvalidHeldEntryToken);
        return {};
    }
    if (!owner.IsRetained()) {
        record_pair_failure(PairFailure::OwnerNotRetained);
        return {};
    }
    const auto identity = ExecutableWrite::Detail::PairReservationAccess::Identity(owner);

    std::lock_guard lock(executable_writer_mutex);
    if (!held.ValidForCurrentThread()) {
        record_pair_failure(PairFailure::InvalidHeldEntryToken);
        return {};
    }
    if (!owner.IsRetained()) {
        record_pair_failure(PairFailure::OwnerNotRetained);
        return {};
    }
    if (requests.size() > MaxWriteSetSpans) {
        record_pair_failure(PairFailure::ReservationCapacity);
        return {};
    }

    std::array<Request, MaxWriteSetSpans> snapshot{};
    for (std::size_t i = 0; i < requests.size(); ++i) snapshot[i] = requests[i];
    const std::span<const Request> snapshot_view(snapshot.data(), requests.size());
    WindowsThreads os;
    const auto validation = ExecutableWrite::Detail::validate_write_set(os, snapshot_view);
    if (validation != PairFailure::None) {
        record_pair_failure(validation);
        return {};
    }

    if (ExecutableWrite::Detail::ConflictsWithPairsForSet(
            pair_reservations, identity, snapshot_view)) {
        record_pair_failure(PairFailure::ReservationConflict);
        return {};
    }

    auto claim = write_set_reservations.Reserve(identity, snapshot_view);
    if (!claim) {
        record_pair_failure(write_set_reservations.Size() >= MaxWriteSetOwners
            ? PairFailure::ReservationCapacity
            : PairFailure::ReservationConflict);
        return {};
    }
    record_pair_failure(PairFailure::None);
    return claim;
}

ExecutableWrite::WriteSetResult PublishExecutableWriteSet(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    const ExecutableWrite::SetClaim& claim,
    std::span<const ExecutableWrite::Request> requests) noexcept {
    using namespace ExecutableWrite;
    if (!held.ValidForCurrentThread()) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0,
                                    PairFailure::InvalidHeldEntryToken};
        record_pair_failure(result.failure);
        return result;
    }
    if (!owner.IsRetained()) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0,
                                    PairFailure::OwnerNotRetained};
        record_pair_failure(result.failure);
        return result;
    }
    const auto identity = ExecutableWrite::Detail::PairReservationAccess::Identity(owner);

    std::lock_guard lock(executable_writer_mutex);
    if (!held.ValidForCurrentThread()) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0,
                                    PairFailure::InvalidHeldEntryToken};
        record_pair_failure(result.failure);
        return result;
    }
    if (!owner.IsRetained()) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0,
                                    PairFailure::OwnerNotRetained};
        record_pair_failure(result.failure);
        return result;
    }
    if (requests.size() > MaxWriteSetSpans) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0,
                                    PairFailure::ReservationCapacity};
        record_pair_failure(result.failure);
        return result;
    }

    std::array<Request, MaxWriteSetSpans> snapshot{};
    for (std::size_t i = 0; i < requests.size(); ++i) snapshot[i] = requests[i];
    const std::span<const Request> snapshot_view(snapshot.data(), requests.size());
    if (!write_set_reservations.Matches(identity, claim, snapshot_view)) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0,
                                    PairFailure::InvalidClaim};
        record_pair_failure(result.failure);
        return result;
    }

    WindowsThreads os;
    const auto validation = ExecutableWrite::Detail::validate_write_set(os, snapshot_view);
    if (validation != PairFailure::None) {
        const WriteSetResult result{WriteSetOutcome::Rejected, 0, validation};
        record_pair_failure(result.failure);
        return result;
    }

    const auto result = ExecutableWrite::Detail::RunWriteSet(snapshot_view,
        [&held, &owner]() noexcept -> PairFailure {
            if (!held.ValidForCurrentThread()) return PairFailure::InvalidHeldEntryToken;
            if (!owner.IsRetained()) return PairFailure::OwnerNotRetained;
            return PairFailure::None;
        },
        [](const Request& request) noexcept -> ExecutableWrite::Result {
            return write_locked(request.address, request.replacement,
                                request.size, request.expected);
        });
    record_pair_failure(result.failure);
    if (result.outcome == WriteSetOutcome::Indeterminate) uncertain_write();
    return result;
}

ExecutableWrite::PairClaim ReserveExecutableWritePair(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    const ExecutableWrite::Pair& pair) noexcept {
    if (!held.ValidForCurrentThread()) {
        record_pair_failure(ExecutableWrite::PairFailure::InvalidHeldEntryToken);
        return {};
    }
    if (!owner.IsRetained()) {
        record_pair_failure(ExecutableWrite::PairFailure::OwnerNotRetained);
        return {};
    }
    const auto identity = ExecutableWrite::Detail::PairReservationAccess::Identity(owner);

    std::lock_guard lock(executable_writer_mutex);
    if (!held.ValidForCurrentThread()) {
        record_pair_failure(ExecutableWrite::PairFailure::InvalidHeldEntryToken);
        return {};
    }
    WindowsThreads os;
    const auto validation = ExecutableWrite::Detail::validate_pair(os, pair);
    if (validation != ExecutableWrite::PairFailure::None) {
        record_pair_failure(validation);
        return {};
    }
    if (ExecutableWrite::Detail::ConflictsWithSetsForPair(
            write_set_reservations, identity, pair)) {
        record_pair_failure(ExecutableWrite::PairFailure::ReservationConflict);
        return {};
    }
    auto claim = pair_reservations.Reserve(identity, pair);
    if (!claim) {
        const auto failure = pair_reservations.Size() >= 64
            ? ExecutableWrite::PairFailure::ReservationCapacity
            : ExecutableWrite::PairFailure::ReservationConflict;
        record_pair_failure(failure);
        return {};
    }
    record_pair_failure(ExecutableWrite::PairFailure::None);
    return claim;
}

ExecutableWrite::PairResult PublishExecutableWritePair(
    const HeldEntryToken& held, const RetainedModuleOwnerToken& owner,
    const ExecutableWrite::PairClaim& claim,
    const ExecutableWrite::Pair& pair) noexcept {
    if (!held.ValidForCurrentThread()) {
        const ExecutableWrite::PairResult result{
            ExecutableWrite::PairOutcome::Rejected,
            ExecutableWrite::PairFailure::InvalidHeldEntryToken};
        record_pair_failure(result.failure);
        return result;
    }
    if (!owner.IsRetained()) {
        const ExecutableWrite::PairResult result{
            ExecutableWrite::PairOutcome::Rejected,
            ExecutableWrite::PairFailure::OwnerNotRetained};
        record_pair_failure(result.failure);
        return result;
    }
    const auto identity = ExecutableWrite::Detail::PairReservationAccess::Identity(owner);

    std::lock_guard lock(executable_writer_mutex);
    if (!held.ValidForCurrentThread()) {
        const ExecutableWrite::PairResult result{
            ExecutableWrite::PairOutcome::Rejected,
            ExecutableWrite::PairFailure::InvalidHeldEntryToken};
        record_pair_failure(result.failure);
        return result;
    }
    if (!pair_reservations.Matches(identity, claim, pair)) {
        const ExecutableWrite::PairResult result{
            ExecutableWrite::PairOutcome::Rejected,
            ExecutableWrite::PairFailure::InvalidClaim};
        record_pair_failure(result.failure);
        return result;
    }
    WindowsThreads os;
    const auto validation = ExecutableWrite::Detail::validate_pair(os, pair);
    if (validation != ExecutableWrite::PairFailure::None) {
        const ExecutableWrite::PairResult result{
            ExecutableWrite::PairOutcome::Rejected, validation};
        record_pair_failure(result.failure);
        return result;
    }

    const auto result = ExecutableWrite::Detail::RunPair(os, pair);
    record_pair_failure(result.failure);
    if (result.outcome == ExecutableWrite::PairOutcome::Indeterminate)
        uncertain_write();
    return result;
}

bool HeldEntryToken::ValidForCurrentThread() const noexcept {
    return worker_ && worker_ == GetCurrentThreadId() &&
        held_entry_worker.load(std::memory_order_acquire) == worker_ &&
        proof_.parked_thread_id && proof_.parked_thread_id != worker_ &&
        proof_.is_held && proof_.is_held(proof_.parked_thread_id);
}
bool RunHeldEntryPreparation(const HeldEntryProof& proof,HeldEntryCallback prepare,void* context) noexcept {
    const auto worker = GetCurrentThreadId();
    if (proof.size != sizeof(proof) || proof.version != 1 || !prepare || !proof.is_held ||
        !proof.parked_thread_id || proof.parked_thread_id == worker || !proof.is_held(proof.parked_thread_id)) return false;
    std::uint32_t empty{};
    if (!held_entry_worker.compare_exchange_strong(empty,worker,std::memory_order_acq_rel)) return false;
    const HeldEntryToken token(proof,worker);
    const bool result = token.ValidForCurrentThread() && prepare(token,context) && token.ValidForCurrentThread();
    held_entry_worker.store(0,std::memory_order_release);
    return result;
}
}
