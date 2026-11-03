// SPDX-License-Identifier: MIT
#include "flight/owner_state.hpp"
#include <cassert>

// Catches takeover, stale retirement and default-On regressions in the real
// control state. No native process, hooks or game files are involved.
int main() {
    shroudforge::flight::OwnerState state;
    assert(!state.requested());
    assert(!state.claim(0));
    assert(state.claim(17));
    assert(!state.claim(18));
    assert(!state.set(18, true, true));
    assert(!state.requested());
    assert(!state.set(17, true, false));
    assert(!state.requested());
    assert(state.set(17, true, true));
    assert(state.requested());
    assert(!state.reset(18));
    assert(state.requested());
    assert(state.reset(17));
    assert(!state.requested());
    assert(state.set(17, true, true));
    assert(!state.retire(18));
    assert(state.requested());
    assert(state.retire(17));
    assert(!state.requested());
    assert(!state.set(17, true, true));
    assert(state.claim(18));
    assert(!state.requested());
    assert(!state.retire(17));
    assert(state.set(18, true, true));
    state.provider_reset();
    assert(!state.requested());
    assert(state.owns(18));
}
