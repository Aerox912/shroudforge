# Qualified Greeneye entrypoint continuation

Task RB84. This private ESM/Shroudweave host integration preserves GreeneyeDS's binaries and normal callback. It does not expose a gameplay service or invoke a private Greeneye RVA.

The selected full-runtime contract remains version 2/3 with the existing `greeneyePaths` field and unchanged 24-byte Native PlanV2. A nonempty Greeneye selection now requires exactly one explicitly approved ordinary Library at `mods/<selected-package>/dbgcore.dll`, SHA256 `7b97b41bd89d46528db9de4fb63b4f9fb32878192ade605c6da5e39da003687f`, 288256 bytes. Historical assets-only schemas 3/4/5 keep their separate loader route. No-Greeneye full startup keeps its existing commit path.

The full contract must bind these scan inputs, with the same read leases as the host:

| Relative input | Bytes | SHA256 |
| --- | ---: | --- |
| greeneye/find/mod.json | 265 | 911775560b0a23fd8f8571c9f75b108562f358e6653752a246ebcfe917ac1293 |
| greeneye/find/find.dll | 519680 | 0c25d97f36fb35b711e5e5240f53ea745c6077d2adf04420055dba049b03b299 |
| greeneye/map-teleport/mod.json | 279 | ca543192371fd1afc1537f12425ab1996303180b25c67d8e21184b1c5b6e92d3 |
| greeneye/map-teleport/map-teleport.dll | 444928 | 9138d31acab0623a63824390ab0418fb18183709df9758f7f4c89670cc836c78 |

Admission refuses a root `dbgcore.dll`, any extra direct `greeneye` scan entry, hotreload marker, reparse directory, missing input, unsupported host, or preloaded selected host. Existing Find/Teleport settings and non-discovered siblings such as `find_upstream.dll` are not edited. The final installer must explicitly resolve the historical root host; its presence is never silently tolerated. File leases deny writes/deletion; directory handles pin rename/delete identity. They do **not** prevent child creation. The existing prepared-mode updater/reload restrictions and a quiescent installation namespace are required; this is not a guarantee against manual concurrent additions or privileged native code.

## Original operation and ordering

1. Verified admission captures the actual canonical ordinary DLL entry and hashes the original root, complete plan/file map, request and producer receipt into an immutable ticket binding. Its files and directory handles remain process-owned.
2. The real DLL Manager queue validates its usual approval, mode and declaration checks, then consumes this exact original entry once as `deferred-entrypoint`. It does not call the loader for this host. Other libraries keep their ordinary route.
3. Required native services prepare and activate while StartupGate's exact 14-byte jump is held. Native commit with Greeneye means `EntryPending`, not full runtime readiness.
4. The private take export returns one 48-byte `EntryTicketV1` (`bytes=48, version=1, kind=1, reserved=0, binding[32]`). Return values are 0=no work, 1=ticket, -1=refused. Load returns the actual module base or zero; confirm binds the same ticket and exact returned module. No public permission or provider ABI changes.
5. Bootstrap process-pins the runtime module and arms one retained callback/context under the real held proof. StartupGate verifies its jump, restores all original 14 bytes/protection and flushes before executing that callback. `IsHeld` is intentionally false during the deferred load.
6. The load export consumes the original work once, drops the Rust mutex, uses the existing normal `LoadLibraryExW` implementation, validates the actual loaded module path, and pins the module. Neither bootstrap nor Rust unloads a potentially published hook on failure.
7. The bootstrap requires EP bytes `48 B8 <actual-module+0x2050> FF E0` and the two unchanged trailing original bytes. This is a readback profile of the exact hash above. A normal DLL load without that witness refuses, including Greeneye's patch-failure direct-scan fallback. Rust confirmation commits only the same uncancelled operation/module.
8. Only then does the gate signal successful continuation and invoke the actual game EP. The worker waits for that success before `create`. Greeneye's own trampoline restores its saved original EP, scans its normal root namespace and calls the original game EP. No World/player wait is introduced.

Duplicate takes, mismatched tickets, early create, cancellation, wrong module or false witness cannot revive the admission. Callback context, events, loaded modules, host bytes and all four scan inputs remain resident/leased through process exit. A module PIN after rejected arming is conservative residency. There is no timeout that would release code while its callback may still execute.

The observed terminal DLL status is `loaded-entrypoint-hooked-scan-pending`. It witnesses the qualified host load and EP hook, **not** plugin Query/Start, session/gameplay success, or support on every game mode. No actual Greeneye DLL is loaded by the local tests.

## Focused checks

`modules/entrypoints/windows-bootstrap/tests/run_greeneye.ps1 -Output <E: task output>` builds only an inert fixture. It uses the real StartupGate and continuation callback with owned generated EP bytes and simulated loader/confirmation functions. It covers no-host behavior, restore-before-load, exact hook plus original tail, duplicate arming, failed/missing hook and held-jump tamper. Failure cases must exit 1114 before entry.

`cargo test --offline --locked -p shroudforge-api --lib greeneye_entry -- --test-threads=1` covers the real DLL Manager queue's deferral, original ticket/phase behavior, load outside the admission mutex, no early create, duplicate/mismatch/cancel refusal, retained module reference on failure, exact input profiles and namespace refusal. Synthetic source closures used for these tests do not describe a production runtime. The private embedded exports and actual lifecycle TU must also compile; their calls are part of the independent integration review.
