#include "semantic-attribution/native_cpu_state.h"
#include "semantic-attribution/native_frame_capture.h"
#include <windows.h>
#include <cstdint>
#include <cstring>
#include <cstdio>
#include <cstdlib>

extern "C" std::uint64_t SF_SemanticXsaveMask{};
extern "C" int SF_SemanticOwnedThunkFixture();
extern "C" void SF_SemanticOwnedClobber();
extern "C" void SF_SemanticNativeThunk();
extern "C" std::uintptr_t SF_SemanticOwnedInvokeRelay(void*,std::uintptr_t*);
namespace {bool captured{};bool unwind_ok{};bool frames_ok{};bool synthetic{};std::uintptr_t expected_stack{},expected_relay_return{};}
extern "C" void SF_SemanticNativeDispatch(std::uint64_t* f,void*) noexcept {
    captured=synthetic?(f[14]==expected_stack && f[16]==expected_relay_return &&
        f[14]==reinterpret_cast<std::uintptr_t>(f)+0x90+0x20):
        f[14]==0x1122334455667788ULL && f[13]==0x8877665544332211ULL &&
        f[12]==0x1020304050607080ULL && f[7]==0x0fedcba987654321ULL &&
        f[6]==0x7654321012345678ULL && f[5]==0x0011223344556677ULL &&
        f[4]==0x7766554433221100ULL && (f[15]&1);
    CONTEXT c{};RtlCaptureContext(&c);
    for(unsigned i=0;i<8;++i) {
        DWORD64 image{};auto* row=RtlLookupFunctionEntry(c.Rip,&image,nullptr);
        if(!row) break;
        PVOID handler{};DWORD64 establisher{};
        RtlVirtualUnwind(UNW_FLAG_NHANDLER,image,c.Rip,row,&c,&handler,&establisher,nullptr);
        // The common bridge's frame register must unwind dynamic alignment and
        // restore its saved GPRs to the genuine caller after its own CALL.
        if(c.Rip==f[16]) {unwind_ok=c.Rsp==reinterpret_cast<DWORD64>(f)+0x88;break;}
    }
    const auto image=reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    const auto* dos=reinterpret_cast<const IMAGE_DOS_HEADER*>(image);
    const auto* nt=reinterpret_cast<const IMAGE_NT_HEADERS64*>(image+dos->e_lfanew);
    SemanticAttribution::Native::Site site{};
    site.rva=static_cast<std::uint32_t>(f[17]-image-5);site.expected_size=5;
    SemanticAttribution::Native::FrameChain frames{};
    frames_ok=SemanticAttribution::Native::CaptureOriginalFrames(site,
        SemanticAttribution::Native::CapturePhase::After,
        *reinterpret_cast<SemanticAttribution::Native::SavedMachine*>(f),image,nt->OptionalHeader.SizeOfImage,frames);
    frames_ok=frames_ok && frames.count>1 && frames.frames[0].entry_rsp>reinterpret_cast<std::uintptr_t>(f)+0x90;
    SetLastError(0xDEAD);
    SF_SemanticOwnedClobber();
}
int main() {
    SemanticAttribution::Native::CpuStateSupport support{};
    if(!QueryCpuStateSupport(support)) {std::puts("SF-NA-30 unsupported CPU state, no relay publication permitted");return 77;}
    SF_SemanticXsaveMask=support.enabled_mask;
    SetLastError(0x1234);
    const auto good=SF_SemanticOwnedThunkFixture();
    if(!good || !captured || !unwind_ok || !frames_ok || GetLastError()!=0x1234) {
        std::printf("SF-NA-30 owned thunk failed gpr=%d captured=%d unwind=%d frames=%d last=%lu\n",good,captured,unwind_ok,frames_ok,GetLastError());return 1;
    }
    // This executable buffer contains ONLY this manually defined synthetic LEA
    // and the owned bridge calls/RET. No approved-server instruction is run.
    using namespace SemanticAttribution::Native;
    Site own{};own.rva=0x100;own.expected_size=5;own.instruction_count=1;own.phase=CapturePhase::After;
    own.instructions[0]={{{0x48,0x8d,0x44,0x24,0x20}},5,RelocationKind::OriginalStack,4,1,2,0x20,0};
    auto* memory=VirtualAlloc(nullptr,RelayPlan::SlotSize,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if(!memory)return 1;
    const auto base=reinterpret_cast<std::uintptr_t>(memory);RelayPlan plan{};
    if(!BuildRelay(own,0x140000000ULL,base,reinterpret_cast<std::uintptr_t>(&SF_SemanticNativeThunk),plan))return 1;
    std::memcpy(memory,plan.bytes.data(),plan.bytes.size());DWORD previous{};
    if(!VirtualProtect(memory,RelayPlan::SlotSize,PAGE_EXECUTE_READ,&previous) ||
       !FlushInstructionCache(GetCurrentProcess(),memory,RelayPlan::SlotSize))return 1;
    RUNTIME_FUNCTION function{0,plan.code_size,static_cast<DWORD>(RelayPlan::UnwindOffset)};
    if(!RtlAddFunctionTable(&function,1,base))return 1;
    synthetic=true;captured=false;frames_ok=false;unwind_ok=false;expected_relay_return=base+plan.after_return;
    const auto actual=SF_SemanticOwnedInvokeRelay(memory,&expected_stack);
    if(actual!=expected_stack || !captured || !frames_ok || !unwind_ok)return 1;
    if(!RtlDeleteFunctionTable(&function) || !VirtualFree(memory,0,MEM_RELEASE))return 1;
    std::printf("SF-NA-30 owned thunk passed GPR/flags/XMM/YMM/LastError/dynamic-unwind/original-frame-chain; XCR0=%llx size=%u\n",support.enabled_mask,support.enabled_size);
    std::puts("SF-NA-30 OWN synthetic emitted relay passed original-RSP relocation and registered-leaf unwind; archived bytes never executed");
}
