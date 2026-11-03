// Actual provider lifecycle/held dispatch and accepted writer, with OWN service
// bodies only. No game image, native hook or First Person feature code executes.
#include "abi/runtime_api.h"
#include "engine-hooks/held_entry_preparation.h"
#include "ecs/entity_session.h"
#include "graphics/render_overlay.h"
#include "loaded-resources/loaded_resource_access.h"
#include "main-menu/native_main_menu.h"
#include <windows.h>
#include <cassert>
#include <iostream>
#include <vector>
#include <stdexcept>
extern "C" bool __cdecl KfcRuntimePrepareHeldEntry(const EngineHooks::HeldEntryProof*,unsigned);
namespace Fixture {
bool hooks{},held{},feature_result=true,throw_tick{};
unsigned feature_calls{};
std::vector<unsigned> stopped;
}
namespace EngineHooks {bool StartupReady(){return Fixture::hooks;}}
namespace EcsRuntime {
bool Initialize(){return true;}void Shutdown(){Fixture::stopped.push_back(6);}
void Tick(){if(Fixture::throw_tick)throw std::runtime_error("owned synthetic tick failure");}
}
namespace ResourceRuntime {bool Initialize(){return false;}void Shutdown(){Fixture::stopped.push_back(5);}}
namespace NativeMainMenu {void Initialize() noexcept{}void Tick() noexcept{}void Shutdown() noexcept{Fixture::stopped.push_back(4);}}
namespace OverlayRuntime {bool PrepareGraphicsBoot(bool){return true;}void TickServices(){}void ShutdownServices(){Fixture::stopped.push_back(2);}}
extern "C" void __cdecl KfcRuntimeNetworkServiceShutdown(){Fixture::stopped.push_back(3);}
namespace FirstPersonControl {
bool Prepare(const EngineHooks::HeldEntryToken& token) noexcept {assert(token.ValidForCurrentThread());++Fixture::feature_calls;return Fixture::feature_result;}
void Shutdown() noexcept{Fixture::stopped.push_back(1);}
}
int main(){
    assert(KfcRuntimeInitialize());assert(!KfcRuntimeStartupReady()); // metadata init != hooks.
    Fixture::hooks=true;assert(KfcRuntimeStartupReady());
    assert(KfcRuntimePrepareHeldEntry(nullptr,0));assert(Fixture::feature_calls==0);
    assert(!KfcRuntimePrepareHeldEntry(nullptr,1));assert(!KfcRuntimePrepareHeldEntry(nullptr,2));
    EngineHooks::HeldEntryProof proof;proof.parked_thread_id=GetCurrentThreadId()+1;
    proof.is_held=+[](std::uint32_t) noexcept{return Fixture::held;};
    assert(!KfcRuntimePrepareHeldEntry(&proof,1));assert(Fixture::feature_calls==0);
    // The trusted host-verifier is synthetic here. This tests transport to an
    // owned stub, never qualifies the real parked gate or a feature profile.
    Fixture::held=true;assert(KfcRuntimePrepareHeldEntry(&proof,1));assert(Fixture::feature_calls==1);
    Fixture::feature_result=false;assert(!KfcRuntimePrepareHeldEntry(&proof,1));assert(Fixture::feature_calls==2);
    KfcRuntimeShutdown();assert((Fixture::stopped==std::vector<unsigned>{1,2,3,4,5,6}));assert(!KfcRuntimeStartupReady());
    Fixture::stopped.clear();assert(KfcRuntimeInitialize());Fixture::throw_tick=true;KfcRuntimeTick();
    assert((Fixture::stopped==std::vector<unsigned>{1,2,3,4,5,6}));assert(!KfcRuntimeStartupReady());
    std::cout<<"RB11 actual private held export passed Off/no-touch, missing/invalid proof and demand, dispatched owned token callback/refusal; StartupReady rejects metadata-only init; shutdown and tick-error stop First Person first. Real gate/feature qualification not inferred.\n";
}
