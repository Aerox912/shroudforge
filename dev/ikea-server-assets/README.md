# Private IKEA server asset preparation

This work is isolated from the accepted full 8K client build. It prepares mod
sources and checks the server package plan. It does not execute Lua, read game
assets, install files, or restart a server.

`prepare_recipe.py` binds the observed current IKEA asset pair to the retained
generation recipe. It stages 83 mod files, excluding four evidence documents
and a historical backup. The verified current Godmode Lua replaces its older
generation input. This is a preservation baseline, not the final recipe.

`prepare_ported_recipe.py` uses the pinned current full 8K package manifest to
select the newer ported Witcher, Rested, Workshop and six independent EMBER
modules. It retains Godmode, the Vein pickaxe asset patch, Gifting registration
and the grappling asset patch from the baseline. It preserves IKEA's exact
Rested and Workshop configuration bytes. Only private extension target metadata
changes to `server`; this declares the intended preparation mode, not proven
server compatibility. The four retained wrappers also receive explicit server
extensions; without them the loader defaults to client and omits those mods.
The result has 13 modules and 63 files.

The six EMBER modules represent the existing active overrides: magic furniture,
magic factories, buff refresh, cleared scene build zones, placement below fog
with the Material safety exclusion, and stack size 65535 preserving single-slot
items. The new stack module is the sole active stack owner. No-build-zone-needed
remains disabled. The original installed files and settings remain unchanged.

`ikea-server-recipe-check` uses the current production package loader with held
source admission and the server target. It records the actual ordered package
plan, issues and final source verification without running the asset runtime.
The workspace API version is 1.0.0, independently of the ShroudForge release tag.

The scripts use explicit private staging paths and pinned historical evidence.
They are project-specific preparation tools, not portable release installers.
All original module credits, notices and compatibility identifiers are retained.

Still required: accepted current dedicated-server schema, bounded server asset
generation through the prepared writer, independent effect readback, compatible
native cohort and reviewed IKEA-only update/restart. A successful package plan
alone establishes none of those results. Main owns these sources and staging
inputs until the combined server candidate is accepted; shared inputs are not
cleanup targets.
