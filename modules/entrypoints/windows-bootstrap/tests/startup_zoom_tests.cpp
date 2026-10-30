#include "zoom_fixture.h"
#include <iostream>
using namespace StartupZoom;
using namespace ZoomFixture;
int main() {
    unsigned cases=0;
    for (unsigned mutation=0;mutation<17;++mutation) {
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
        }
        std::vector<unsigned char> before(image,image+Profile::ImageSize); Protect(image);
        { Cap cap; assert(cap.TestApply(image,Profile::ImageSize,1,true,EngineHooks::ReplaceExecutableBytes)==Result::Unavailable); }
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
    std::cout << "PASS " << cases << " cap validation/ownership cases" << std::endl;
}
