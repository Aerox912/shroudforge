#include "../native/ecs/generic_write_policy.h"
#include <cassert>
#include <iostream>
#include <windows.h>
static_assert(FileIdInfo==18 && sizeof(FILE_ID_INFO)==24); // Rust's private file-lease FFI layout.
int main() {
    EcsImplementation::GenericWritePolicy policy;
    // All original direct-view conditions can be true: read_only_binding,
    // LocalAuthority, matching current view, world-bound registry and an external
    // direct-write profile. They cannot bypass the selected execution fence.
    const bool direct_local_authority_profile_admission = true;
    assert(policy.Allows(false, direct_local_authority_profile_admission)); // unchanged legacy
    policy.Selected(false);
    assert(!policy.Allows(false, direct_local_authority_profile_admission));
    assert(!policy.Allows(true, direct_local_authority_profile_admission)); // unrelated hooks cannot opt in
    policy.Selected(true);
    assert(!policy.Allows(false, direct_local_authority_profile_admission));
    assert(policy.Allows(true, direct_local_authority_profile_admission));
    assert(!policy.Allows(true, false)); // selection/readiness cannot replace original gates
    policy.Legacy();assert(policy.Allows(false, direct_local_authority_profile_admission));
    std::cout<<"selected generic writes: external direct-profile admission cannot bypass disabled/missing hooks; original gates and legacy behavior retained\n";
}
