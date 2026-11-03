// SPDX-License-Identifier: MIT
#include "vein/ikea_client/profile.hpp"
#include "vein/ikea_client/native_input_profile.hpp"
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <bcrypt.h>
#include <cstring>
#include <limits>
namespace vein::ikea_client {
namespace {
template<class T> bool get(std::span<const std::byte> s,std::size_t at,T& value) noexcept {
 if(at>s.size()||sizeof(T)>s.size()-at)return false;
 std::memcpy(&value,s.data()+at,sizeof(T));return true;
}
bool sha(std::span<const std::byte> s,std::string_view expected) noexcept {
 if(expected.size()!=64||s.size()>(std::numeric_limits<ULONG>::max)())return false;
 BCRYPT_ALG_HANDLE algorithm=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;unsigned char digest[32]{};
 if(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0)return false;
 const auto created=BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0);
 const bool ok=created>=0&&BCryptHashData(hash,reinterpret_cast<PUCHAR>(const_cast<std::byte*>(s.data())),static_cast<ULONG>(s.size()),0)>=0&&BCryptFinishHash(hash,digest,32,0)>=0;
 if(hash)BCryptDestroyHash(hash);BCryptCloseAlgorithmProvider(algorithm,0);
 if(!ok)return false;
 constexpr char hex[]="0123456789abcdef";
 for(std::size_t i=0;i<32;++i)if(expected[i*2]!=hex[digest[i]>>4]||expected[i*2+1]!=hex[digest[i]&15])return false;
 return true;
}
}
bool validate_mapped_profile(std::span<const std::byte> image,std::span<const Guard> guards) noexcept {
 std::uint16_t mz=0,machine=0,sections=0,optional=0,magic=0;std::uint32_t nt=0,signature=0,size=0;
 if(!get(image,0,mz)||mz!=0x5a4d||!get(image,0x3c,nt)||nt<0x40||nt>0x1000||!get(image,nt,signature)||signature!=0x4550||!get(image,nt+4,machine)||machine!=0x8664||!get(image,nt+6,sections)||!sections||sections>96||!get(image,nt+20,optional)||optional<0xf0||optional>0x1000||!get(image,nt+24,magic)||magic!=0x20b||!get(image,nt+24+56,size)||size<0x1000||size>image.size())return false;
 const auto table=static_cast<std::size_t>(nt)+24+optional;
 if(table>0x1000||sections*40u>0x1000-table||table+sections*40u>image.size()||guards.empty())return false;
 for(const auto& guard:guards){
  if(!guard.size||guard.rva>size||guard.size>size-guard.rva)return false;
  bool executable=false;
  for(unsigned i=0;i<sections;++i){std::uint32_t va=0,virtual_size=0,flags=0;const auto entry=table+i*40;
   if(!get(image,entry+8,virtual_size)||!get(image,entry+12,va)||!get(image,entry+36,flags))return false;
   if(va<=guard.rva&&guard.rva-va<=virtual_size&&guard.size<=virtual_size-(guard.rva-va)&&(flags&0x20000000))executable=true;
  }
  if((guard.executable&&!executable)||!sha(image.subspan(guard.rva,guard.size),guard.sha256))return false;
 }
 return true;
}
bool validate_mapped_layout(std::span<const std::byte> image,std::uintptr_t load_base) noexcept {
 if(!validate_mapped_profile(image,input_layout_guards))return false;
 std::uint32_t nt=0,size=0;std::uint64_t preferred=0;
 if(!get(image,0x3c,nt)||!get(image,nt+24+56,size)||!get(image,nt+24+24,preferred))return false;
 const auto base=load_base?static_cast<std::uint64_t>(load_base):preferred;
 if(!base)return false;
 for(const auto& guard:input_layout_pointer_guards){
  std::uint64_t actual=0;
  if(guard.rva>size||sizeof(actual)>size-guard.rva||guard.target_rva>=size||
     guard.target_rva>(std::numeric_limits<std::uint64_t>::max)()-base||
     !get(image,guard.rva,actual)||actual!=base+guard.target_rva)return false;
 }
 return true;
}

}
