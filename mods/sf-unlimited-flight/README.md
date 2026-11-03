# SF Unlimited Flight

Independent ShroudForge runtime mod by bonsaibauer. It has no mod dependencies.
The mod is disabled by default. Fresh settings keep Upward Gliding and Unlimited
Glider Stamina Off; the four speed multipliers start at 100%.

## Controls

F6 toggles Upward Gliding for the current runtime session. Its default SDL
binding is `SDL:63`; reassign it under Compatibility → Hotkeys. A saved
Upward Gliding setting change takes precedence over the temporary toggle.
Each F6 transition requests a three-second On/Off popup. The local selection
and a successful intent reply do not confirm that native movement was applied.
Unavailable server replies report that limitation; actual in-game popup display
has not been verified for this offline candidate.

Turn Speed adjusts yaw rate, Move Speed adjusts forward acceleration, and Lift
Speed and Descend Speed adjust glider air resistance. Values above 100% increase
the selected coefficient.

## Local runtime behavior

The legacy Lua path applies checked overrides to the glider resources visible to
that runtime. Glider `ItemInfo` values are shared by every user of the same
glider resource, so this path is not per-player multiplayer Flight. Upward tilt
requires one player in the authoritative world. The optional stamina setting
belongs to this local path.

Historical local-game reports describe upward flight in an earlier performance
build and a working installed R2 build. Those observations predate the current
100 ms update interval and do not verify the native server callback or current
multiplayer behavior.

## Dedicated-server status

F6 can send a reliable, peer-specific request to a connected Dedicated Server.
The server validates the transport peer, message envelope, and request
sequence. The candidate native callback selects only that player's glider pitch
operand. It does not change shared glider flight values or stamina, even if a
saved server setting enables Unlimited Glider Stamina. Height hold is not
implemented; the game's positive-pitch recovery remains stock.

Native admission starts Unprepared. The supported dedicated-server profile can
become Ready only through successful early-bootstrap held-entry preparation.
FLIGHT75 accepted the bounded production-admission work. WORLD91 R2 subsequently
accepted the shared World callback and readiness-publication repair: preparation
cannot publish Ready before the coordinator's final proof check succeeds.
Failure after publication retains original-call forwarding but permanently
refuses callback readers. Service teardown drains short reader leases before
the native data can be freed.

Offline checks cover two independent player intents, full actor-key matching,
Off preserving stock pitch, retirement during a callback read, and original-call
forwarding. The common World hook survives Flight Off or scalar-hook failure;
it carries no player Flight effect by itself. No new game executable or saved
server state is modified by these offline checks.

The retirement repair deliberately stays unavailable for the rest of the process
after it observes alternate service teardown. Starting another world or session
does not reopen it; native per-peer support would require a process restart.
On reports unavailable in that state, while Off can still clear the intent.

The Release native provider was built, and its actual copied DLL was checked in
a private non-game bootstrap host. Independent WORLD91 R2 review verified the
bounded delta with owned inert memory and helper calls, not a second provider
rebuild. Neither check proves execution in a GameServer. The client's separate
five-degree pitch clamp and prediction behavior remain unresolved; full
multiplayer Flight and IKEA readiness are unverified. Single-player and hosted
multiplayer are not validated by the dedicated-server native profile. The final
combined host must incorporate these reviewed sources and pass its own assembly
checks; the Flight-only provider artifact is dependency evidence, not that final
host. Keep Unlimited Glider Stamina Off for this candidate on all runtimes.

## Level-flight offline checkpoint (31 October 2026)

The offline operand core passed interval, stack-frame, selector, and 18 archived
arithmetic fixtures. It selects neutral `gliderY` and the later gravity `Y`
contribution while preserving the original add and other vertical forces. The
selectors are not connected to the native callback or a provider, so native
level flight is not ready for IKEA.

Native integration remains blocked on canonical `Binding` and generation,
query/chunk lifetime and writer exclusion, the producer-to-gravity scheduler
dependency and shared simulation-step identity, and typed PlayerInput mode and
ActorInput axis decoding from validated component slots. The client's pitch
clamp, prediction, and horizontal-orientation behavior also remain unresolved.
Keep Unlimited Glider Stamina Off for this checkpoint.

## Credits and provenance

The mod manifest credits bonsaibauer, links to the [ShroudForge source
repository](https://github.com/bonsaibauer/shroudforge), and does not declare a
license. SF Unlimited Flight uses the ShroudForge runtime API; it is an
independent community mod, not an official Keen Games product.
