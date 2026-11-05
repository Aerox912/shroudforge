# Private server preparation work

This experimental tool prepares a private server asset stage. It has three
separate commands: `check-schema`, `plan-staged` and `prepare-once`. It never
installs files, recovers a transaction, creates backups or controls processes.

The `ikea-server-prepared` feature is disabled in ordinary builds. It adds a
server-only schema constructor that retains the strict negative observation and
full collision ledger. All captured files, the exact recipe and its package
plan are pinned. The SDK serialization roundtrip must preserve the entire
captured representation. Client schema admission and ordinary cache loading are
unchanged.

The separate source-lease entrypoint admits only E development roots or the
one named remote private server staging environment. It rejects replacements
that have not already been staged. Its directory handles request generic read
access with no delete sharing, including all package directories. The ordinary
E-only source API retains its existing behavior.

`plan-staged` reads only the exact mod/evidence closure under the fixed remote
private environment. It seals the actual directory traversal and creates a
request after the server plan and source finalization pass. It reads no game
assets. `prepare-once` requires a fresh, hash-selected operation, independently
accepted source/artifact and operation reviews, and an exact compiled source
closure. It creates a durable consumed marker before entering preparation.

Preparation holds the source directories and all 13 original files read-only.
The three known source hashes must match. The ten DAT files must match the
captured namespace, sizes, file identities and timestamps; their content
hashes are first observed by this operation, then verified after generation.
These hash-pass byte totals do not count the asset runtime's separate reads.
The reviewed outer process must provide the actual memory, deadline and log
limits. Input and evidence leases survive result serialization. Generated
output is private and requires independent semantic readback before any
deployment transaction may use it.

Seven focused behavioral tests cover write/rename refusal, hardlinks and
metadata drift, path and expiry boundaries, non-overwriting receipts and the
exact 63-file server plan. They execute no game asset patches or native mods.
Passing those tests or `check-schema` is not server compatibility acceptance.

Main owns this candidate and its build intermediates until writer and combined
server acceptance. Retain its exact source, receipts and current inputs; do not
clean shared/client checkpoints as part of this work.
