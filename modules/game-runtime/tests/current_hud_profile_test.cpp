#include "map/current_client_hud_guards.h"
#include "fixtures/current5d_hud_bytes.h"
#include <vector>
#include <cstdio>
namespace {
std::vector<std::uint8_t> image(Current5dHudFixture::ImageSize);
unsigned checks{},failed{};
void check(bool value,const char* message){++checks;if(!value){++failed;std::printf("FAIL %s\n",message);}}
bool read(const std::uint8_t* from,void* to,std::size_t size){
 const auto at=reinterpret_cast<std::uintptr_t>(from),base=reinterpret_cast<std::uintptr_t>(image.data());
 if(at<base||at-base>image.size()||size>image.size()-(at-base))return false;
 std::memcpy(to,from,size);return true;
}
}
int main(){
 using namespace Current5dHudFixture;
 for(const auto& p:Rows)std::memcpy(image.data()+p.rva,p.bytes.data(),p.bytes.size());
 const auto match=[&](const std::uint8_t* renderer=nullptr,std::size_t rn=0,const std::uint8_t* call=nullptr,std::size_t cn=0){return MapRuntime::CurrentHud5d9ef2f1::Match(image.data(),image.size(),renderer,rn,call,cn,read);};
 check(match(),"current retained nine HUD guards admit");
 for(const auto& p:Rows){image[p.rva+p.bytes.size()-1]^=1;check(!match(),"each HUD guard mutation refuses");image[p.rva+p.bytes.size()-1]^=1;}
 const auto* renderer=reinterpret_cast<const std::uint8_t*>(Rows[0].bytes.data());
 const auto* call=reinterpret_cast<const std::uint8_t*>(Rows[7].bytes.data());
 image[Rows[0].rva]^=1;image[Rows[7].rva]^=1;
 check(!match(),"patched image needs original ownership prefixes");
 check(match(renderer,18,call,5),"owned original prefixes preserve adjacent guard verification");
 image[Rows[7].rva+5]^=1;check(!match(renderer,18,call,5),"retained first call cannot hide changed adjacent call");image[Rows[7].rva+5]^=1;
 check(!match(renderer,17,call,5)&&!match(renderer,18,call,4),"original prefix lengths stay exact");
 std::array<std::uint8_t,18> wrong{};std::memcpy(wrong.data(),renderer,18);wrong[17]^=1;
 check(!match(wrong.data(),18,call,5),"changed original renderer proof refuses");
 check(!MapRuntime::CurrentHud5d9ef2f1::Match(image.data(),Rows[8].rva+7,renderer,18,call,5,read),"truncated image refuses");
 std::printf("%u/%u current HUD profile checks\n",checks-failed,checks);return failed?1:0;
}
