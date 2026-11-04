# Greeneye Loader 1.2.2 entry continuation

Profile `greeneye-entry-continuation-v2`, version 2, extends the existing original-entry continuation without changing its 48-byte ticket ABI. Ticket kind 1 retains the original profile. Kind 2 selects only this exact loader and scan set:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| dbgcore.dll | 294912 | 973a2284ba4faf24e55c9e4b165c136c81dec4d3b1706b3f21f9cbd314c26eed |
| greeneye/find/mod.json | 265 | 937c32f41858bae9b2fa9fceef1ce53c01bac39cd4e048c13c20545a3235e512 |
| greeneye/find/find.dll | 522752 | 1660249c2312258600f7bd7c9ce2469e3bc8c7e27f71896d77fb77c9e50669b0 |
| greeneye/map-teleport/mod.json | 280 | 17e852be7d064260a17412df91f3b6b604ebb96e16c94d1c56d025ad5ed34b41 |
| greeneye/map-teleport/map-teleport.dll | 448512 | 438610671f1dfe0ce4b9d24f45a878664a5d9d504e6f797e21b44aa3d39a6630 |

The publisher's Find 1.2.8 and Teleport 1.1.10 archives contain identical loader bytes. Offline PE decoding identifies the new loader's patch construction at RVA 0x4dd5: `48 B8 <module+0x2080> FF E0`, written as exactly 12 bytes to the saved executable entrypoint. The callback beginning at 0x2080 restores those saved bytes and page protection, flushes the instruction cache, calls the loader scan at 0x2b50 from 0x2104, then calls the saved original entrypoint from 0x2113. If the callback cannot obtain write protection at 0x20a0, upstream skips restoration and still scans and calls the saved entrypoint. The pre-entry witness does not prove that later restoration succeeds. Patch failure can take a direct scan path; it must not count as a successful continuation witness.

The bootstrap restores its own held entry bytes before invoking the ordinary loader operation. For kind 2 it then requires the exact 12-byte jump to module+0x2080 and the two unchanged trailing original bytes. It never invokes the private RVA directly. Kind 1 still requires module+0x2050. Unknown kinds fail. Rust confirmation must retain the same ticket, full-file/scan bindings and module identity. The existing worker barrier, poisoned-failure state, resident-module policy and no-retry ownership rules are unchanged.

This profile qualifies only the entry handoff shape of the exact original bytes. It does not qualify the current game build, plugin startup/gameplay, hot reload, native HUD providers or multiplayer. The ordinary root namespace remains restricted to the selected Find and Teleport inputs and preserved settings/history; a separate root dbgcore.dll or unbound scan directory is refused.

Required validation: runtime rejects mixed old/new host and scan identities; assembler requires the matching versioned profile document and exact artifacts; inert bootstrap tests cover both kinds, wrong-kind targets, missing hooks and trailing-byte tampering. No original DLL is executed by these tests.

Sources: https://github.com/Greeneye0/find-enshrouded/releases/tag/v1.2.8 and https://github.com/Greeneye0/embervale-teleport-enshrouded/releases/tag/v1.1.10. Original binaries and credits remain unchanged.
