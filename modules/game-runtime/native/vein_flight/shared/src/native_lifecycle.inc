// SPDX-License-Identifier: MIT
// Included inside native_adapter.cpp's private namespace, sharing ONE gate/mux.
// All pointers below are opaque owned values. No cached native pointer is read.
struct HostScope {HostScope* prior;std::uintptr_t manager;std::uint64_t ticket;bool vein_transition=false;};
struct AddScope {AddScope* prior;HostScope* host;void* manager;bool qualified;};
thread_local HostScope* current_host_scope=nullptr;
thread_local AddScope* current_add_scope=nullptr;
bool begin_vein_lifecycle_transition(Runtime& r) noexcept {
 if(!r.vein_policy_allowed)return false;
 r.vein_input_session.begin_transition();
 reset_vein_consumer(r,r.epoch);
 return true;
}
void finish_vein_lifecycle_transition(Runtime& r,bool started) noexcept {
 if(!started)return;
 r.vein_input_session.end_transition();
 reset_vein_consumer(r,r.epoch);
}
void close_native_admission(void*) noexcept {
 auto& r=state();
 // HookTransaction may call this while Load owns the gate; closing admission
 // blocks commits and the next gated activation retires the Vein ticket.
 r.ready=false;r.enabled=false;
 reset_vein_consumer(r,r.epoch);
 if(r.flight_transport_enabled){
  r.native_reset.close();
 }
}
void finish_host_scope(HostScope* scope,bool success) noexcept {
 const DWORD error=GetLastError();current_host_scope=scope->prior;
 try{auto& r=state();std::lock_guard lock(r.gate);
  if(r.flight_transport_enabled)(void)r.native_reset.finish_host(scope->ticket,success);
  finish_vein_lifecycle_transition(r,scope->vein_transition);
 }catch(...){fault_worker();}
 SetLastError(error);
}
// Raw SEH finally leaves: no C++ object/destructor, no catch around stock.
// Both normal and abnormal native unwind restore the owned TLS stack. The
// original exception remains visible; native failures are never swallowed.
std::uint64_t invoke_host(HostScope* scope,void* owner,std::uint8_t attempt) {
 std::uint64_t result=0;
 __try{result=state().host_original(owner,attempt);}
 __finally{const DWORD error=GetLastError();const bool abnormal=AbnormalTermination()!=FALSE;SetLastError(error);finish_host_scope(scope,!abnormal&&(result&0xff)!=0);}
 return result;
}
__declspec(noinline) std::uint64_t __fastcall native_host(void* owner,std::uint8_t attempt) {
 const DWORD error=GetLastError();
 auto& runtime=state();
 // Pinned109DBBA tests DL: zero jumps109DD5E and only queries status.
 // It must not reset intent/selection or introduce a new owned host scope.
 if(!attempt){const auto original=runtime.host_original;SetLastError(error);return original(owner,attempt);}
 HostScope scope{current_host_scope,0,0};
 const auto address=reinterpret_cast<std::uintptr_t>(owner);
 if(address&&address<=UINTPTR_MAX-0x348fc0)scope.manager=address+0x348fc0;
 try{auto& r=state();std::lock_guard lock(r.gate);
  scope.vein_transition=begin_vein_lifecycle_transition(r);
  if(r.flight_transport_enabled)scope.ticket=r.native_reset.begin_host(scope.manager);}
 catch(...){fault_worker();}
 current_host_scope=&scope;SetLastError(error);return invoke_host(&scope,owner,attempt);
}
void finish_add_scope(AddScope* scope) noexcept {current_add_scope=scope->prior;}
std::uint64_t invoke_add(AddScope* scope,void* manager,const void* identity,std::uint8_t role) {
 __try{return state().add_original(manager,identity,role);}
 __finally{finish_add_scope(scope);}
}
__declspec(noinline) std::uint64_t __fastcall native_add(void* manager,const void* identity,std::uint8_t role) {
 const auto caller=reinterpret_cast<std::uintptr_t>(_ReturnAddress());
 auto* host=current_host_scope;
 AddScope scope{current_add_scope,host,manager,host&&host->ticket&&host->manager==reinterpret_cast<std::uintptr_t>(manager)&&role==1&&caller==state().game_base+host_add_caller_rva};
 current_add_scope=&scope;return invoke_add(&scope,manager,identity,role);
}
__declspec(noinline) void* __fastcall native_lookup(void* manager,std::uint16_t key) {
 const auto caller=reinterpret_cast<std::uintptr_t>(_ReturnAddress());
 // Exact standard RCX/DX inputs; no implicit nonvolatile inputs in pinned body.
 // The original validates the ENTIRE WORD and supplies the actual current entry.
 auto* result=state().lookup_original(manager,key);const DWORD error=GetLastError();
 if(result){
  try{auto& r=state();std::lock_guard lock(r.gate);
   if(r.flight_transport_enabled){
    if(caller==r.game_base+add_lookup_caller_rva){
     auto* add=current_add_scope;
     if(add&&add->qualified&&add->manager==manager&&add->host==current_host_scope){
      (void)r.native_reset.select(add->host->ticket,reinterpret_cast<std::uintptr_t>(manager),key);
     }
    }else if(caller==r.game_base+refresh_lookup_caller_rva){
     (void)r.native_reset.refresh(reinterpret_cast<std::uintptr_t>(manager),key);
    }
   }
  }catch(...){fault_worker();}
 }
 SetLastError(error);return result;
}
struct RetireScope {bool matched=false;bool roster=false;bool vein_transition=false;};
void finish_retire_scope(RetireScope* scope) noexcept {
 const DWORD error=GetLastError();
 try{auto& r=state();
  if(scope->matched||scope->vein_transition){std::lock_guard lock(r.gate);
   if(scope->matched&&r.flight_transport_enabled)r.native_reset.finish_retire(true);
   finish_vein_lifecycle_transition(r,scope->vein_transition);}
 }catch(...){fault_worker();}
 SetLastError(error);
}
std::uint64_t invoke_remove(RetireScope* scope,PeerRemove original,void* manager,std::uint16_t key) {
 __try{return original(manager,key);}
 __finally{finish_retire_scope(scope);}
}
std::uint64_t remove_peer(PeerRemove original,void* manager,std::uint16_t key) {
 const DWORD error=GetLastError();RetireScope scope{};
 try{auto& r=state();std::lock_guard lock(r.gate);
  scope.vein_transition=begin_vein_lifecycle_transition(r);
  if(r.flight_transport_enabled)scope.matched=r.native_reset.begin_retire(reinterpret_cast<std::uintptr_t>(manager),key,false);}
 catch(...){fault_worker();}
 SetLastError(error);return invoke_remove(&scope,original,manager,key);
}
__declspec(noinline) std::uint64_t __fastcall native_remove(void* manager,std::uint16_t key){return remove_peer(state().remove_original,manager,key);}
__declspec(noinline) std::uint64_t __fastcall native_retire(void* manager,std::uint16_t key){return remove_peer(state().retire_original,manager,key);}
std::uint64_t invoke_roster(RetireScope* scope,void* manager) {
 __try{return state().roster_original(manager);}
 __finally{finish_retire_scope(scope);}
}
__declspec(noinline) std::uint64_t __fastcall native_roster(void* manager) {
 const DWORD error=GetLastError();RetireScope scope{};scope.roster=true;
 try{auto& r=state();std::lock_guard lock(r.gate);
  scope.vein_transition=begin_vein_lifecycle_transition(r);
  if(r.flight_transport_enabled)scope.matched=r.native_reset.begin_retire(reinterpret_cast<std::uintptr_t>(manager),0,true);}
 catch(...){fault_worker();}
 SetLastError(error);return invoke_roster(&scope,manager);
}
bool copy_flight_v2(const EsmVeinFlightIntentV2* source,EsmVeinFlightIntentV2* value) noexcept {
 if(!source||!value||!readable(source,sizeof(*value)))return false;
 __try{*value=*source;return true;}
 __except(memory_fault_filter(GetExceptionCode())){return false;}
}
bool copy_reset_serial(std::uint64_t* target,std::uint64_t value) noexcept {
 if(!target||!writable_range(reinterpret_cast<std::uintptr_t>(target),sizeof(*target)))return false;
 __try{*target=value;return true;}
 __except(memory_fault_filter(GetExceptionCode())){return false;}
}
bool get_reset_serial(std::uint64_t* target) noexcept {
 const DWORD error=GetLastError();std::uint64_t value=0;
 try{auto& r=state();std::lock_guard lock(r.gate);if(r.ready&&r.flight_transport_enabled&&!r.fault.load()&&r.native_reset.available())value=r.native_reset.serial();}catch(...){fault_worker();}
 const bool copied=value&&copy_reset_serial(target,value);SetLastError(error);return copied;
}
bool publish_flight_v2(const EsmVeinFlightIntentV2* source) noexcept {
 const DWORD error=GetLastError();auto& r=state();
 EsmVeinFlightIntentV2 value{};bool accepted=false;
 if(copy_flight_v2(source,&value)){
  try{
   std::lock_guard lock(r.gate);
   if(r.ready&&r.flight_transport_enabled&&!r.fault.load()&&!GetModuleHandleW(L"XHL-Vein-Mining.dll"))
    accepted=r.native_reset.publish_decision(value,GetTickCount64())==FlightPublishDecision::accepted;
  }catch(...){fault_worker();}
 }
 SetLastError(error);return accepted;
}
bool leaf_trampoline(const HookEntry& e) noexcept {
 // All seven pinned native entries begin with EXACTLY one five-byte home spill.
 // It changes neither RSP nor a register. The relay/jump also changes neither.
 // Every trampoline/relay IP is therefore a Win64 leaf frame. The remaining
 // original prologue uses its original .pdata, guarded separately.
 const auto* code=static_cast<const std::byte*>(e.original);
 if(!readable(code,33)||!readable(e.target,5)||std::memcmp(code,e.target,5))return false;
 constexpr unsigned char jump[6]={0xff,0x25,0,0,0,0};
 if(std::memcmp(code+5,jump,6)||std::memcmp(code+19,jump,6))return false;
 std::uintptr_t continuation=0,detour=0;std::memcpy(&continuation,code+11,8);std::memcpy(&detour,code+25,8);
 return continuation==reinterpret_cast<std::uintptr_t>(e.target)+5&&detour==reinterpret_cast<std::uintptr_t>(e.detour);
}
bool create_owned_hook(void*,HookEntry& e) noexcept {return MH_CreateHook(e.target,e.detour,&e.original)==MH_OK;}
bool enable_owned_hook(void*,HookEntry& e) noexcept {return MH_EnableHook(e.target)==MH_OK;}
bool disable_owned_hook(void*,HookEntry& e) noexcept {const auto status=MH_DisableHook(e.target);return status==MH_OK||status==MH_ERROR_DISABLED;}
HookOperations hook_operations() noexcept {return {nullptr,create_owned_hook,enable_owned_hook,disable_owned_hook,close_native_admission};}
