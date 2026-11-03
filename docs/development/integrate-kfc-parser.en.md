# Integrate the KFC parser

`kfc-parser/` is the Git submodule itself. Rust consumers use its crates through local Cargo paths. There is no additional parser wrapper.

Format access and resource resolution live in `crates/kfc-base`. Existing codecs live in `crates/kfc-content` and `crates/kfc-resource`. `crates/kfc/src/schema.rs` and `snapshot.rs` contain schema projection and snapshot export. The current prepared-generator pin is `ddeb44ecd44d80d1964fcc0d1c3fff29a1c49d54`, adding the generated-only incremental writer while retaining the integrated parser and loader bridge. Git submodules store commit IDs, so the parent repository records the exact commit rather than a branch selector. The prior `sf-v0.2.0` integration revision `db1580a98dcd679cb9f309b8e26fac97f0734de4` is historical context, not the current prepared-generator pin.

After a fresh checkout, initialize the parser submodule at the parent repository's recorded gitlink. `dev/parser/prepare_parser.ps1` verifies an exact parser revision, so keep its accepted revision synchronized with the parent gitlink before using the full dev or release build actions. The prior integrated revisions `db1580a9`, `2cf45e36`, and `742b1cb7` do not need a patch. For the legacy revision `34a156f6`, the helper retains support for `dev/parser/parser-integration.patch` and detects changes that have already been applied.

Game-file selection belongs to `modules/entrypoints/loader/game_files.rs`. Original files, parser state, and the persistent file transaction belong to `modules/game-files/`. The selected mod configuration fingerprint is calculated in `modules/modloader/calculate_preparation_fingerprint.rs`. Live game access and the native ABI belong to `modules/game-runtime/`.

The workspace is defined in `Cargo.toml` and the release composition is in `dev/build/release_build.ps1`. This integration does not introduce new parser or mod formats.
