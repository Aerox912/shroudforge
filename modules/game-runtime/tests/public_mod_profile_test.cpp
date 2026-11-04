// Reads original publisher DLLs as files only; never loads or executes them.
#include "map/public_creative_compat.h"
#include <filesystem>
#include <fstream>
#include <iostream>
#include <vector>
#include <cstdlib>

namespace {
unsigned checks{};
void check(bool ok,const char* message) {
    ++checks;
    if(!ok){std::cerr<<message<<'\n';std::exit(1);}
}
void image(const char* path,const PublicCreativeCompat::Profile& profile,bool creative) {
    using namespace PublicCreativeCompat;
    std::ifstream file(std::filesystem::path(path),std::ios::binary);
    check(bool(file),"publisher input unavailable");
    std::vector<unsigned char> bytes{std::istreambuf_iterator<char>(file),{}};
    check(bytes.size()==profile.fileSize,"file size differs");
    check(HasHash(bytes.data(),bytes.size(),profile.fileHash),"whole-file identity differs");
    IMAGE_DOS_HEADER dos{};check(bytes.size()>=sizeof(dos),"short DOS header");
    std::memcpy(&dos,bytes.data(),sizeof(dos));
    check(dos.e_magic==IMAGE_DOS_SIGNATURE && dos.e_lfanew>0,"DOS header differs");
    const auto pe=static_cast<std::size_t>(dos.e_lfanew);
    check(pe<=bytes.size() && sizeof(IMAGE_NT_HEADERS64)<=bytes.size()-pe,"short PE header");
    IMAGE_NT_HEADERS64 nt{};std::memcpy(&nt,bytes.data()+pe,sizeof(nt));
    check(nt.Signature==IMAGE_NT_SIGNATURE && nt.FileHeader.Machine==IMAGE_FILE_MACHINE_AMD64 &&
        nt.OptionalHeader.Magic==IMAGE_NT_OPTIONAL_HDR64_MAGIC && nt.OptionalHeader.SizeOfImage==profile.imageSize,"PE geometry differs");
    const auto table=pe+4+sizeof(IMAGE_FILE_HEADER)+nt.FileHeader.SizeOfOptionalHeader;
    check(table<=bytes.size() && nt.FileHeader.NumberOfSections<96 &&
        nt.FileHeader.NumberOfSections*sizeof(IMAGE_SECTION_HEADER)<=bytes.size()-table,"section table differs");
    const auto span=[&](std::uintptr_t rva,std::size_t count)->const unsigned char* {
        const unsigned char* found=nullptr;
        for(unsigned i=0;i<nt.FileHeader.NumberOfSections;++i){
            IMAGE_SECTION_HEADER section{};std::memcpy(&section,bytes.data()+table+i*sizeof(section),sizeof(section));
            if(rva<section.VirtualAddress || rva-section.VirtualAddress>section.SizeOfRawData ||
                count>section.SizeOfRawData-(rva-section.VirtualAddress))continue;
            const auto offset=std::size_t(section.PointerToRawData)+(rva-section.VirtualAddress);
            check(offset<=bytes.size() && count<=bytes.size()-offset,"section span escapes file");
            check(!found,"ambiguous RVA");found=bytes.data()+offset;
        }
        check(found!=nullptr,"unmapped profile span");return found;
    };
    check(HasHash(span(profile.body,profile.bodySize),profile.bodySize,profile.bodyHash),"callback body differs");
    if(creative){
        check(HasHash(span(profile.pattern,BlockSize),BlockSize,TemplateHash),"template differs");
        check(!std::memcmp(span(profile.callback,5),profile.thunk.data(),5),"callback thunk differs");
        check(SelectCreativeProfile(0x100000,0x100000+profile.callback,nt.FileHeader.TimeDateStamp)==&profile,"timestamp selected wrong Creative identity");
    }
    bytes.back()^=1;
    check(!HasHash(bytes.data(),bytes.size(),profile.fileHash),"changed publisher file accepted");
}
}
int main(int argc,char** argv) {
    using namespace PublicCreativeCompat;
    check(argc==4,"expected Creative312, Find128, Teleport1110 file paths");
    image(argv[1],Creative312,true);image(argv[2],Find128,false);image(argv[3],Teleport1110,false);
    check(SelectCreativeProfile(0x100000,0x104e67,0)==&Creative311,"legacy Creative identity lost");
    check(SelectCreativeProfile(0x100000,0x104e67,Creative312Timestamp)==&Creative312,"latest Creative identity lost");
    check(SelectCreativeProfile(0x100000,0x104e68,Creative312Timestamp)==nullptr,"unknown callback accepted");
    check(SelectCreativeProfile(0x100000,0xfffff,Creative312Timestamp)==nullptr,"callback underflow accepted");
    check(FiniteGraph::Observation::New(&Find128) && FiniteGraph::Observation::New(&Teleport1110),"latest graph family absent");
    check(FiniteGraph::Observation::New(&Find125) && FiniteGraph::Observation::New(&Teleport117),"previous graph family lost");
    std::cout<<checks<<" original-file/profile selection checks passed; no DLL execution\n";
}
