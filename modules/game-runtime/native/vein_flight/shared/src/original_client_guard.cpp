// SPDX-License-Identifier: MIT
#include "vein/ikea_client/original_client_guard.hpp"
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
namespace vein::ikea_client {
bool canonical_original_client_absent(const std::filesystem::path& directory) noexcept {
 try{
  if(!directory.is_absolute())return false;
  const auto dll=directory/L"mods"/L"XHL-Vein-Mining"/L"XHL-Vein-Mining.dll";
  // Shroudtopia LoadLibrary precedes export rejection and ignores this mod's
  // native-plugin.ini. Presence itself is a conflict, regardless Enabled0/1.
  const auto attributes=GetFileAttributesW(dll.c_str());
  if(attributes!=INVALID_FILE_ATTRIBUTES)return false;
  const auto error=GetLastError();
  return error==ERROR_FILE_NOT_FOUND||error==ERROR_PATH_NOT_FOUND;
 }catch(...){return false;}
}
}
