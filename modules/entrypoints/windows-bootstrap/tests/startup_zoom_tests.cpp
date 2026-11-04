#include "zoom_fixture.h"
#include <iostream>
using namespace StartupZoom;
using namespace ZoomFixture;
int main(int argc, char** argv) {
    if (argc==2 && !std::strcmp(argv[1],"--independent-admission")) {
        auto* image=Create(); Protect(image);
        Result result;
        { Cap cap; result=cap.TestApply(image,ObservedZoomFixture::ImageSize,1,true,EngineHooks::ReplaceExecutableBytes);
          if(result==Result::Applied)cap.Commit(); }
        VirtualFree(image,0,MEM_RELEASE);
        std::cout << (result==Result::Applied?"PASS":"FAIL") << " independent observed current fixture admission" << std::endl;
        return result==Result::Applied?0:1;
    }
    unsigned cases=0;
    assert(Profile::ImageSize==0x02A3F000u);
    assert(Profile::Timestamp==0x6AE9B2ACu);
    const std::uint32_t expectedSites[]={0x00C2E3A0u,0x00D54400u,0x00D52150u,0x00D952F6u};
    const std::uint32_t expectedBegins[]={0x00C2E2E4u,0x00D54400u,0x00D51F3Fu,0x00D95022u};
    const std::uint32_t expectedEnds[]={0x00C2ED87u,0x00D5455Du,0x00D5221Eu,0x00D9546Fu};
    const std::uint32_t expectedUnwinds[]={0x01F40480u,0x01F41048u,0x01F4125Cu,0x01F41138u};
    for (unsigned i=0;i<4;++i) {
        assert(Profile::Signatures[i].rva==expectedSites[i]);
        assert(Profile::Signatures[i].begin==expectedBegins[i]);
        assert(Profile::Signatures[i].end==expectedEnds[i]);
        assert(Profile::Signatures[i].unwind==expectedUnwinds[i]);
        assert(Profile::Signatures[i].length==64);
        assert(!std::memcmp(Profile::Signatures[i].bytes.data(),ObservedZoomFixture::Signatures[i].bytes,64));
    }
    {
        auto* image=Create(); Protect(image);
        const auto site=Profile::Signatures[0].rva;
        assert(!std::memcmp(image+site,Profile::Original,4));
        {
            Cap cap;
            assert(cap.TestApply(image,Profile::ImageSize,1,true,EngineHooks::ReplaceExecutableBytes)==Result::Applied);
            assert(!std::memcmp(image+site,Profile::Replacement,4));
            assert(image[site+3]==0x05);
            cap.Commit();
        }
        VirtualFree(image,0,MEM_RELEASE); ++cases;
    }
    for (unsigned mutation=0;mutation<20;++mutation) {
        auto* image=Create(); auto* nt=reinterpret_cast<IMAGE_NT_HEADERS64*>(image+0x100);
        auto* fn=reinterpret_cast<RUNTIME_FUNCTION*>(image+0x1000);
        switch(mutation) {
        case 0: image[0]=0;break;
        case 1: reinterpret_cast<IMAGE_DOS_HEADER*>(image)->e_lfanew=0x7fffffff;break;
        case 2: nt->FileHeader.TimeDateStamp++;break;
        case 3: nt->FileHeader.Machine=IMAGE_FILE_MACHINE_I386;break;
        case 4: nt->OptionalHeader.SizeOfImage--;break;
        case 5: nt->FileHeader.SizeOfOptionalHeader--;break;
        case 6: nt->OptionalHeader.DataDirectory[3].VirtualAddress=Profile::ImageSize-1;break;
        case 7: nt->OptionalHeader.DataDirectory[3].Size--;break;
        case 8: IMAGE_FIRST_SECTION(nt)->Misc.VirtualSize=Profile::ImageSize;break;
        case 9: IMAGE_FIRST_SECTION(nt)->Characteristics=IMAGE_SCN_MEM_READ;break;
        case 10: fn[0].BeginAddress++;break;
        case 11: fn[0].UnwindData++;break;
        case 12: fn[1]=fn[0];break;
        case 13: std::memcpy(image+0x3000,Profile::Signatures[0].bytes.data(),64);break;
        case 14: image[Profile::Signatures[0].rva+27]=4;break;
        case 15: image[Profile::Signatures[2].rva+10]^=1;break;
        case 16: image[Profile::Signatures[3].rva+10]^=1;break;
        case 17: image[Profile::Signatures[1].rva+10]^=1;break;
        case 18: nt->FileHeader.TimeDateStamp=1792507660;break; // prior d64966e2 profile identity
        case 19: nt->FileHeader.TimeDateStamp=1793351710;nt->OptionalHeader.SizeOfImage=44302336;break; // prior0403 identity
        }
        std::vector<unsigned char> before(image,image+Profile::ImageSize); Protect(image);
        { Cap cap; assert(cap.TestApply(image,Profile::ImageSize,1,true,EngineHooks::ReplaceExecutableBytes)==Result::Unavailable); }
        assert(!std::memcmp(before.data(),image,before.size())); VirtualFree(image,0,MEM_RELEASE); ++cases;
    }
    for(unsigned i=0;i<4;++i) {
        auto* image=Create(); image[ObservedZoomFixture::Signatures[i].rva+63]^=1;
        std::vector<unsigned char> before(image,image+ObservedZoomFixture::ImageSize); Protect(image);
        { Cap cap; assert(cap.TestApply(image,ObservedZoomFixture::ImageSize,1,true,EngineHooks::ReplaceExecutableBytes)==Result::Unavailable); }
        assert(!std::memcmp(before.data(),image,before.size())); VirtualFree(image,0,MEM_RELEASE); ++cases;
    }
    {
        constexpr unsigned oldImageSize=44163072;
        auto* image=static_cast<unsigned char*>(VirtualAlloc(nullptr,oldImageSize,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));
        assert(image);
        auto* dos=reinterpret_cast<IMAGE_DOS_HEADER*>(image); dos->e_magic=IMAGE_DOS_SIGNATURE; dos->e_lfanew=0x100;
        auto* nt=reinterpret_cast<IMAGE_NT_HEADERS64*>(image+0x100); nt->Signature=IMAGE_NT_SIGNATURE;
        nt->FileHeader.Machine=IMAGE_FILE_MACHINE_AMD64; nt->FileHeader.TimeDateStamp=1792507660;
        nt->FileHeader.SizeOfOptionalHeader=sizeof(IMAGE_OPTIONAL_HEADER64);
        nt->OptionalHeader.Magic=IMAGE_NT_OPTIONAL_HDR64_MAGIC; nt->OptionalHeader.SizeOfImage=oldImageSize;
        std::vector<unsigned char> before(image,image+4096);
        { Cap cap; assert(cap.TestApply(image,oldImageSize,1,true,EngineHooks::ReplaceExecutableBytes)==Result::Unavailable); }
        assert(!std::memcmp(before.data(),image,before.size())); VirtualFree(image,0,MEM_RELEASE); ++cases;
    }
    for (auto pair : {std::pair{0u,true},std::pair{2u,true},std::pair{1u,false}}) {
        Cap cap; assert(cap.TestApply(nullptr,0,pair.first,pair.second,EngineHooks::ReplaceExecutableBytes)==
            (pair.first==0?Result::Disabled:Result::Unavailable)); ++cases;
    }
    for (unsigned mode=0;mode<7;++mode) {
        auto* image=Create(); std::vector<unsigned char> before(image,image+Profile::ImageSize); Protect(image);
        const auto target=reinterpret_cast<std::uintptr_t>(image+Profile::Signatures[0].rva);
        {
            Cap cap; auto writer=EngineHooks::ReplaceExecutableBytes;
            if(mode==1)writer=NoWrite; if(mode==2)writer=Partial; if(mode==3)writer=FullButFalse; if(mode==4)writer=BadProtection;
            const auto result=cap.TestApply(image,Profile::ImageSize,1,true,writer);
            if(mode==1)assert(result==Result::Unavailable);
            else if(mode>=2 && mode<=4) {
                assert(result==Result::Unsafe);
                assert(cap.Rollback()==(mode==3));
            } else {
                assert(result==Result::Applied);
                before[Profile::Signatures[0].rva+3]=5;
                assert(!std::memcmp(before.data(),image,before.size())); // only one immediate differs
                assert(cap.TestApply(image,Profile::ImageSize,1,true,writer)==Result::Unavailable);
                if(mode==5) { const unsigned char conflict=6; Put(target+3,&conflict,1); assert(!cap.Rollback()); }
                else if(mode==6)cap.Commit();
            }
        }
        if(mode==0 || mode==1 || mode==3) assert(!std::memcmp(reinterpret_cast<void*>(target),Profile::Original,4));
        if(mode==5)assert(image[Profile::Signatures[0].rva+3]==6);
        if(mode==6)assert(!std::memcmp(reinterpret_cast<void*>(target),Profile::Replacement,4));
        VirtualFree(image,0,MEM_RELEASE); ++cases;
    }
    std::cout << "PASS current ce6f independent observed profile 4->5; " << cases << " cap validation/ownership cases" << std::endl;
}
