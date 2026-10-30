#include "zoom_fixture.h"
#include <fstream>
#include <iterator>
#include <iostream>
int main(int argc,char** argv) {
    assert(argc==2); std::ifstream input(argv[1],std::ios::binary); assert(input);
    std::vector<unsigned char> raw((std::istreambuf_iterator<char>(input)),{});
    assert(raw.size()>0x1000);
    const auto* dos=reinterpret_cast<const IMAGE_DOS_HEADER*>(raw.data());
    assert(dos->e_magic==IMAGE_DOS_SIGNATURE && dos->e_lfanew>0 && dos->e_lfanew<0x1000);
    const auto* nt=reinterpret_cast<const IMAGE_NT_HEADERS64*>(raw.data()+dos->e_lfanew);
    assert(nt->OptionalHeader.SizeOfImage==StartupZoom::Profile::ImageSize);
    auto* image=static_cast<unsigned char*>(VirtualAlloc(nullptr,StartupZoom::Profile::ImageSize,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));assert(image);
    assert(nt->OptionalHeader.SizeOfHeaders<raw.size());std::memcpy(image,raw.data(),nt->OptionalHeader.SizeOfHeaders);
    const auto* s=IMAGE_FIRST_SECTION(nt);
    for(unsigned i=0;i<nt->FileHeader.NumberOfSections;++i) {
        assert(std::uint64_t(s[i].PointerToRawData)+s[i].SizeOfRawData<=raw.size());
        assert(std::uint64_t(s[i].VirtualAddress)+s[i].SizeOfRawData<=StartupZoom::Profile::ImageSize);
        std::memcpy(image+s[i].VirtualAddress,raw.data()+s[i].PointerToRawData,s[i].SizeOfRawData);
    }
    std::vector<unsigned char> before(image,image+StartupZoom::Profile::ImageSize);ZoomFixture::Protect(image);
    {
        StartupZoom::Cap cap;
        auto result=cap.TestApply(image,StartupZoom::Profile::ImageSize,1,true,EngineHooks::ReplaceExecutableBytes);
        assert(result==StartupZoom::Result::Applied);
        before[StartupZoom::Profile::Signatures[0].rva+3]=5;
        assert(!std::memcmp(image,before.data(),before.size()));assert(cap.Rollback());
        before[StartupZoom::Profile::Signatures[0].rva+3]=4;
        assert(!std::memcmp(image,before.data(),before.size()));
    }
    VirtualFree(image,0,MEM_RELEASE);
    std::cout << "PASS retained image map: unique signatures, pdata associations, exact one-byte immediate delta, exact rollback; game code never executed" << std::endl;
}
