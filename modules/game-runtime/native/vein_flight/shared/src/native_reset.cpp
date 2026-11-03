// SPDX-License-Identifier: MIT
#include "vein/ikea_client/native_reset.hpp"
#include <limits>
namespace vein::ikea_client {
NativeReset::NativeReset(FlightMux& mux,std::uint64_t initial,FlightAuthority authority) noexcept: mux_(mux),authority_(authority),serial_(initial?initial:1) {
 exhausted_=serial_==(std::numeric_limits<std::uint64_t>::max)();
}
std::uint64_t NativeReset::observe_input(std::uintptr_t context,std::uint32_t channel,std::uint64_t now) noexcept {
 if(authority_!=FlightAuthority::current_input||!available()||hosts_||retires_||!context||!channel)return 0;
 if(!input_current(context,channel,serial_,now)){
  reset();if(!available())return 0;
  input_context_=context;input_channel_=channel;
 }
 input_seen_=now;return serial_;
}
bool NativeReset::input_current(std::uintptr_t context,std::uint32_t channel,std::uint64_t serial,std::uint64_t now) const noexcept {
 return authority_==FlightAuthority::current_input&&available()&&!hosts_&&!retires_&&
  context&&channel&&context==input_context_&&channel==input_channel_&&serial==serial_&&
  now>=input_seen_&&now-input_seen_<=1000;
}
void NativeReset::reset() noexcept {
 mux_.native_reset();
 input_context_=0;input_channel_=0;input_seen_=0;
 if(serial_!=(std::numeric_limits<std::uint64_t>::max)())++serial_;
 if(serial_==(std::numeric_limits<std::uint64_t>::max)())exhausted_=true;
}
bool NativeReset::open() noexcept {
 if(attempted_)return false;
 attempted_=true;active_=true;return true;
}
void NativeReset::close() noexcept {active_=false;selected_manager_=0;selected_key_=0;reset();}
std::uint64_t NativeReset::begin_host(std::uintptr_t manager) noexcept {
 if(!active_)return 0;
 reset();selected_manager_=0;selected_key_=0;host_manager_=manager;
 if(!manager||hosts_==(std::numeric_limits<std::uint32_t>::max)()){close();return 0;}
 ++hosts_;return serial_;
}
HostFinishDisposition NativeReset::finish_host(std::uint64_t ticket,bool success) noexcept {
 if(!ticket)return HostFinishDisposition::no_ticket;
 if(hosts_)--hosts_;
 if(success)return HostFinishDisposition::success;
 // Preserve the existing short-circuit: failed finishes alone compare the ticket.
 if(serial_==ticket){selected_manager_=0;selected_key_=0;reset();return HostFinishDisposition::failed_current;}
 return HostFinishDisposition::failed_stale;
}
bool NativeReset::select(std::uint64_t ticket,std::uintptr_t manager,std::uint16_t key) noexcept {
 // The adapter additionally requires exact host/add TLS, caller PC, owner to
 // manager forward address and role1 before this owned-value transition.
 if(!available()||!hosts_||retires_||!ticket||ticket!=serial_||!manager||manager!=host_manager_||!key)return false;
 if(selected_manager_&&(selected_manager_!=manager||selected_key_!=key))return false;
 selected_manager_=manager;selected_key_=key;return true;
}
bool NativeReset::refresh(std::uintptr_t manager,std::uint16_t key) noexcept {
 if(!active_||!key||!selected_manager_||selected_manager_!=manager||selected_key_!=key)return false;
 reset();return true;
}
bool NativeReset::begin_retire(std::uintptr_t manager,std::uint16_t key,bool roster) noexcept {
 if(!active_)return false;
 if(authority_==FlightAuthority::current_input){
  // Conservatively close input admission for a native retirement. Do not
  // reinterpret the old WORD peer key as the input channel's DWORD identity.
  if(!manager||(!input_context_&&!selected_manager_))return false;
 }else if(!selected_manager_||selected_manager_!=manager||(!roster&&selected_key_!=key))return false;
 reset();selected_manager_=0;selected_key_=0;
 if(retires_==(std::numeric_limits<std::uint32_t>::max)()){close();return false;}
 ++retires_;return true;
}
void NativeReset::finish_retire(bool matched) noexcept {
 if(!matched)return;
 if(retires_)--retires_;
 // A host/publication started during original cleanup cannot survive its end.
 selected_manager_=0;selected_key_=0;reset();
}
FlightPublishDecision NativeReset::publish_decision(const EsmVeinFlightIntentV2& p,std::uint64_t now) noexcept {
 if(!active_)return FlightPublishDecision::inactive;
 if(p.struct_bytes!=32||p.abi_version!=2||!p.publisher_epoch||p.requested_on>1||p.reserved0)return FlightPublishDecision::invalid;
 if(!p.observed_reset_serial||p.observed_reset_serial!=serial_)return FlightPublishDecision::stale_serial;
 if(p.requested_on){
  if(!available())return FlightPublishDecision::inactive;
  if(hosts_||retires_)return FlightPublishDecision::lifecycle_transition;
  if(authority_==FlightAuthority::current_input){
   if(!input_current(input_context_,input_channel_,serial_,now))return FlightPublishDecision::missing_input;
  }else if(!selected_manager_||!selected_key_)return FlightPublishDecision::missing_association;
 }
 return mux_.publish({24,1,p.publisher_epoch,p.requested_on,0},now)?FlightPublishDecision::accepted:FlightPublishDecision::mux_rejected;
}
bool NativeReset::publish(const EsmVeinFlightIntentV2& p,std::uint64_t now) noexcept {
 return publish_decision(p,now)==FlightPublishDecision::accepted;
}
bool NativeReset::publish_v1(const EsmVeinFlightIntentV1& p,std::uint64_t now) noexcept {
 return active_&&p.requested_on==0&&mux_.publish(p,now);
}
}
