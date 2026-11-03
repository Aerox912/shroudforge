# Static proxy startup

The SF build of the existing dbghelp and dinput8 proxies uses the opt-in
`sf-static-startup` Cargo feature. Both retain their original forwarding exports.
Build the x64 MSVC bootstrap first: its import library is explicitly named
`ShroudForgeWinmm.lib`, while its DLL remains `winmm.dll`. The proxy build uses
`SF_BOOTSTRAP_IMPORT_DIR` to select that library, with no dynamic winmm fallback.
`build_windows_proxies.ps1 -StandaloneEml` builds the featureless EML proxies
without a winmm dependency. Featureless proxies refuse an SF selection.

SF winmm records its static process attachment and stays dormant. After selecting
SF, the proxy calls the private `ShroudforgeStart(module, reserved)` C export from
its actual process-attach callback. The host requires its own static attachment,
the same attach thread, a non-null proxy attachment argument, a supported game
process, and a root dbghelp/dinput8 module. These arguments are a trusted internal
native contract, not a security boundary against other native code in-process.
Static initialization may be transitive; it does not prove an EXE imports winmm.
Dynamic winmm attachment in the supported game process fails before gate work.

The host admits startup once, caching success or failure. Reentry refuses while
startup is in progress. A second valid proxy receives the completed result and
cannot install another gate or worker. Importing winmm with EML selected leaves
the gate untouched. The existing entrypoint gate and its held-entry proof remain
the runtime preparation boundary; `ShroudforgeStart` does not claim runtime or
World readiness.

Any prepared marker requires a readable JSON policy of at most 1 MiB with a
supported integer `schemaVersion`. Versions 1 through 5 use the separately named
historical assets bootstrap; version 6 uses static startup. Invalid, unsupported,
missing or unreadable policies refuse without falling through to EML. This
bridge only discriminates routes: full policy, composition and receipt validation
remain in the runtime. With no markers, the existing configured backend selection
is preserved. Schema 6 still requires its new producer closure and explicit
full-runtime admission; historical assets receipts do not authorize it.

The planned package carrier is dbghelp. Producing both proxy variants does not
authorize installing both or replacing another module's dinput8. Actual game
startup, prepared composition, native feature support and installation are
separate qualification steps.
