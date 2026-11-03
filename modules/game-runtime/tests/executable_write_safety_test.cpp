#include <windows.h>
#include <tlhelp32.h>
#include <array>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <type_traits>

namespace TestWindows {
enum class Enumeration { Complete, FirstFailure, FirstEmpty, NextFailure, NextNoError, BadSize, SnapshotFailure };
Enumeration enumeration{};
unsigned closes{}, native_calls{}, fatal_calls{};
bool bad_native{}, no_native_progress{}, fail_flush{}, wrong_owner{}, wrong_thread{}, held{true};
DWORD parked{};
HANDLE WINAPI Snapshot(DWORD,DWORD) {return enumeration==Enumeration::SnapshotFailure ? INVALID_HANDLE_VALUE : reinterpret_cast<HANDLE>(0x1000);}
BOOL WINAPI First(HANDLE,LPTHREADENTRY32 entry) {
    if (enumeration==Enumeration::FirstFailure || enumeration==Enumeration::FirstEmpty) {
        SetLastError(enumeration==Enumeration::FirstEmpty ? ERROR_NO_MORE_FILES : ERROR_ACCESS_DENIED);return FALSE;
    }
    entry->dwSize=enumeration==Enumeration::BadSize ? 1 : sizeof(*entry);
    entry->th32OwnerProcessID=GetCurrentProcessId();entry->th32ThreadID=GetCurrentThreadId();return TRUE;
}
BOOL WINAPI Next(HANDLE,LPTHREADENTRY32) {SetLastError(enumeration==Enumeration::NextFailure ? ERROR_ACCESS_DENIED : enumeration==Enumeration::NextNoError ? ERROR_SUCCESS : ERROR_NO_MORE_FILES);return FALSE;}
BOOL WINAPI Close(HANDLE) {++closes;return TRUE;}
HANDLE WINAPI Open(DWORD,BOOL,DWORD id) {return reinterpret_cast<HANDLE>(std::uintptr_t(id)+0x3000);}
DWORD WINAPI ThreadProcess(HANDLE) {return GetCurrentProcessId()+(wrong_owner?1:0);}
DWORD WINAPI ThreadId(HANDLE handle) {
    const auto value=reinterpret_cast<std::uintptr_t>(handle);
    return value>=0x3000&&!wrong_thread ? static_cast<DWORD>(value-0x3000) : GetCurrentThreadId();
}
LONG NTAPI NativeNext(HANDLE,HANDLE previous,ACCESS_MASK,ULONG,ULONG,PHANDLE output) {
    ++native_calls;
    if (bad_native) return static_cast<LONG>(0xc0000022U);
    if (!previous || no_native_progress) {*output=reinterpret_cast<HANDLE>(0x2000);return 0;}
    return static_cast<LONG>(0x8000001aU);
}
FARPROC WINAPI Proc(HMODULE,LPCSTR) {return reinterpret_cast<FARPROC>(&NativeNext);}
BOOL WINAPI Protect(LPVOID,SIZE_T,DWORD,PDWORD previous) {*previous=PAGE_EXECUTE_READWRITE;return TRUE;}
BOOL WINAPI Flush(HANDLE,LPCVOID,SIZE_T) {return !fail_flush;}
struct Fatal {};
BOOL WINAPI Terminate(HANDLE,UINT) {++fatal_calls;throw Fatal{};}
}
#define CreateToolhelp32Snapshot TestWindows::Snapshot
#define Thread32First TestWindows::First
#define Thread32Next TestWindows::Next
#define CloseHandle TestWindows::Close
#define OpenThread(...) TestWindows::Open(__VA_ARGS__)
#define GetProcessIdOfThread TestWindows::ThreadProcess
#define GetThreadId TestWindows::ThreadId
#define GetProcAddress TestWindows::Proc
#define VirtualProtect TestWindows::Protect
#define FlushInstructionCache TestWindows::Flush
#define TerminateProcess TestWindows::Terminate
#include "../native/engine-hooks/write_executable_bytes.cpp"
#undef CreateToolhelp32Snapshot
#undef Thread32First
#undef Thread32Next
#undef CloseHandle
#undef OpenThread
#undef GetProcessIdOfThread
#undef GetThreadId
#undef GetProcAddress
#undef VirtualProtect
#undef FlushInstructionCache
#undef TerminateProcess

namespace {
unsigned failures{}, checks{};
void check(bool value,const char* label) {++checks;if(!value){++failures;std::cerr<<"FAIL "<<label<<'\n';}}
using namespace EngineHooks::ExecutableWrite;
struct FakeOs {
    std::array<std::uint8_t,8> memory{1,2,3,4,5,6,7,8};
    std::array<unsigned,32> suspended{}, resumed{}, closed{};
    unsigned inventories{}, opens{}, protects{}, restores{}, flushes{};
    bool initial_failure{}, fresh_failure{}, new_thread{}, endless{}, occupied{}, occupied_child{}, open_failure{}, suspend_failure{}, pc_failure{}, resume_failure{};
    unsigned fail_protect{}, fail_restore{}, fail_flush{};
    bool all_flushes_fail{}, all_restores_fail{};
    std::uint32_t CurrentThread() const {return 1;}
    bool ValidRange(std::uintptr_t,std::size_t) const {return true;}
    bool Inventory(bool fresh,std::array<std::uint32_t,MaxPeers>& ids,std::size_t& count) {
        ++inventories;if((!fresh&&initial_failure)||(fresh&&fresh_failure))return false;
        count=0;ids[count++]=1;ids[count++]=2;
        if (fresh&&new_thread) ids[count++]=3;
        if (fresh&&endless) for(unsigned i=3;i<3+inventories;++i)ids[count++]=i;
        return true;
    }
    std::uintptr_t Open(std::uint32_t id) {++opens;return open_failure?0:id;}
    bool Suspend(std::uintptr_t id,std::uint32_t& previous) {if(suspend_failure)return false;previous=0;++suspended[id];return true;}
    bool Pc(std::uintptr_t id,std::uintptr_t& pc) {pc=reinterpret_cast<std::uintptr_t>(memory.data())+((occupied||(occupied_child&&id==3))?1:64);return !pc_failure;}
    bool Resume(std::uintptr_t id,std::uint32_t) {++resumed[id];return !resume_failure;}
    void Close(std::uintptr_t id) {++closed[id];}
    bool Writable(std::uintptr_t,std::size_t,std::uint32_t& previous) {previous=0x20;return ++protects!=fail_protect;}
    bool Restore(std::uintptr_t,std::size_t,std::uint32_t) {++restores;return !all_restores_fail&&restores!=fail_restore;}
    bool Flush(std::uintptr_t,std::size_t) {++flushes;return !all_flushes_fail&&flushes!=fail_flush;}
    Result run() {const std::array<std::uint8_t,3> expected{1,2,3},replacement{9,8,7};return Run(*this,reinterpret_cast<std::uintptr_t>(memory.data()),replacement.data(),3,expected.data());}
};

void policy_tests() {
    for (unsigned failure=0;failure<6;++failure) {
        FakeOs os;os.initial_failure=failure==0;os.fresh_failure=failure==1;os.open_failure=failure==2;
        os.suspend_failure=failure==3;os.pc_failure=failure==4;os.occupied=failure==5;
        const auto before=os.memory;const auto result=os.run();
        check(result.outcome==Outcome::Rejected,"incomplete enumeration/open/suspend/context/occupied-PC rejects");
        check(os.memory==before&&os.protects==0,"prewrite refusal leaves bytes and protection untouched");
        check(os.suspended[2]==os.resumed[2],"only acquired suspensions are balanced");
        check(os.opens==0||os.closed[2]==(!os.open_failure?1u:0u),"opened handles closed on refusal");
    }
    {
        FakeOs os;os.new_thread=true;const auto result=os.run();
        check(result.outcome==Outcome::Published,"new in-process child joined fixed point before write");
        check(os.inventories==3&&os.suspended[2]==1&&os.suspended[3]==1,"fresh inventory includes and freezes new child");
        check(os.resumed[2]==1&&os.resumed[3]==1&&os.closed[2]==1&&os.closed[3]==1,"complete child cleanup");
    }
    {
        FakeOs os;os.new_thread=true;os.occupied_child=true;const auto before=os.memory;const auto result=os.run();
        check(result.outcome==Outcome::Rejected&&result.failure==Failure::OccupiedPc&&os.memory==before,"unsafe newly created child's PC refuses before write");
        check(os.resumed[2]==1&&os.resumed[3]==1,"new occupied child and original peer both resumed");
    }
    {
        FakeOs os;os.fail_protect=1;const auto before=os.memory;const auto result=os.run();
        check(result.outcome==Outcome::Rejected&&os.memory==before&&os.flushes==0,"first writable-protection failure never mutates code");
    }
    {
        FakeOs os;os.endless=true;const auto before=os.memory;const auto result=os.run();
        check(result.outcome==Outcome::Rejected&&result.failure==Failure::UnstableInventory,"unbounded thread creation refuses at bounded limit");
        check(os.memory==before&&os.inventories==MaxInventories+1,"bounded inventory attempts never write unstable code");
    }
    for(bool protection_failure:{false,true}) {
        FakeOs os;os.fail_flush=protection_failure?0:1;os.fail_restore=protection_failure?1:0;
        const auto before=os.memory;const auto result=os.run();
        check(result.outcome==Outcome::Rejected&&os.memory==before,"flush or first protection-restore failure verified rollback");
        check(os.flushes==2&&os.resumed[2]==1,"rollback flushed and peers resumed only after verified original restore");
    }
    for(unsigned failure=0;failure<3;++failure) {
        FakeOs os;os.all_flushes_fail=failure==0;os.all_restores_fail=failure==1;os.resume_failure=failure==2;
        const auto result=os.run();
        check(result.outcome==Outcome::Indeterminate,"unknown cache/protection/resume state cannot return ordinary rejection or success");
        if(failure!=2)check(os.resumed[2]==0&&os.closed[2]==0,"uncertain execution is retained frozen for fail-fast without cleanup/free");
    }
}
void adapter_tests() {
    for(auto failure:{TestWindows::Enumeration::FirstFailure,TestWindows::Enumeration::FirstEmpty,
            TestWindows::Enumeration::NextFailure,TestWindows::Enumeration::NextNoError,TestWindows::Enumeration::BadSize,TestWindows::Enumeration::SnapshotFailure}) {
        TestWindows::enumeration=failure;WindowsThreads os;std::array<std::uint32_t,MaxPeers> ids{};std::size_t count{};
        check(!os.Inventory(false,ids,count),"actual initial adapter refuses First failure/empty, Next error and malformed snapshot");
    }
    TestWindows::enumeration=TestWindows::Enumeration::Complete;
    WindowsThreads os;std::array<std::uint32_t,MaxPeers> ids{};std::size_t count{};
    check(os.Inventory(false,ids,count)&&count==0,"only ERROR_NO_MORE_FILES is complete initial EOF");
    TestWindows::bad_native=true;check(!os.Inventory(true,ids,count),"native fresh iterator failure is not empty success");TestWindows::bad_native=false;
    TestWindows::no_native_progress=true;check(!os.Inventory(true,ids,count),"native iterator without progress is bounded");TestWindows::no_native_progress=false;
    check(os.Inventory(true,ids,count),"native iterator complete EOF succeeds");
    const auto next=os.next;os.next=nullptr;
    check(!os.Inventory(true,ids,count),"missing native iterator refuses without fallback");os.next=next;
    const auto test_id=GetCurrentThreadId()+11;
    auto handle=os.Open(test_id);check(handle!=0,"opened peer handle process and thread identity match");if(handle)os.Close(handle);
    TestWindows::wrong_owner=true;
    check(os.Open(test_id)==0,"reused snapshot ID in another process is closed before suspension");TestWindows::wrong_owner=false;
    TestWindows::wrong_thread=true;
    check(os.Open(test_id)==0,"changed opened thread identity is refused before suspension");TestWindows::wrong_thread=false;
    auto* memory=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE));
    check(memory!=nullptr,"test-only executable page allocated");
    if(memory) {
        memory[0]=1;const std::uint8_t expected=1,replacement=2;
        TestWindows::fail_flush=true;bool stopped{};
        try {EngineHooks::WriteExecutableBytes(reinterpret_cast<std::uintptr_t>(memory),&replacement,1,&expected);}
        catch(const TestWindows::Fatal&) {stopped=true;}
        check(stopped&&TestWindows::fatal_calls==1,"actual public bool adapter fails fast on unknown rollback flush rather than permitting relay free");
        TestWindows::fail_flush=false;VirtualFree(memory,0,MEM_RELEASE);
    }
}
bool __cdecl proof_held(std::uint32_t id) noexcept {return TestWindows::held&&id==TestWindows::parked;}
bool prepared(const EngineHooks::HeldEntryToken& token,void* seen) noexcept {*static_cast<bool*>(seen)=token.ValidForCurrentThread();return true;}
bool withdrawn(const EngineHooks::HeldEntryToken& token,void*) noexcept {if(!token.ValidForCurrentThread())return false;TestWindows::held=false;return true;}
void token_tests() {
    static_assert(!std::is_copy_constructible_v<EngineHooks::HeldEntryToken>);
    static_assert(!std::is_default_constructible_v<EngineHooks::HeldEntryToken>);
    EngineHooks::HeldEntryProof proof;proof.parked_thread_id=TestWindows::parked=GetCurrentThreadId()+100;proof.is_held=&proof_held;
    bool seen{};check(EngineHooks::RunHeldEntryPreparation(proof,&prepared,&seen)&&seen,"private verified held scope issues current-worker token");
    TestWindows::held=false;seen=false;check(!EngineHooks::RunHeldEntryPreparation(proof,&prepared,&seen)&&!seen,"unprepared/runtime scope refuses before callback");
    TestWindows::held=true;check(!EngineHooks::RunHeldEntryPreparation(proof,&withdrawn,nullptr),"gate must remain held through preparation completion");
    TestWindows::held=true;proof.version=2;seen=false;
    check(!EngineHooks::RunHeldEntryPreparation(proof,&prepared,&seen)&&!seen,"unknown private proof version cannot issue a token");proof.version=1;
    proof.size=0;seen=false;
    check(!EngineHooks::RunHeldEntryPreparation(proof,&prepared,&seen)&&!seen,"wrong proof size cannot issue a token");proof.size=sizeof(proof);
    TestWindows::held=true;proof.parked_thread_id=GetCurrentThreadId();seen=false;
    check(!EngineHooks::RunHeldEntryPreparation(proof,&prepared,&seen)&&!seen,"parked thread cannot self-authorize worker preparation");
}
}
int main() {
    policy_tests();adapter_tests();token_tests();
    std::cout<<checks<<" checks, "<<failures<<" failures\n";return failures?1:0;
}
