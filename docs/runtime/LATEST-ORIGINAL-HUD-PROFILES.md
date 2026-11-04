# Latest original HUD profiles

The finite HUD owner graph recognizes these additional exact publisher images:

| Original | File SHA-256 | Callback RVA | Original-renderer continuation slot |
| --- | --- | --- | --- |
| Creative Mode 3.1.2 | `4badee09d1329d1f246d4b38eee2d30f8bc90b6fc38c2c662c3d2b7d1d23fd05` | `0x4e67` | `0x57c208` |
| Find 1.2.8 | `1660249c2312258600f7bd7c9ce2469e3bc8c7e27f71896d77fb77c9e50669b0` | `0xe290` | `0x7b6f0` |
| Embervale Teleport 1.1.10 | `438610671f1dfe0ce4b9d24f45a878664a5d9d504e6f797e21b44aa3d39a6630` | `0x3110` | `0x6a4b8` |

Creative's callback thunk, 0x103-byte body and 0x187-byte trampoline template are byte-identical to the existing 3.1.1 profile. Both versions have the same callback RVA and image size. The finite graph uses its already observed PE timestamp to select the 3.1.2 candidate (`0x6aea306f`), then still requires the complete file hash and mapped-byte checks. Timestamp is only a routing hint. Historical helpers without this observation retain their previous closed selection.

Find and Teleport each have exactly one executable-section match for their predecessor callback instruction bytes with only RIP-relative displacements and relative branch/call immediates masked for discovery. The discovered callback bodies are then pinned without masks. Their original-renderer slots are the RIP-relative indirect call operands, not the other state fields in those bodies. Neither new body has PE base relocations. No masked comparison runs in the game.

These descriptors preserve the existing finite graph's read bounds, maximum three provider families, duplicate/cycle rejection, full-file identity, lifetime leases and before/after mapped-byte stability. Prior profiles remain available. This change does not admit a new game build, qualify the new Greeneye loader entrypoint, authorize hot reload, or establish gameplay compatibility.

`kfc-runtime-test-public-mod-profile` reads the three original DLLs as files, validates their exact geometry, hashes, callback/template spans and Creative selection, and rejects modified bytes. It never loads those DLLs. Run it with Creative, Find and Teleport file paths in that order. The affected map-capture regression target also remains required.

Publisher sources: [Creative Mode](https://www.nexusmods.com/enshrouded/mods/146?tab=files), [Find 1.2.8](https://github.com/Greeneye0/find-enshrouded/releases/tag/v1.2.8), [Teleport 1.1.10](https://github.com/Greeneye0/embervale-teleport-enshrouded/releases/tag/v1.1.10). Original authors, binaries and licenses are unchanged.
