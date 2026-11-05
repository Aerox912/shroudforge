// Public image admission over retained bytes in an inert private allocation.
// No installed game image, original file, hook publication or game call is used.
#define WriteExecutableBytes ComponentFixtureWriteExecutableBytes
#define ReplaceExecutableBytes ComponentFixtureReplaceExecutableBytes
#include "map/capture_game_map.cpp"
#undef WriteExecutableBytes
#undef ReplaceExecutableBytes
#include "fixtures/current5d_map_bytes.h"
#include <cstdio>

namespace EngineHooks {
void* AllocateNearCode(std::uintptr_t,std::size_t){return nullptr;}
bool ComponentFixtureWriteExecutableBytes(std::uintptr_t,const void*,std::size_t,const void*){return false;}
bool ComponentFixtureReplaceExecutableBytes(std::uintptr_t,const void*,const void*,std::size_t){return false;}
}
namespace {
unsigned checks{},failed{};
void check(bool value,const char* message){++checks;if(!value){++failed;std::printf("FAIL %s\n",message);}}
}
int main(){
 using namespace Current5dMapFixture;
 std::vector<std::uint8_t> image(ImageSize);
 const auto base=reinterpret_cast<std::uintptr_t>(image.data());
 for(const auto& p:Spans)std::memcpy(image.data()+p.rva,p.bytes.data(),p.bytes.size());
 for(const auto& p:Pointers){const auto value=base+p.target;std::memcpy(image.data()+p.rva,&value,sizeof(value));}
 for(const auto& row:Names){std::memcpy(image.data()+row.nameRva,row.name,std::strlen(row.name)+1);const std::uintptr_t values[]{base+row.nameRva,std::strlen(row.name),base+row.callback};std::memcpy(image.data()+row.descriptor,values,sizeof(values));}
 IMAGE_DOS_HEADER dos{};dos.e_magic=IMAGE_DOS_SIGNATURE;dos.e_lfanew=0x140;std::memcpy(image.data(),&dos,sizeof(dos));
 IMAGE_NT_HEADERS64 nt{};nt.Signature=IMAGE_NT_SIGNATURE;nt.FileHeader.Machine=IMAGE_FILE_MACHINE_AMD64;nt.FileHeader.TimeDateStamp=Timestamp;nt.OptionalHeader.Magic=IMAGE_NT_OPTIONAL_HDR64_MAGIC;nt.OptionalHeader.SizeOfImage=ImageSize;std::memcpy(image.data()+dos.e_lfanew,&nt,sizeof(nt));
 const auto inspect=[&]{return MapRuntime::InspectMapImage(image,Sha,true);};
 auto admitted=inspect();check(admitted.status==SF_MAP_READY,"all nine current map guard groups admit retained current bytes");
 check(admitted.player&&admitted.renderer&&admitted.markers&&admitted.clock&&admitted.weather&&admitted.fog&&admitted.world&&admitted.deaths&&admitted.discovery,"all required capabilities remain checked");
 check(MapRuntime::InspectMapImage(image,Sha,false).status==SF_MAP_CAPABILITY_DENIED,"capability denial precedes admission");
 check(MapRuntime::InspectMapImage(image,"unknown",true).status!=SF_MAP_READY,"unknown executable identity refused");
 check(MapRuntime::InspectMapImage(std::span<const std::uint8_t>(image).first(image.size()-1),Sha,true).status!=SF_MAP_READY,"image extent drift refused");
 for(const auto& p:Spans){
   const auto at=p.rva+p.bytes.size()-1;image[at]^=1;
   check(inspect().status!=SF_MAP_READY,"each retained consumed span rejects a final-byte mutation");image[at]^=1;
 }
 for(const auto& p:Pointers){image[p.rva]^=1;check(inspect().status!=SF_MAP_READY,"each relocated pointer rejects target drift");image[p.rva]^=1;}
 for(const auto& row:Names){image[row.descriptor+16]^=1;check(inspect().status!=SF_MAP_READY,"named callback binding rejects target drift");image[row.descriptor+16]^=1;}
 image[dos.e_lfanew+8]^=1;check(inspect().status!=SF_MAP_READY,"PE timestamp drift refused");image[dos.e_lfanew+8]^=1;
 check(inspect().status==SF_MAP_READY,"restored fixture recovers exact admission");
 MEMORY_BASIC_INFORMATION metadata{};check(VirtualQuery(image.data(),&metadata,sizeof(metadata))==sizeof(metadata)&&!(metadata.Protect&(PAGE_EXECUTE|PAGE_EXECUTE_READ|PAGE_EXECUTE_READWRITE|PAGE_EXECUTE_WRITECOPY)),"fixture remains physically non-executable");
 std::printf("%u/%u current map profile checks\n",checks-failed,checks);return failed?1:0;
}
