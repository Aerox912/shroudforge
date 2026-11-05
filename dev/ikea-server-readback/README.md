# Private server resource readback

This separate reader exports paired baseline/current typed resource projections
for independent semantic checks. It never runs Lua or native game code. The
baseline index is the exact current server index; both cursors read resource
bytes from the generated resource file. Before interpreting the baseline, the
reader independently hashes its first 144,056,320 bytes against the known
original resource hash. It rejects a shorter or changed prefix and leaves the
appended tail intact. It never opens the original EXE, DAT or resource file.

The caller must first review and select the exact request, artifact, captured
schema consumer and bounded outer process. The reader is not an operation
approval mechanism. A lost/failed operation remains spent under that outer.
The prepared stage must spell the exact fixed local drive path, either ordinary
or with the Windows `\\?\` prefix. This comparison does not resolve aliases,
dot segments, UNC paths or device namespaces; held directory checks still apply.
The fixed request binds the prepared receipt and reader hash. All stage members
are held and hash-verified before/after, along with the exact baseline index and
captured registry. The strict negative schema decision remains visible.

The report preserves original container/content mappings and resource identities,
and checks that new content points to generated containers. It does not decode
content bytes, establish gameplay behavior or approve deployment. Each resource
record includes full raw baseline/current digests, but selected fields only for
items, scenes and unchanged templates. Changed templates retain all components.
Seven core resource kinds are covered completely; additional changed/new kinds
are selected from the held index delta and decoded in full. Unchanged Impact
programs are omitted except the Rested target. Every visited type/count is recorded in the terminal completion
record. A report without that final record is incomplete.

Limits: 32 staged files/2 GiB total, 100,000 resources, 16 MiB per decoded
resource, 4 GiB cumulative decoded bytes, 256 MiB JSONL. SDK compressed-chunk
reads/caches need the separately reviewed process memory/time limits. The sole
original input is the 4,263,936-byte KFC index, read for initial/final hashes
and twice by the SDK. No backup or original-file copy is created.

Main retains this source, build and fixtures until independent readback and
combined server acceptance. Retire owned intermediates after that checkpoint;
preserve evidence and the client/full 8K candidate.
