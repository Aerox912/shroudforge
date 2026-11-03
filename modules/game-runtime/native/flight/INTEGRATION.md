# Bundled Flight private integration

The SF bundled mod is the sole controller. No separate Flight Mod or Shroudtopia
host context is loaded. The existing XHL companion retains server authority.

RB11 host owns startup and final stop. Before callbacks, construct
`VeinFlight::ProviderConfig` with actual process role, explicit qualified
dedicated-guest demand, the two reviewed opt-ins (absence is false), and the
admitted canonical existing Vein INI path. Call `prepare_provider` once. Select
Vein separately through `set_vein_enabled`; Vein failure/Off must leave Flight
available. Tick is optional because the resident native worker owns input.

Final host retirement calls `shroudforge::flight::stop_client()` **before**
`VeinFlight::stop_provider()`. Stop closes admission, keeps originals/callbacks
pinned, and requires process restart. No dynamic unload, retry or shared MinHook
uninitialize is supported. Partial starts retain owned hooks and their originals.
An invalid Vein INI disables only Vein if Flight transport remains selected.

Flight optional ABI12 is token-bound. Rust obtains the generation from the
actual registered mod identity; scripts supply no token. Only
`sf-unlimited-flight` may call the Lua controller. Owner retirement sends bound
Off and clears local pitch selection without retiring the shared provider.
The exact reset-aware V2 publisher binds directly inside this resident module,
refreshes On every100ms and never falls back to local-only activation.

The generic Scalar SDK remains a private pinned sibling of `kfc-runtime.dll`:
`shroudweave-native-scalar-sdk.dll`,32256bytes,
SHA256 `7398a3fb255e5b0aace3a106aec49dbdd70f5972a08edfff899bfb0138708892`.
Source commits contain only its public C headers and compiled hash, not binary.
The loader rechecks exact path/hash/API and retains dependency residency.

Stamina is excluded. Speed settings retain their original keys/values and shared
glider resource scope. Never present them as personal coefficients.

Open acceptance: this adapts the reviewed local-subject pitch-bound callback.
SF signed-rate, neutral and inverted-recovery parity is **not established**;
SP/listen are unqualified. Combined startup, loader graph, two-player isolation,
reset/reconnect and performance need Main's complete build and Tester review,
then authorized gameplay. No installation is authorized by these files.
