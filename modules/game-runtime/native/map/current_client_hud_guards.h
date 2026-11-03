#pragma once
#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string_view>
// Exact selected current HUD dependencies, adapted from accepted557 profile2.
// No historical guard table is changed. Only retained native18/CALL5 overlay.
namespace MapRuntime::CurrentHud0403981f {
using Reader=bool(*)(const std::uint8_t*,void*,std::size_t);
struct Pin{std::size_t rva;std::string_view bytes;};
using namespace std::literals;
inline constexpr Pin Pins[]={
{0xC60310,"\x4c\x8b\xdc\x55\x49\x8d\xab\xe8\xfd\xff\xff\x48\x81\xec\x10\x03\x00\x00"sv},
{0xC60322,"\x48\x8b\x05\x7f\x01\xbe\x01\x48\x8b\x88\xd0\x00\x00\x00\x48\x8b\x01\x48\x85\xc0\x0f\x84\x94\x0e\x00\x00\x4d\x89\x73\xc8\x4c\x8b\x70\x38\x4c\x89\xb5\x30\x02\x00\x00\x4d\x85\xf6\x0f\x84\x74\x0e\x00\x00\x4d\x89\x63\xd8"sv},
{0xC60358,"\x4d\x8d\xa6\xd8\x1c\x00\x00\x41\x0f\xb6\x04\x24\x4c\x89\x65\x80\x3c\x33\x0f\x84\x50\x0e\x00\x00\x84\xc0\x0f\x84\x48\x0e\x00\x00"sv},
{0xBD1056,"\x48\x89\x05\x4b\xf4\xc6\x01"sv},
{0xBD1579,"\x4c\x89\x25\x28\xef\xc6\x01"sv},
{0xBD139B,"\x41\x0f\xb6\x47\x0c\xff\xc8\x83\xf8\x06\x0f\x87\xa4\x01\x00\x00\x48\x8d\x15\x4e\xec\x42\xff\x48\x98\x8b\x8c\x82\xf4\x15\xbd\x00\x48\x03\xca\xff\xe1"sv},
{0xBD15F4,"\xc0\x13\xbd\x00\xca\x13\xbd\x00\xd4\x13\xbd\x00\xde\x13\xbd\x00\x4b\x14\xbd\x00\xb8\x14\xbd\x00\x4a\x15\xbd\x00"sv},
{0xBD154A,"\xe8\xc1\xed\x08\x00\xe8\x2c\xb2\x0b\x00"sv},
{0xD80E70,"\x48\x89\x0d\x31\xf6\xab\x01\xc3"sv}
};
inline bool Match(const std::uint8_t* image,std::size_t size,const std::uint8_t* rendererPrefix,
    std::size_t rendererSize,const std::uint8_t* callPrefix,std::size_t callSize,Reader read) {
    if(!image || !read)return false;
    if(rendererPrefix || rendererSize){
        if(!rendererPrefix || rendererSize!=Pins[0].bytes.size() ||
           std::memcmp(rendererPrefix,Pins[0].bytes.data(),rendererSize))return false;
    }
    if(callPrefix || callSize){
        if(!callPrefix || callSize!=5 || std::memcmp(callPrefix,Pins[7].bytes.data(),5))return false;
    }
    for(const auto& pin:Pins){
        if(pin.rva>size || pin.bytes.size()>size-pin.rva || pin.bytes.size()>64)return false;
        if(rendererPrefix && pin.rva==0xc60310)continue;
        std::array<std::uint8_t,64> actual{};
        if(!read(image+pin.rva,actual.data(),pin.bytes.size()))return false;
        if(callPrefix && pin.rva==0xbd154a)std::memcpy(actual.data(),callPrefix,5);
        if(std::memcmp(actual.data(),pin.bytes.data(),pin.bytes.size()))return false;
    }return true;
}
}
