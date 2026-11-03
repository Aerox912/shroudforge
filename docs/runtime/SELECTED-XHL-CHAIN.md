# Selected XHL registration in Shroudweave

This is the typed loader route for XHL Core 2.0.6 and the selected Remote
2.0.1, Grapple 2.0.0 and Gifting 1.0.2 images. Upstream identifiers, credits,
licenses and plugin configuration semantics remain intact. Registration proves
only that the admitted Core was loaded and accepted by the SF DirectInput host.
It does not prove a plugin started, acquired a session or installed a game hook.

## Composition contract

The outer prepared policy remains schema 6. Its embedded runtime contract is
version 3, host `sf-winmm-selected-services-v3`. Version 2 with no XHL selection
keeps its existing Library-only route. Historical prepared schemas are unchanged.
The native `SelectedBootPlanV2` remains exactly 24 bytes/version 2. Core
registration is a loader responsibility and adds no native service bit.

`selection.xhl` contains `service: "xhl-core-registration-v1"`, a `container`
directory identity, all scan `directories` and an exact ordered file-name list
`files`. Each directory identity contains `relative`, a volume serial `volume`
and the full 16-byte file `id`. The container relative name is empty. Scan
identities use names relative to `mods/xhl-core/mods`; the empty name identifies
that scan root. All input content stamps remain in the existing runtime
`files` map. IDs are captured from the final installed candidate and are not
portable producer constants.

Core is exactly `mods/xhl-core/xhl-core.dll`, SHA256
`98bb1f5506f6d15097c55de274e8b8079e8b901fd3d7b934f6433dde14d3c052`.
Its private `mods/xhl-core/dinput8-chain.ini` must contain exactly ASCII
`[Options]\r\nEnabled=0\r\n`. Absence or any other content refuses. This new
candidate config disables Core's auxiliary chain, whose readable-config default
otherwise enables it. Its read lease survives Lua creation for Core's full loaded
lifetime; an admitted file alias refuses. The root SF selector is separate.
Only these three plugin DLL paths are admitted beneath the scan root; their
exact image hashes are recorded in `xhl_selection.rs`:

- `XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll`
- `XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll`
- `XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll`

The selected Gifting version remains 1.0.2, using the independently statically
accepted current private test image SHA256
`6049751069281134704447909cb7a34e13607309c5fa09b9ba63ea3bb8509280`, 142848 bytes.
The old `3f5fed...` image is refused. This static image acceptance is separate
from current-game load, session and gameplay qualification.

Every scan-tree input, including each `native-plugin.ini`, config input and
retained notice, must be listed and content-bound. This does not rewrite their
Enabled values. The matching root SF `dinput8.dll` must also be content-bound.
Core requires one explicitly approved DirectInput selection. SF retains the
whole canonical DLL-plan identity but delegates these exact private paths before
ordinary/managed classification. Later SF queue requests for those paths refuse.
A basename match elsewhere grants no delegation. Selected duplicate XHL images
outside the private tree refuse full startup.

## Namespace ownership

`xhl_tree.rs` separates the installer and runtime operations. `FreshTree::create`
requires an exclusively new absolute root and new children. It never adopts or
changes an existing directory. Populate through `write_new`, then seal. Fresh
input files receive protected ACLs before directory ACL changes, preserving
their readable input permissions during Windows inheritance propagation.

Each scan directory gets an explicit protected DACL denying the current user
`FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY | FILE_DELETE_CHILD` (`0x46`). Owner
maintenance `WRITE_DAC` remains available. Directory handles omit delete sharing
to pin names; separate file read leases omit write/delete sharing to pin input
bytes. Sharing flags alone do not freeze child creation. Runtime admission
changes no ACL: it checks exact directory IDs, owned DACL bytes, plain paths,
case-unique exact membership and all file read leases. Ancestor/container handles
pin the Core path. The admitted records remain resident for process lifetime.

Only fresh private scan directories receive this seal. Core's container stays
writable for `XHL-Native-Plugin-Host.log`. RB48's isolated no-game probe observed
plugin JSON under the EXE's `XHL/config`, outside the scan tree, even with a
separate CWD. This is evidence of those writes, not every future gameplay write.
Those known writable log/config paths cannot be included in the runtime file
map or prepared read-leased environment. Their ordinary plugin writes must stay
available even if the first real export call precedes Lua create completion.
Future plugin writes inside the sealed scan tree will fail. This design is not
a sandbox against an owner changing the DACL with `WRITE_DAC`.

The installer retains original ACLs and the exact installed seal. Bind its
candidate record to the actual owned process. Restoration is allowed only after
that child exits and every identity/owned DACL still matches. Sealing protects
children before parents; restoration processes parents before children, leaving
descendants protected against inheritance propagation until their own turn.
Each entry records Original, SealPending, Sealed, RestorePending or Restored.
A write is marked pending before Windows is called and verified by exact ID and
DACL readback. If a write reports failure after applying, a retry accepts only
the exact before/after endpoints of that recorded operation. Verified restored
entries are checked and skipped, never blindly rewritten.

`FreshTree::seal` returns `RecoveryFailure` on error, preserving the initial
error, any rollback error and the complete recovery owner. Initial rollback
failure is observable. `restore_unlaunched` likewise returns that owner on error;
call `resume_restore` on it. `RunningCandidate::restore_when_exited` keeps its
record in place on every error and releases it only after all originals verify.
Callers must retain a failure object until recovery is complete. Abandoning an
incomplete owner retains its complete boxed record, descriptors and directory
leases for process lifetime; it neither claims cleanup nor automatically retries.
This provides in-process recovery, not a durable restart recovery journal.
Unexpected identity/DACL drift refuses without overwriting it. There is no
implicit restoration at a DirectInput return or on a dropped running record.
An unlaunched candidate can be restored explicitly. Maintenance/installation
must not modify user-existing directories, global policy or accounts.
The installer preserves existing executable-root XHL configs and never overlays
defaults blindly. Deployment separately inventories those settings before any
authorized installation. An unexpected existing private Core container refuses
new-candidate ownership.

The finite private [XHL tree adapter](../../dev/xhl-tree-adapter/README.md) is
the concrete caller of this canonical helper. It pins an exact approved input
map, prepares only a new `mods/xhl-core`, emits actual IDs/hashes and waits for
the same prepared-receipt transaction ack after Deployment's complete readback.
Only then does the helper enter its separate Installed terminal state and
release handles without restoring the seal. The outcome is
`COMMITTED_SEALED_TREE`, with runtime drift detection rather than permanent byte
immutability. Rollback explicitly reports retained new paths after canonical
restoration; deletion remains a separate Deployment operation. Partial creation
errors retain acquired original objects and list unresolved just-created paths,
without reopening/adopting them. No game launch/PID, daemon or restart journal is
part of this adapter. Its README defines the finite schema and actual-ID merge
steps; staging IDs cannot be substituted for final installed IDs.

## Provider publication

Only feature-enabled SF dinput8 proxies export
`ShroudforgeSetDinput8Provider(HMODULE)`: 0 refuses, 1 registers, 2 acknowledges
the same already registered module. The host rejects null/self/conflicting
modules and forwarded exports, validates the actual `DirectInput8Create` export
and pins its owning module before publishing the callback. Runtime startup
requires successful loading and accepted registration; a rejected setter is an
error. A partial failure keeps loaded callback modules resident and does not
release the full-mode entry barrier as ready.

The typed five-argument `DirectInput8Create` forwards each real call exactly
once and returns its actual HRESULT/output. There is no synthetic DirectInput
call, manual Query/Start API or registration-failure fallback. The five other
root exports keep their system forwarding and all six original ordinals remain.
Featureless EML builds retain the original six system forwarders and no SF
winmm dependency.

## Remaining acceptance boundaries

Fresh-tree and inert-provider fixtures exercise actual production helper code;
they do not run Core, a game or plugin hooks. RB44/RB48 observed Core calling
Remote/Grapple in an unsupported host and Gifting gracefully refusing Start.
Gifting's current game qualification remains pending. First actual game export
call, scan completion, plugin startup/session and gameplay acceptance are separate.

RB54's bounded static review resolves the auxiliary chain to the adjacent INI
and the Enabled=0 bypass. The scan-tree seal plus this permanent input lease
addresses the named scan/auxiliary-chain selection routes. It is not a general
filesystem or DLL-dependency sandbox. No live installer operation, final producer
closure or release is supplied here.
