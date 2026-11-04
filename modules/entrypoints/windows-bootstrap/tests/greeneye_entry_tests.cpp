// Own inert EP bytes and loader callbacks only. Never loads a game/plugin DLL.
#include "../startup_gate.h"
#include "../greeneye_entry.h"
#include <cassert>
#include <thread>
#include <iostream>
#include <cstdlib>
namespace {
unsigned scenario{};
unsigned calls{},confirms{},entries{};
int result{};
unsigned char* code{};
std::uintptr_t module{};
int __cdecl MockGreeneyeEntry() {
    ++entries;
    assert(calls==1&&confirms==1);
    // Model the host's normal EP callback restoring its captured original bytes.
    std::memcpy(code,StartupGate::original,14);
    FlushInstructionCache(GetCurrentProcess(),code,14);
    return reinterpret_cast<int(__cdecl*)()>(code)();
}
std::uintptr_t __cdecl Load(const wchar_t* root,const GreeneyeEntry::TicketV1* ticket) {
    ++calls;assert(std::wstring(root)==L"inert original root"&&GreeneyeEntry::Valid(*ticket));
    assert(!StartupGate::IsHeld(StartupGate::parked_thread_id.load()));
    assert(!std::memcmp(code,StartupGate::original,14)); // Restore BEFORE loader.
    unsigned char jump[12]{0x48,0xb8};
    auto target=&MockGreeneyeEntry;std::memcpy(jump+2,&target,8);jump[10]=0xff;jump[11]=0xe0;
    if(scenario!=2)std::memcpy(code,jump,12); // Missing hook must fail, not scan-ready.
    if(scenario==3 || scenario==7)code[13]^=1; // Trailing-byte ownership is part of the witness.
    FlushInstructionCache(GetCurrentProcess(),code,14);
    module=reinterpret_cast<std::uintptr_t>(target)-(scenario>=5 && scenario!=6?0x2080:0x2050);
    return module;
}
bool __cdecl Confirm(const wchar_t*,const GreeneyeEntry::TicketV1*,std::uintptr_t actual,bool witness) {
    ++confirms;assert(actual==module);return witness;
}
}
int main(int argc,char** argv) {
    scenario=argc>1?static_cast<unsigned>(std::atoi(argv[1])):0;
    code=static_cast<unsigned char*>(VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE));assert(code);
    unsigned char original[14]{0xb8,77,0,0,0,0xc3,0x90,0x90,0x90,0x90,0x90,0x90,0x90,0x90};
    std::memcpy(StartupGate::original,original,14);StartupGate::entrypoint=code;
    unsigned char jump[14]{0xff,0x25,0,0,0,0};auto destination=&StartupGate::Run;std::memcpy(jump+6,&destination,8);std::memcpy(code,jump,14);
    StartupGate::entered=CreateEventW(nullptr,TRUE,FALSE,nullptr);StartupGate::completed=CreateEventW(nullptr,TRUE,FALSE,nullptr);
    std::thread entry([]{result=StartupGate::Run();});
    assert(WaitForSingleObject(StartupGate::entered,5000)==WAIT_OBJECT_0);
    const auto parked=StartupGate::parked_thread_id.load();assert(StartupGate::IsHeld(parked));
    GreeneyeEntry::Context context{L"inert original root",{48,1,1,0,{}},&Load,&Confirm};
    if(scenario>=5)context.ticket.kind=scenario==8?99:2;
    if(scenario) {
        if(scenario==4) {code[0]^=1;assert(!StartupGate::ArmPostRestore(parked,&GreeneyeEntry::Continue,&context));}
        else {
            assert(StartupGate::ArmPostRestore(parked,&GreeneyeEntry::Continue,&context));
            assert(!StartupGate::ArmPostRestore(parked,&GreeneyeEntry::Continue,&context));
        }
    }
    StartupGate::Complete(true);
    if(scenario==1||scenario==5)assert(StartupGate::AwaitPostRestore()); // Worker barrier before create.
    if(scenario==2||scenario==3||scenario>=6){if(StartupGate::AwaitPostRestore())return 95;entry.join();return 96;}
    entry.join();
    assert(result==77);
    assert(scenario?(calls==1&&confirms==1&&entries==1):(calls==0&&confirms==0&&entries==0));
    if(scenario)assert(!StartupGate::RestoreAndContinue()); // No callback/EP revival.
    std::cout<<"greeneye gate scenario "<<scenario<<" passed\n";
    return 0;
}
