// Execute the actual host lifecycle with inert owned feature implementations.
#include <cassert>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
#include "../native/host/manage_runtime_lifecycle.cpp"
namespace Fixture {
std::vector<std::string> calls;
bool execution{}, hooks{}, graphics_throws{}, tick_throws{}, selected{};
void record(const char* name) {calls.emplace_back(name);}
}
namespace EcsRuntime {
bool Initialize(bool execution) {Fixture::execution=execution;Fixture::record("initialize");return true;}
bool Initialize() {return Initialize(true);}
void Tick() {if(Fixture::tick_throws)throw std::runtime_error("fixture tick failure");}
void Shutdown() {Fixture::record("ecs-stop");}
}
namespace ResourceRuntime {bool Initialize(){return true;}void Shutdown(){Fixture::record("resource-stop");}}
namespace NativeMainMenu {void Initialize() noexcept{}void Tick() noexcept{}void Shutdown() noexcept{Fixture::record("menu-stop");}}
namespace FirstPersonControl {bool Prepare(const EngineHooks::HeldEntryToken&) noexcept{return false;}void Shutdown() noexcept{Fixture::record("fp-stop");}}
namespace MapRuntime {void StopCapture() noexcept{Fixture::record("map-stop");}} // Inert test double only.
namespace shroudforge::flight {void stop_client() noexcept{Fixture::record("flight-stop");}}
namespace VeinFlight {void stop_provider() noexcept{Fixture::record("provider-stop");}}
namespace OverlayRuntime {
bool PrepareGraphicsBoot(bool){return true;}void TickServices(){}
void ShutdownServices(){Fixture::record("graphics-stop");if(Fixture::graphics_throws)throw std::runtime_error("fixture shutdown failure");}
}
namespace EngineHooks {
bool StartupReady(){return Fixture::hooks;}
bool RunHeldEntryPreparation(const HeldEntryProof&,HeldEntryCallback,void*) noexcept{return false;}
}
namespace ProviderImplementation {void close_selected_startup() noexcept{Fixture::record("close");}bool selected_startup_attempted() noexcept{return Fixture::selected;}}
extern "C" void KFC_RUNTIME_CALL KfcRuntimeNetworkServiceShutdown(){Fixture::record("network-stop");}
int main(){
    assert(ProviderImplementation::initialize_provider(false));
    assert(!Fixture::execution&&!KfcRuntimeStartupReady());
    Fixture::calls.clear();Fixture::graphics_throws=true;
    KfcRuntimeShutdown();
    const std::vector<std::string> stop={"close","flight-stop","provider-stop","fp-stop","map-stop","graphics-stop","network-stop","menu-stop","resource-stop","ecs-stop"};
    assert(Fixture::calls==stop&&!ProviderImplementation::provider_initialized.load());
    Fixture::calls.clear();Fixture::graphics_throws=false;Fixture::hooks=true;
    assert(KfcRuntimeInitialize()&&Fixture::execution&&KfcRuntimeStartupReady());
    Fixture::calls.clear();Fixture::tick_throws=true;KfcRuntimeTick();
    assert(Fixture::calls==stop&&!KfcRuntimeStartupReady());
    Fixture::calls.clear();Fixture::selected=true;assert(!KfcRuntimeInitialize()&&Fixture::calls.empty());
    std::cout<<"actual native host: metadata excludes generic execution, legacy includes it, unchanged hook readiness, partial/exception stop ordering passed\n";
}
