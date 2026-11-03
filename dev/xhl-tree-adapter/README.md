# Private XHL tree adapter

This Windows Rust tool owns one install transaction for a new private
`mods/xhl-core` container. It reuses the canonical runtime `xhl_tree.rs` ACL and
recovery code and the data-only `xhl_pins.rs` image pins. It never loads a DLL,
launches a game, accepts a PID, watches a directory or installs a background
service. It is not a public binary release or an installer for the rest of the
composition. Deployment owns invocation and its complete install transaction.

## Approved plan, schema 1

Supply a UTF-8 JSON file and its separately approved exact SHA256:

```
sf-xhl-tree-adapter.exe <approved-plan.json> <approved-plan-sha256>
```

The digest is over the exact input bytes. No digest, path or selection defaults
are inferred. Unknown fields refuse. The finite schema is:

| Field | Type and meaning |
|---|---|
| `schemaVersion` | Integer 1 |
| `transaction` | 16–128 ASCII letters/digits/hyphens/underscores, chosen by the outer transaction |
| `sourceRoot` | Absolute existing source directory |
| `sourceRootIdentity` | `{ "volume": u64, "id": [16 byte integers] }` from that actual directory |
| `targetParent` | Absolute existing `mods` directory; only its new `xhl-core` child may be created |
| `targetParentIdentity` | Actual parent identity, same shape |
| `files` | Map from exact paths relative to the new Core container to input records below |

Each input record has `source` (relative source path), `sourceIdentity` (actual
source file identity), `bytes` (u64) and lowercase `sha256`. Only the adjacent
`dinput8-chain.ini` uses null source/identity: its exact declared bytes/hash must
equal canonical `[Options]\r\nEnabled=0\r\n`. The adapter writes those bytes only
through `FreshTree::write_disabled_chain_new`. All other files, including plugin
INIs, notices and configuration inputs, come from the explicit approved map.
There are no generated plugin defaults or changes to existing XHL settings.

Required code paths are `xhl-core.dll` and the three `mods/<plugin>/<plugin>.dll`
paths in canonical `xhl_pins.rs`, plus each private plugin's `native-plugin.ini`.
Other DLLs refuse. Core/Remote/Grapple/current private Gifting hashes are exact;
Gifting is also exactly 142848 bytes. Writable Core log and executable-root XHL
config outputs are not inputs. Limits: 128 files, 32 MiB/file, 64 MiB total and 1 MiB
plan. Nested paths belong only under the `mods/` scan subtree.

The assembler must obtain real FileIdInfo18 identities (u64 volume serial and
full 128-bit ID, 24-byte Win32 layout) from held plain directory/file handles. The
adapter independently rechecks them and pins every source parent ancestry,
source file and target parent. Reparses, source aliases, overlapping roots,
unexpected existing target ownership, byte drift and incorrect identities refuse.
Never supply zero/fabricated IDs or derive IDs from names. JSON consumers must
preserve u64 values exactly; do not round them through JavaScript doubles.

## Finite subprocess protocol

After preparing, one flushed JSON line has `outcome: "PREPARED_TREE"`,
`preparedReceiptSha256` and `receipt`. The process stays alive holding its leases.
This means only that the private tree was prepared, not that the full install
or any plugin is ready. On a failure after creation it emits `PREPARE_FAILED`
with `ownerRetained: true` and accepts rollback commands while preserving state.

The prepared receipt schema 1 contains:

- `transaction`, `approvedPlanSha256` and the actual `target` path.
- `container`: actual directory pin `{relative:"",volume,id}`.
- `directories`: actual scan pins, relative to `mods/xhl-core/mods`; empty names
  identify the scan root. The Core container itself is not sealed, so its log
  remains writable.
- `files`: actual readback entries keyed by `mods/xhl-core/<relative-file>`, each
  with `bytes`, `sha256` and diagnostic file `identity`.

The prepared digest is over canonical serde serialization of that receipt.
Copy the emitted digest exactly instead of recomputing it with another JSON
serializer. After the **same complete outer install transaction** has committed
and Deployment has read it back, send one line:

```
{"command":"commit","ack":{"transaction":"<issued transaction>","approvedPlanSha256":"<issued plan digest>","preparedReceiptSha256":"<issued receipt digest>"}}
```

The ack is trusted internal transaction sequencing, not a privileged permission
token. The adapter revalidates the owned tree and file hashes itself. Bad ack or
drift returns `COMMIT_REFUSED` with ownership retained. Success returns
`COMMITTED_SEALED_TREE`, releases leases without restoring directory ACLs, and
exits. The canonical helper uses a distinct Installed terminal state. This is
**not permanent byte immutability**: runtime must reacquire leases and reject
hash/ID/DACL drift before loading Core.

`{"command":"rollback"}` invokes canonical recoverable ACL restoration. Success
is explicitly `ACL_RESTORED_NEW_TREE_RETAINED`, or `UNSEALED_NEW_PATHS_RETAINED`
if sealing never began. It lists actual known created paths/IDs, unresolved
just-created paths and unresolved write attempts. It does not delete anything or
claim full outer-transaction rollback. Deployment owns any later authorized
removal of verified task-created paths. Failed restoration retains its original
recovery owner and accepts another rollback attempt. No arbitrary existing path
is reopened and adopted to manufacture missing creation identity.

`{"command":"status"}` reports the retained owner/receipt. Invalid commands
refuse. EOF, kill or process failure can leave an uncommitted new tree requiring
separate maintenance. There is no crash-safety guarantee, daemon or restart
journal. Nick launches manually after the full installation is accepted.

## Actual-ID merge for assembler and Deployment

Keep the adapter alive while merging its **actual** receipt into the final
schema6/runtime-contract3 composition:

1. Copy `receipt.container` and `receipt.directories` unchanged into
   `runtime.selection.xhl.container` and `.directories`.
2. Set `.service` to `xhl-core-registration-v1`. Set `.files` to sorted file-map
   keys beginning `mods/xhl-core/mods/`, stripping that prefix. This list is the
   exact sealed scan membership.
3. Merge every receipt file into `runtime.files` under its emitted full key,
   projecting **only** `bytes` and `sha256`. Diagnostic `identity` is not a
   runtime NativeFile field. Keep existing unrelated runtime files and all the
   independently required request/receipt/source-closure bindings intact.
4. Bind the exact matching SF root dinput8 proxy and the approved canonical DLL
   plan separately. Core uses DirectInput registration; the three private DLLs
   remain Core-owned, not ordinary/managed SF duplicate loads. Native PlanV2 is
   unchanged.
5. Complete outer publication/readback, send the matching commit ack, require
   `COMMITTED_SEALED_TREE`, then finish the outer receipt. Runtime validates only.

IDs are for the physical tree actually prepared. Copying an E-only candidate to
another tree/volume does not preserve its IDs. A final installation must invoke
this adapter on its own newly owned actual target and rebind its resulting IDs
and composition receipts. No staging IDs may be promoted as installed IDs.

## Verification boundary

Focused tests use fresh E-only fixture trees, real approved DLL bytes as data,
independent actual-ID readback and own inert adapter subprocesses. They do not
load Core/Gifting, invoke DirectInput or establish session/gameplay readiness.
Existing user directories/configs, backups, Git and live games remain untouched.
