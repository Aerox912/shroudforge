# Shroudweave First Person

Shroudweave First Person is a client-only ShroudForge mod. When enabled at client startup, it starts in third person. Press the configured toggle to switch the first-person camera and corrected stamina-gauge position together.

## Controls and settings

- **Toggle First Person:** Home by default. Change it in the mod's keybind setting.
- **Camera Distance:** camera offset while first person is active; default `-0.15`, range `-0.5` to `0.5`, step `0.01`.

Enable the mod before launching the client so startup can prepare its native hooks. If you first enable it after the client session is running, restart the client. Once prepared, the toggle and camera-distance setting apply live. Disabling or unloading the mod turns off both effects. Its native forwarding stubs remain resident until the client process exits; this is not a hot-unload.

## Compatibility status

This package targets the player client only and has no dedicated-server component. Multiplayer behavior has not been verified.

The current static profile admits only the client image with SHA-256 `0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6` (PE timestamp `0x6ae4601e`, image size `0x2a40000`). The `0403` prefix is part of that file hash, not a game-version number. Other client images are refused. Static profile checks and independent static native-source/fixture review passed. The combined provider/host build, Minimap composition, live SDL input/settings behavior, and in-game behavior remain unverified.

## Credits and license

Based on **First Person View** by s0T7x (Miguel Oppermann). The original MIT copyright and license are preserved in [`LICENSE`](LICENSE).