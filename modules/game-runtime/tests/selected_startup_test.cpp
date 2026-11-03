#include "../native/host/selected_startup_state.h"
#include <cassert>
#include <stdexcept>
#include <string>
#include <vector>
#include <iostream>
using namespace SelectedStartup;
struct Fixture {
    std::vector<std::string> calls;
    std::string failure;
    bool hook_available=true, execution=false;
    bool call(const char* name) { calls.emplace_back(name); return failure!=name; }
    Operations ops() {
        return {this,
            +[](void* p,bool execution) {auto& f=*static_cast<Fixture*>(p);f.execution=execution;return f.call("metadata");},
            +[](void* p) {auto& f=*static_cast<Fixture*>(p);return f.call("generic")&&f.hook_available;},
            +[](void* p) {return static_cast<Fixture*>(p)->call("map");},
            +[](void* p) {return static_cast<Fixture*>(p)->call("fp");},
            +[](void* p) {return static_cast<Fixture*>(p)->call("graphics");},
            +[](void* p) {return static_cast<Fixture*>(p)->call("provider");},
            +[](void* p) {return static_cast<Fixture*>(p)->call("scalar");},
            +[](void* p) {return static_cast<Fixture*>(p)->call("vein");},
            +[](void* p) {static_cast<Fixture*>(p)->call("stop");}};
    }
};
int main() {
    unsigned completed=999;
    { State s;Fixture f;PlanV2 p;
      assert(s.Prepare(p,f.ops(),completed)&&completed==Metadata);
      assert(!f.execution&&(f.calls==std::vector<std::string>{"metadata"}));
      assert(s.Activate(p,f.ops(),completed)&&completed==Metadata);
      assert(!s.Activate(p,f.ops(),completed)&&completed==0);
      assert(!s.Prepare(p,f.ops(),completed)); }
    { State s;Fixture f;f.hook_available=false;PlanV2 p;p.services|=GenericHooks;
      assert(!s.Prepare(p,f.ops(),completed)&&completed==0&&f.execution);
      assert((f.calls==std::vector<std::string>{"metadata","generic","stop"}));
      assert(!s.Activate(p,f.ops(),completed)); }
    PlanV2 full;full.services=AllServices;full.opt_ins=AllOptIns;
    { State s;Fixture f;assert(s.Prepare(full,f.ops(),completed)&&completed==(AllServices&~Vein));
      assert((f.calls==std::vector<std::string>{"metadata","generic","map","fp","graphics","provider","scalar"}));
      // Native prepare has not enabled Vein; host releases the INI lease next.
      assert(s.Activate(full,f.ops(),completed)&&completed==AllServices);
      assert(f.calls.back()=="vein"); }
    for(const char* failed: {"metadata","generic","map","fp","graphics","provider","scalar"}) {
      State s;Fixture f;f.failure=failed;assert(!s.Prepare(full,f.ops(),completed)&&completed==0);
      assert(f.calls.back()=="stop");assert(!s.Activate(full,f.ops(),completed));
    }
    { State s;Fixture f;f.failure="vein";assert(s.Prepare(full,f.ops(),completed));
      assert(!s.Activate(full,f.ops(),completed)&&completed==0&&f.calls.back()=="stop"); }
    { State s;Fixture f;assert(s.Prepare(full,f.ops(),completed));auto changed=full;changed.opt_ins=0;
      assert(!s.Activate(changed,f.ops(),completed)&&f.calls.back()=="stop"); }
    { State s;Fixture f;auto missing=f.ops();missing.map=nullptr;
      assert(!s.Prepare(full,missing,completed)&&completed==0&&f.calls.back()=="stop"); }
    for(unsigned unknown:{256u,0x80000000u}) {State s;Fixture f;auto p=full;p.services|=unknown;
      assert(!s.Prepare(p,f.ops(),completed)&&f.calls.empty());}
    for(unsigned opt: {0u,1u,2u,4u,8u}) {State s;Fixture f;auto p=full;p.opt_ins=opt;
      assert(!s.Prepare(p,f.ops(),completed)&&f.calls.empty());}
    { State s;Fixture f;PlanV2 p;p.services=Metadata|SharedProvider|FlightScalar;p.opt_ins=DedicatedGuest|FlightTransport;
      assert(s.Prepare(p,f.ops(),completed));assert(!f.execution);
      assert((f.calls==std::vector<std::string>{"metadata","provider","scalar"}));
      assert(s.Activate(p,f.ops(),completed));assert(f.calls.back()=="scalar"); }
    { State s;Fixture f;s.Close();assert(!s.Prepare(full,f.ops(),completed)&&f.calls.empty()); }
    std::cout<<"selected startup: exact masks, metadata/feature-only routes, required failures, Map-before-FP, deferred Vein, duplicate/unknown/changed plans passed\n";
}
