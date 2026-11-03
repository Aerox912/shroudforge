#include "../static_start.h"
#include <cassert>
#include <iostream>
#include <thread>
using namespace BootstrapImplementation;

int main() {
    assert(ClassifyHostAttach(true, true, false) == HostAttach::Reject);
    assert(ClassifyHostAttach(true, true, true) == HostAttach::Deferred);
    assert(ClassifyHostAttach(false, true, false) == HostAttach::Inert);
    assert(ClassifyHostAttach(true, false, false) == HostAttach::Legacy);
    assert(ClassifyHostAttach(true, false, true) == HostAttach::Legacy);
    StaticStart selected;
    int gates = 0, workers = 0;
    const auto begin = [&] { ++gates; ++workers; return true; };
    // Import alone and EML selection do not call Try; a late proxy cannot start.
    assert(!selected.Try(false, begin));
    assert(gates == 0 && workers == 0);
    assert(selected.Try(true, begin));
    assert(selected.Try(true, begin)); // second statically attached proxy
    assert(!selected.Try(false, begin)); // late attach is refused even after success
    assert(gates == 1 && workers == 1);
    StaticStart failed;
    int attempts = 0;
    assert(!failed.Try(true, [&] { ++attempts; return false; }));
    assert(!failed.Try(true, [&] { ++attempts; return true; }));
    assert(attempts == 1);
    StaticStart exception;
    assert(!exception.Try(true, []() -> bool { throw 1; }));
    assert(!exception.Try(true, [] { assert(false); return true; }));
    StaticStart reentrant;
    assert(reentrant.Try(true, [&] {
        assert(!reentrant.Try(true, [] { assert(false); return true; }));
        return true;
    }));
    StaticStart concurrent;
    std::atomic<bool> entered = false, finish = false;
    std::thread first([&] {
        assert(concurrent.Try(true, [&] {
            entered.store(true);
            while (!finish.load()) std::this_thread::yield();
            return true;
        }));
    });
    while (!entered.load()) std::this_thread::yield();
    assert(!concurrent.Try(true, [] { assert(false); return true; }));
    finish.store(true);
    first.join();
    assert(concurrent.Try(true, [] { assert(false); return false; }));
    std::cout << "static attach policy, once-only success/failure, reentry and concurrent start passed\n";
}
