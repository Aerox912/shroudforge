// SPDX-License-Identifier: MIT
// The pinned 863BF0 and 87A1D0 callers check the complete DWORD transport
// channel before calling 3A8890 (the latter through pinned helper 85AFD0).
// The producer checks that channel against its input record and
// resolves ClientPlayerInput for that record's entity before reaching the packer.
// This synchronous scope is packet authority. Cached values alone are not.
struct NativeInputScope {
 NativeInputScope* prior;
 std::uintptr_t context;
 std::uint32_t channel;
 bool qualified;
};
thread_local NativeInputScope* current_input_scope=nullptr;
bool qualified_input_packet(const NativeInputScope* scope,std::uintptr_t caller,std::uintptr_t base) noexcept {
 return scope&&scope->qualified&&!scope->prior&&scope->context&&scope->channel&&
  base&&base<=UINTPTR_MAX-input_packet_caller_rva&&caller==base+input_packet_caller_rva;
}
std::uint64_t invoke_input(NativeInputScope* scope,void* output,void* buffer,std::uint32_t tick,void* context,std::uint32_t channel,std::uint8_t flag) {
 __try{return state().input_original(output,buffer,tick,context,channel,flag);}
 __finally{current_input_scope=scope->prior;}
}
__declspec(noinline) std::uint64_t __fastcall native_input(void* output,void* buffer,std::uint32_t tick,void* context,std::uint32_t channel,std::uint8_t flag) {
 const auto caller=reinterpret_cast<std::uintptr_t>(_ReturnAddress());
 const DWORD entry_error=GetLastError();
 const auto base=state().game_base;
 NativeInputScope scope{current_input_scope,reinterpret_cast<std::uintptr_t>(context),channel,
  !current_input_scope&&output&&buffer&&context&&channel&&flag==0&&base&&
  base<=UINTPTR_MAX-input_transport_second_caller_rva&&
  (caller==base+input_transport_caller_rva||caller==base+input_transport_second_caller_rva)};
 current_input_scope=&scope;
 SetLastError(entry_error);
 return invoke_input(&scope,output,buffer,tick,context,channel,flag);
}
