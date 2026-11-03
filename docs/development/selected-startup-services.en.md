# Selected startup services

Schema 6 uses runtime contract version 2, host
`sf-winmm-selected-services-v2`. Historical schemas 3–5 retain their assets-only
routes. Old full-runtime version-1 handshakes refuse this contract. Production
producer/runtime closures are supplied after the final composition freeze;
synthetic test values do not authorize a release.

`bootServices` is an ordered, unique list of these exact names:

| Service | Required operation |
| --- | --- |
| `metadata-init-v1` | ECS metadata, resource and menu initialization; always required |
| `required-engine-hooks-v1` | Actual qualified generic execution hooks |
| `map-capture-preparation-v1` | Held-entry, inactive guarded Map preparation |
| `first-person-preparation-v1` | Held-entry FP preparation after Map |
| `graphics-boot-v1` | Requested graphics broker preparation |
| `vein-flight-provider-v1` | Shared native provider preparation, initially Vein disabled |
| `flight-scalar-preparation-v2` | Actual bound Flight scalar SDK/profile preparation |
| `vein-activation-v1` | Actual selected Vein activation after releasing its input lease |

Metadata initialization is not a World, player, read-view or gameplay readiness
claim. It neither loads a generic execution profile nor installs its hooks.
`KfcRuntimeStartupReady` retains its existing actual generic-hook meaning. Both
generic ECS write admission points also enforce selected execution demand and
actual hook readiness; an external direct-write profile cannot bypass them.
Legacy no-argument initialization retains its original behavior. A selected
resident startup cannot be upgraded by calling that legacy initializer later.

Demand is checked against the admitted canonical runtime/DLL plan and effective
settings. FP requires its Map preparation prerequisite. An enabled approved
managed plugin declaring `shroudforge.map` requires Map preparation, while its
overlay declaration determines graphics demand. Selected SF Flight settings
must contain all four actual finite percentages in the supported 25–400 range:
`turnSpeed`, `moveSpeed`, `liftSpeed`, `descendSpeed`. Any non-100 value requires
generic execution hooks. Missing, malformed or unresolved values refuse startup;
they never become 100. F6 demand follows the actual `unlimitedFlying` selection.

`selection` binds explicit `processRole`, `dedicatedGuestQualified`,
`allowUnverifiedCursorGate`, `flightTransportEnabled`, `veinSelected` and
`veinSettingsPath`. Missing opt-ins are false. The current managed-library graph
supports the client role; no dedicated-guest qualification is inferred from
Steam peers. Requested Flight/Vein native operations require the appropriate
explicit opt-ins. Selected Vein requires the existing canonical
`mods/shroudweave-vein-mining-test.ini`, with bytes bound in the composition.
There is no alternate KFC INI or new settings JSON. Flight-only preparation does
not create an A6 consumer/worker. The private scalar SDK must also be explicitly
bound when Flight is requested.

The private 24-byte `SelectedBootPlanV2` transports the exact service mask, role
and opt-ins. Take is one-shot and root-bound. Native prepare consumes the still
leased inputs and returns exactly the prepared mask, excluding Vein activation.
The Rust Taken operation then releases only the selected admitted INI handles.
Differently named owned input aliases of that file refuse admission, and an
unselected INI lease stays held. Other metadata/settings/script/native leases
retain their existing through-create lifetime.

Native activation checks the same immutable prepared plan, calls the actual
Vein activation when selected, and reports the exact final mask. Only then may
the full-runtime commit release the entrypoint once. Missing exports, unknown
bits, changed plans, incomplete masks or selected feature failures stop startup
without updater, backup, copying or legacy fallback. No World wait occurs behind
the entrypoint barrier. Partial failures keep callback modules resident.

Live changes that need unprepared generic hooks or unprepared F6 services are
reported as unsupported/restart-required before applying the new runtime
settings. Saved values are retained. Speed controls keep their shared glider
resource scope; this contract does not turn them into personal coefficients.

Final retirement stops Flight before its shared provider, then FP before Map
logical retirement and graphics/ECS shutdown. Original callback bridges remain
resident. Final Map profile/body, selected binaries, producer receipt membership
and full installed startup still require their own accepted composition evidence.
