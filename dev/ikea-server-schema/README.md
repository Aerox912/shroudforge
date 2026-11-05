# IKEA dedicated-server schema qualification

Private, static metadata checker for the pinned Steam dedicated-server build
26430253. This is an independently built source derivative of the accepted
strict reflection checker. It does not change the prepared full 8K client.

The executable admits only its two fixed arguments and three fixed official
staged server files. It does not load game code, read DAT bodies, generate mod
assets, copy original files, create backups or touch the running server. The
server output directory is fixed and fresh; the operator must create its parent.
Source files and directory parents remain held without write/delete sharing.

The production path has no prior server schema baseline. Its report records
that comparison as unknown instead of comparing server metadata with a client
schema. Unresolved references and attribute collisions retain the parent's
fail-closed qualification result and diagnostic records.

Build with `IKEA_SERVER_SCHEMA_SOURCE_CLOSURE` set to the source manifest digest.
The currently pinned KFC dependency is retained under the absolute development
path in Cargo.toml and is part of the independent review packet. This private
tool is not yet a portable distributed package. The original GPL license and
upstream notice remain in this directory.

Only Deployment operates the tool after exact source/artifact acceptance.
The contract is preparation evidence, not a completed operation or runtime
compatibility result. Schema acceptance alone does not approve asset generation.

Main owns this experimental branch and its small evidence checkpoint until
the integrated IKEA candidate is accepted. Client artifacts, other feature
worktrees and live files remain outside this tool's write set.
