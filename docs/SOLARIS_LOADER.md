# Install Solaris Loader on a client

Solaris Loader is an optional **client-side** mod for Solaris plugins that ship
verified UI (screens/HUD), assets, sounds, items, blocks, or UI/keyboard
interactions. It is not needed when every selected server plugin is
`server_only`.

## Current target

| | |
|---|---|
| Loader version | `0.1.0` (jars are named `loader-<platform>-0.1.0.jar`) |
| Minecraft | Java Edition **26.1.2**, exactly; Java **25** |
| Loader wire protocol | **3**, with client artifact index **schema 2** |
| Server plugin API | WASM component API `solaris:plugin@0.8.0` only |

These values are read from the `solaris-loader` repository (`gradle.properties`,
its README). Its Git history and release version are independent of core: a
server tag is not a Loader revision, and the alpha protocol and bundle contracts
can change between revisions (there is no protocol-1/2 compatibility path;
schema-1 bundles and the removed UI/interaction content kinds fail closed). The
plugin API version does not change the wire protocol or artifact schema.

This documented Loader target is **26.1.2**. The server's supported window
includes 26.1.2, 26.2 and canonical 26.3; content import installs their endpoint
caches and adjacent translation steps automatically. A missing endpoint cache
disables that release, not other available clients. See
[client protocol compatibility](OPERATING.md#client-protocol-compatibility).
Install the Loader only in a Minecraft 26.1.2 instance; do not use a nearby
Minecraft version.

## Compatibility matrix

The repository builds and tests against:

| Client platform | Baseline for 26.1.2 | Solaris jar |
| --- | --- | --- |
| Fabric | Fabric Loader `0.19.3`, Fabric API `0.155.2+26.1.2` | `loader-fabric-0.1.0.jar` |
| NeoForge | NeoForge `26.1.2.76` | `loader-neoforge-0.1.0.jar` |
| Forge | Forge `26.1.2-64.1.0` | `loader-forge-0.1.0.jar` |

The mod metadata requires Minecraft 26.1.2. Fabric also requires Fabric API and
Fabric Loader 0.19.3 or newer; NeoForge requires 26.1.2.76 or newer within the
26.1.2 line; Forge requires loader major 64 within the 26.1 line. The exact
versions above are the field-tested matrix; other builds are not a release claim.

## Download or build the client jars

Download the matching platform JAR and `SHA256SUMS` from
[Solaris Loader releases](https://github.com/kaiserproger/solaris-loader/releases)
(not GitHub's source ZIP) and verify the JAR's SHA-256 before installing. The
server's curl installer does not install client mods. To build instead, from a
`solaris-loader` checkout:

```sh
./gradlew --no-configuration-cache :loader-fabric:jar :loader-neoforge:jar :loader-forge:jar
```

and use only the jar for the chosen platform
(`loader-<platform>/build/libs/loader-<platform>-0.1.0.jar`). These are client
mods, not server plugins: never put them in the server's `[plugins].directory`.

## Install

1. Close Minecraft. Create or select a **26.1.2** instance for exactly one
   platform (Fabric, NeoForge or Forge) using Java 25; use the baselines above to
   reproduce a field test. Fabric also needs the Fabric API mod.
2. Open **that instance's game directory** and its `mods/` folder (create it if
   needed) and copy the one matching Solaris Loader jar there. Remove any older
   Solaris Loader jar from the same instance so exactly one remains. Do not
   unzip it, and do not use the Fabric jar in a NeoForge or Forge profile (or
   NeoForge's in Forge's: their networking adapters differ although both use TOML
   metadata).
3. Start that same instance, connect to the server and review the permission
   prompt.

| Launcher | Where to place the JAR |
| --- | --- |
| PrismLauncher | Edit the instance, then Mods (or the instance's `minecraft/mods/` folder); choose the mod loader under Version. |
| MultiMC | Edit the instance, then Loader mods; use only a loader/version the launcher offers. |
| Official Minecraft Launcher | Install the chosen platform's client profile with its installer, then use that installation's configured Game Directory and `mods/`. |
| Modrinth App | The matching instance's folder, `mods/`. |
| CurseForge App | The matching custom profile's folder, `mods/`; do not replace a managed pack's loader. |
| Other launchers | The launched profile's game directory, not the launcher's application directory or a global `.minecraft`. |

Installing in another instance has no effect. These are manual instructions, not
graphical validation of every launcher. Loader needs no MCP token or development
client agent.

## First connection and permissions

When the server requires Loader content, it sends a manifest during the
Configuration phase. On the first connection for an exact normalized server
address and requested permission set, Loader opens a confirmation screen:

- **Allow:** Loader requests missing exact bundle identities, verifies each
  declared size and SHA-256, publishes them atomically in the cache, activates
  the permitted content, then completes login.
- **Deny:** nothing is requested or staged and the connection closes without
  acknowledgement.

A previous decision is reused only for that address and exact permission set; a
different address or changed permissions prompt again. The default cache is
`~/.solaris/loader-cache/` (`permissions.properties` plus content-addressed
bundle data); `-Dsolaris.loader.cacheDir=/absolute/path` overrides it for an
isolated profile. Do not copy a permission file between untrusted server
addresses; cached bytes are still checked against the server manifest before
activation. The server allows up to two minutes for this confirmation and
transfer. Loader clears activated registries, HUD state and transient resources
when the connection closes.

What bundles can use: declared keyboard actions leave vanilla movement, menus,
screenshots and fullscreen handling intact (suppressed in menus and without
window focus, reset on reconnect; no rebinding, chords or mouse/gamepad actions).
Schema-2 overlays may declare `view_request_bindings: [{key, view}]`; the key
sends the additive `view_request` message and the server accepts it only for a
connection that completed Loader negotiation, from an owner with the `views` and
`present_views` capabilities. The native settlements bundle is appended to an
already-required plugin Loader manifest and never makes Loader mandatory by
itself; its overview is bound to <kbd>C</kbd> and opens the same dialog as
`/settlement menu`. Sound bundles request `play_sounds` with `load_assets` and
support personal and world-positioned one-shots with volume and pitch; loops and
moving sources are not implemented.

## Required-client behavior

A server with no client bundles sends no Loader payload and works with a vanilla
client. If any selected plugin declares `[client].bundles`, the deployment is
`server_and_client` and Loader is required. A missing Loader, wrong
platform/version, denied permission, missing acknowledgement, invalid bundle,
size/hash mismatch or activation failure stops login during Configuration; the
disconnect reason names the supported platforms and required bundle identities
where available. Solaris never silently substitutes vanilla content.

Operators find out which packages need Loader from each package's validated
`[client]` declarations (supported loaders, permissions, artifact identities,
paths, sizes, hashes) and the actual Configuration negotiation. There is no
native server `--check` report; startup validates the real component and client
deployment, and `content verify` covers only the vanilla cache. See
[plugin deployment](plugins/OPERATIONS.md#operator-deployment).

## Troubleshooting

- **"Loader is required":** confirm the jar is in the `mods/` of the instance
  actually launched, that it reports Minecraft 26.1.2, that the platform matches,
  and (Fabric) that Fabric API is installed; check the client log for
  `solaris_loader` loading.
- **No permission prompt:** a stored decision may already match, or the plugin
  declares no client bundles. Close Minecraft and remove only the entry for that
  server from `permissions.properties` to prompt again.
- **Prompt denied:** reconnect and change the stored decision for that exact
  server only after reviewing the permissions with the operator.
- **Bundle verification or activation fails:** the operator verifies each
  artifact exists and matches `size_bytes` and `sha256`; the client does not edit
  cached ZIPs. Changing artifact bytes needs a new matching manifest identity;
  remove the failing cache identity and reconnect.
- **Platform/version mismatch:** use the matrix above and never rename one
  platform's jar. Report both the client log and the disconnect reason.

## Developer compatibility gate

The repository includes a Loader-required fixture and isolated real-client checks:

```sh
python3 -m tools.harness run loader-live --platform fabric
python3 -m tools.harness run loader-live --platform neoforge
python3 -m tools.harness run loader-live --platform forge
```

See [`examples/loader-live-gate/README.md`](../examples/loader-live-gate/README.md)
for the fixture. These gates are not an end-user installer.
