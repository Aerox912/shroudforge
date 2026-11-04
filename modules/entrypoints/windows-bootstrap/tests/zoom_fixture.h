#pragma once
#include "../startup_zoom.h"
#include "../startup_zoom_profile.h"
#include "startup_zoom_observed_fixture.h"
#include "engine-hooks/write_executable_bytes.h"
#include <cassert>
#include <cstring>
#include <vector>
namespace ZoomFixture {
inline unsigned char* Create() {
    using namespace StartupZoom;
    auto* image = static_cast<unsigned char*>(VirtualAlloc(nullptr, ObservedZoomFixture::ImageSize, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE));
    assert(image);
    auto* dos = reinterpret_cast<IMAGE_DOS_HEADER*>(image); dos->e_magic=IMAGE_DOS_SIGNATURE; dos->e_lfanew=0x100;
    auto* nt = reinterpret_cast<IMAGE_NT_HEADERS64*>(image+0x100); nt->Signature=IMAGE_NT_SIGNATURE;
    nt->FileHeader.Machine=IMAGE_FILE_MACHINE_AMD64; nt->FileHeader.TimeDateStamp=ObservedZoomFixture::Timestamp;
    nt->FileHeader.SizeOfOptionalHeader=sizeof(IMAGE_OPTIONAL_HEADER64); nt->FileHeader.NumberOfSections=1;
    nt->OptionalHeader.Magic=IMAGE_NT_OPTIONAL_HDR64_MAGIC; nt->OptionalHeader.SizeOfImage=ObservedZoomFixture::ImageSize;
    nt->OptionalHeader.NumberOfRvaAndSizes=IMAGE_NUMBEROF_DIRECTORY_ENTRIES;
    nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION]={0x1000,4*sizeof(RUNTIME_FUNCTION)};
    auto* sec=IMAGE_FIRST_SECTION(nt); sec->VirtualAddress=0x2000; sec->Misc.VirtualSize=ObservedZoomFixture::ImageSize-0x2000;
    sec->Characteristics=IMAGE_SCN_MEM_EXECUTE|IMAGE_SCN_MEM_READ;
    auto* f=reinterpret_cast<RUNTIME_FUNCTION*>(image+0x1000);
    for (unsigned i=0;i<4;++i) { const auto& s=ObservedZoomFixture::Signatures[i];
        std::memcpy(image+s.rva,s.bytes,s.length); f[i]={s.begin,s.end,s.unwind}; }
    return image;
}
inline void Protect(unsigned char* image) { DWORD old{}; assert(VirtualProtect(image,ObservedZoomFixture::ImageSize,PAGE_EXECUTE_READ,&old)); }
inline void Put(std::uintptr_t address,const void* bytes,std::size_t size) {
    DWORD old{},ignored{}; assert(VirtualProtect(reinterpret_cast<void*>(address),size,PAGE_EXECUTE_READWRITE,&old));
    std::memcpy(reinterpret_cast<void*>(address),bytes,size);
    assert(VirtualProtect(reinterpret_cast<void*>(address),size,old,&ignored));
    assert(FlushInstructionCache(GetCurrentProcess(),reinterpret_cast<void*>(address),size));
}
inline bool Partial(std::uintptr_t address,const void*,const void*,std::size_t) { const unsigned char bad=0x90; Put(address,&bad,1); return false; }
inline bool NoWrite(std::uintptr_t,const void*,const void*,std::size_t) { return false; }
inline bool FullButFalse(std::uintptr_t a,const void* e,const void* r,std::size_t n) {
    if (!EngineHooks::ReplaceExecutableBytes(a,e,r,n)) return false;
    return std::memcmp(r,StartupZoom::Profile::Original,4)==0;
}
inline bool BadProtection(std::uintptr_t a,const void*,const void* r,std::size_t n) {
    DWORD old{}; assert(VirtualProtect(reinterpret_cast<void*>(a),n,PAGE_EXECUTE_READWRITE,&old));
    std::memcpy(reinterpret_cast<void*>(a),r,n); return true;
}
}
