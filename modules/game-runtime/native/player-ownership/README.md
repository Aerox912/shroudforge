# Native player ownership and shared callback scope

This page records bounded offline component state in the ShroudForge worktree.
It does not claim runtime, multiplayer, mining, or gameplay readiness.

## Flight and service lifetime

FLIGHT64, FLIGHT70, and FLIGHT75 accepted their stated, bounded review scopes.
The production admission policy is now enabled, but the real service phase
starts **Unprepared**. Readers remain closed until successful held-entry
preparation publishes Ready. The captured alternate teardown calls the wrapper
at `0x9E9FD0` before freeing its snapshot arrays; the wrapper drains readers,
forwards the original once, and permanently retires admission. Retired does not
reopen before process restart.

The Flight actor path reads the full entity-ID DWORD already present on the
original actor stack. It avoids borrowing a heap query row across the three
captured structural-deletion paths. The broader replay-driver parent lifetime is
only partly traced; these reviews cover bounded captured routes, not every
possible teardown route. Client prediction and actual multiplayer effects
remain unverified. The native server callback changes only the selected
player's pitch operand; it does not write stamina or shared glider resources
and does not implement level hold.

## Shared World callback scope (B)

The host's existing held-entry coordinator calls `PrepareServiceRetirement`
then prepares the common World wrapper. Preparation validates the supported
server image and original callsite, retains the original target, and publishes
an RX relay through the canonical executable writer. This common wrapper does
not depend on Flight's scalar hook or XSTATE setup. Flight Off or scalar-hook
failure leaves the shared wrapper installed.

On the callback worker, the caller owns a process-shared World interval lease.
`CopyCallbackPlayers` takes a short registry/service/snapshot read scope,
copies the current player and machine snapshots with full-key bindings, and
releases those reader scopes before returning. The caller keeps its own World
lease; the copy helper transfers no worker or stack authority. Flight retains
its existing validation, two-copy identity checks, and registry ordering.

B adds only this shared player/World scope. Query-D/Q ownership, feature-program
and actor integration remain separate work; B adds no Vein mining hooks. The
policy bit being true and a prepared player service do not prove that a World
producer or feature hook is installed. Final integration health must expose
hook-preparation failures; do not add a gameplay-ready flag here.

The first B gate passed 20 offline checks and a native Release build. A later
focused regression exposed a final-coordinator edge: the helper could expose
Ready before the outer held-entry token check. The successor records only
pending publication inside that callback and consumes it once after the actual
coordinator result. A failed final check permanently refuses this publication;
its retained wrapper still forwards the original. Direct helper success alone
cannot expose common World readiness. The separately reviewed service lifetime
phase is unchanged.

Six focused successor checks passed, including final-coordinator refusal,
positive combined preparation, cold state, short-reader release and scalar
failure independence. The successor build and independent B review are tracked
in the frozen review packet. Runtime, multiplayer and IKEA readiness remain
unclaimed. A private wiring fixture uses only a non-game test host; no game or
server was used. Vein effect sources in broad inventories are not compiled into
the native target until their separate integration is registered.

## Separate Vein intent package (A)

Package A is a separate accepted per-peer desired-intent table; it is not a
mining effect or readiness signal. The separately owned r4 effect sources were
present in the B source inventory but not compiled into `kfc-runtime`. A adds no
stamina or level-hold behavior, and its accepted component review does not
establish that Vein hooks or the World producer are installed. No live game or
server was used for A.

## Evidence

- Flight lineage: [r3 source manifest](E:/Build/sf-flight-retirement-paths-20261029-r1/review-r3/MANIFEST.json), [FLIGHT64 review](E:/Build/sf-combined-finish-20261029/verification/flight-r3/FLIGHT64-REVIEW-EN.md), [FLIGHT70 review](E:/Build/sf-combined-finish-20261029/verification/flight-preparation/FLIGHT70-REVIEW.md) and [FLIGHT70 packet](E:/Build/sf-flight-positive-preparation-20261030-r1/review-r1/MANIFEST.json), [FLIGHT75 review](E:/Build/sf-flight-production-admission-20261030-r1/FLIGHT75-ACCEPTED-REVIEW.md).
- B first revision evidence (superseded by final-coordinator repair): [RESULT.json](E:/Build/sf-shared-player-callback-20261030-r1/RESULT.json), [20/20 CTest log](E:/Build/sf-shared-player-callback-20261030-r1/final-tests.log), and [private-host wiring log](E:/Build/sf-shared-player-callback-20261030-r1/wiring.log). The Release build and 2,026-input pre/post pins are recorded in `RESULT.json`.
- A accepted scope: [VEIN84 review](E:/Build/sf-vein-intent-control-20261030-r1/VEIN84-ACCEPTED-REVIEW.md) and [frozen manifest](E:/Build/sf-vein-intent-control-20261030-r1/review-r1/MANIFEST.json), SHA-256 `c740c59036f3333b50b04a53e135a06ac6ac72b6a683174aa7827d3e9d976f92`.

- B final-coordinator successor: [focused tests](E:/Build/sf-shared-player-callback-20261030-r1/r2-focused-tests.log); independent successor review pending.
