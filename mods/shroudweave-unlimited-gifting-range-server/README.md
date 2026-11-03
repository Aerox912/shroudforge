# Shroudweave Unlimited Gifting Range Server

This server-only ShroudForge package supplies the dedicated-server side of the
existing XHL Unlimited Gifting Range selection. Keep the original client DLL
and its profile unchanged. Enable this package only on the dedicated server
when that Unlimited Gifting Range feature is selected.

The adapter changes one verified gifting-range callsite in the dedicated
server's mapped main module. It checks the PE headers, section and unwind
tables, exact caller/getter code digests, and the call target. It does not gate
on a whole-file executable hash, timestamp, or profile-pinned image size. It is
disabled by default, and a server restart is required after changing its
selection. If the server target, mapped-image structure, guarded code,
provider initialization, or held-entry proof does not match, the package keeps
the game's stock range and reports the feature unavailable.

This is startup-only behavior. It does not provide a hotkey or runtime toggle.
