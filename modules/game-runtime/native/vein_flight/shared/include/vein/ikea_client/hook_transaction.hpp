// SPDX-License-Identifier: MIT
#pragma once
#include <array>
#include <cstddef>
namespace vein::ikea_client {
struct HookEntry {void* target=nullptr;void* detour=nullptr;void* original=nullptr;bool created=false;bool attempted=false;};
struct HookOperations {
 void* context;
 bool (*create)(void*,HookEntry&) noexcept;
 bool (*enable)(void*,HookEntry&) noexcept;
 bool (*disable)(void*,HookEntry&) noexcept;
 void (*close_admission)(void*) noexcept;
};
// One owner for the packer and seven native whole-function seams. Originals and
// module stay resident after EVERY failure/cancellation; never remove/free a
// trampoline whose callbacks might still be active. No MH_ALL_HOOKS operations.
class HookTransaction {
 std::array<HookEntry,8> entries_{};
 std::size_t count_=0;
 bool uncertain_=false;
public:
 [[nodiscard]] bool add(void* target,void* detour) noexcept {
  if(count_==entries_.size()||!target||!detour)return false;
  entries_[count_++]={target,detour};return true;
 }
 [[nodiscard]] HookEntry& entry(std::size_t i) noexcept {return entries_[i];}
 [[nodiscard]] bool uncertain() const noexcept {return uncertain_;}
 [[nodiscard]] bool create(const HookOperations& ops) noexcept {
  for(std::size_t i=0;i<count_;++i){if(!ops.create(ops.context,entries_[i])){ops.close_admission(ops.context);return false;}entries_[i].created=true;}
  return true;
 }
 [[nodiscard]] bool cancel(const HookOperations& ops) noexcept {
  ops.close_admission(ops.context);bool ok=true;
  for(std::size_t i=count_;i;--i){auto& e=entries_[i-1];if(e.attempted){if(ops.disable(ops.context,e))e.attempted=false;else ok=false;}}
  uncertain_=uncertain_||!ok;return ok;
 }
 [[nodiscard]] bool activate(const HookOperations& ops) noexcept {
  for(std::size_t i=0;i<count_;++i){auto& e=entries_[i];if(!e.created){ops.close_admission(ops.context);return false;}e.attempted=true;if(!ops.enable(ops.context,e)){(void)cancel(ops);return false;}}
  return true;
 }
};
}
