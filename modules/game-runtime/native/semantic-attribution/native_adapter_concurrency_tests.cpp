#include "semantic-attribution/native_adapter_test_access.h"
#include <windows.h>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <memory>
#include <thread>
#include <vector>

using namespace SemanticAttribution;
using Access=SemanticAttribution::testing::NativeAdapterTestAccess;
#define CHECK(condition) do {if(!(condition)){std::fprintf(stderr,"check failed line %d: %s\n",__LINE__,#condition);std::abort();}}while(false)

namespace {
struct Fixture {
    // Synthetic numeric coordinates only; no PE code is mapped or executed.
    Native::Site site{};
    Native::ResidentSite resident{};
    std::vector<std::byte> ledger_storage,adapter_storage;
    Ledger* ledger{};
    NativeEventAdapter* adapter{};
    Fixture() {
        const Limits limits{24,8,2,2,2,1,Native::ArchivedProfileIdentity};
        static constexpr DescriptorDefinition catalog[]={{1,1}};
        ledger_storage.resize(Ledger::RequiredStorage(limits));
        const auto result=Ledger::Construct(ledger_storage,limits,DescriptorCatalog{catalog});
        CHECK(result);ledger=result.ledger;
        site.rva=0x100;site.native_function_begin=0x100;site.expected_size=5;
        resident={&site,0x20000000,0x20,0x30};
        adapter_storage.resize(NativeEventAdapter::RequiredStorage());
        adapter=Access::Construct(adapter_storage,*ledger,{&resident,1});CHECK(adapter);
    }
    ~Fixture() {adapter->~NativeEventAdapter();ledger->~Ledger();}
};
void WaitPoisoned(NativeEventAdapter& a) {
    const auto deadline=GetTickCount64()+3000;
    while(!Access::Poisoned(a) && GetTickCount64()<deadline)std::this_thread::yield();
    CHECK(Access::Poisoned(a));
}
void CheckDrains(unsigned route) {
    Fixture f;CHECK(Access::Publish(*f.adapter));CHECK(Access::Published(*f.adapter));
    Access::LockCapture(*f.adapter);
    // Represents a capture that passed its admission check and owns the gate.
    CHECK(Access::SendPass(*f.adapter).status==EventStatus::Pending);
    auto frame=std::make_unique<Native::SavedMachine>();
    frame->relay_return=f.resident.relay+f.resident.after_return;
    frame->site_return=0x10000000+f.site.rva+5;
    HANDLE done=CreateEventW(nullptr,TRUE,FALSE,nullptr);CHECK(done);
    std::thread closer([&] {
        if(route==0)f.adapter->Close();
        else {
            if(route==2)Access::Poison(*f.adapter);
            // route1 fails the real frame boundary because frame is heap-owned,
            // outside the calling thread's current stack. route2 is pre-poisoned.
            Access::Capture(*f.adapter,*frame);
        }
        SetEvent(done);
    });
    WaitPoisoned(*f.adapter);
    CHECK(WaitForSingleObject(done,40)==WAIT_TIMEOUT);
    // A CloseAll outside the gate would already have closed this real ledger.
    CHECK(Access::SendPass(*f.adapter).status==EventStatus::Pending);
    Access::UnlockCapture(*f.adapter);
    CHECK(WaitForSingleObject(done,3000)==WAIT_OBJECT_0);closer.join();CloseHandle(done);
    CHECK(!Access::Published(*f.adapter));CHECK(Access::TransportIs(nullptr));
    Access::LockCapture(*f.adapter);
    CHECK(Access::SendPass(*f.adapter).status==EventStatus::Refused);
    Access::UnlockCapture(*f.adapter);
    CHECK(!Access::Publish(*f.adapter));
    // A previously loaded adapter pointer is retained: late entry refuses and
    // cannot publish anything. This fixture destroys it only after threads join.
    Access::Capture(*f.adapter,*frame);
}
void CheckPublishCloseRace() {
    Fixture f;
    Access::LockCapture(*f.adapter);
    std::atomic<bool> publish_started{},publish_done{},publish_result{true};
    std::thread publisher([&] {
        publish_started.store(true,std::memory_order_release);
        publish_result.store(Access::Publish(*f.adapter),std::memory_order_release);
        publish_done.store(true,std::memory_order_release);
    });
    while(!publish_started.load(std::memory_order_acquire))std::this_thread::yield();
    std::thread closer([&] {f.adapter->Close();});
    WaitPoisoned(*f.adapter);
    CHECK(!publish_done.load(std::memory_order_acquire));
    Access::UnlockCapture(*f.adapter);
    publisher.join();closer.join();
    CHECK(!publish_result.load());CHECK(!Access::Published(*f.adapter));CHECK(Access::TransportIs(nullptr));
    CHECK(!Access::Publish(*f.adapter));
}
}
int main() {
    CheckDrains(0);CheckDrains(1);CheckDrains(2);CheckPublishCloseRace();
    std::puts("SF-NA-30 NA55 repair passed: Close, frame-failure and pre-poison drain the same capture gate; real in-flight ledger mutation completes before CloseAll; publication/close race refuses; stale retained entry stays closed.");
}
