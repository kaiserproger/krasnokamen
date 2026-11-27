# Plugins

Krasnokamen plugins are WebAssembly components implementing `krasnokamen:plugin@0.9.0`,
not Bukkit/Paper, Forge or Fabric JARs: those do not run here, and there is no
runtime compatibility layer. The server loads components; it does not compile their
source. Vanilla clients can use server-only plugins.

Before you start: the plugin API is pre-1.0. A change to its WIT can require
rebuilding every plugin, so build against the checkout matching your server.
Each plugin runs in its own sandbox with only the capabilities its manifest
declares, and its callbacks run one at a time
([callback limits and backlog](plugins/OPERATIONS.md#callback-limits-and-backlog)).
Callbacks run outside the world tick; native owners validate requested changes
against current state when applied, so a request can become stale or be refused.

## Players and operators

- [Deploy, configure, protect zones, manage graves and diagnose](plugins/OPERATIONS.md).
- [Install the optional Krasnokamen Loader](KRASNOKAMEN_LOADER.md) for client bundles.

## Plugin developers

- [Your first plugin](plugins/QUICKSTART.md): a minimal `/hello` template, build and deployment.
- [SDK reference and limitations](plugins/SDK.md), [Rust SDK](../sdk/rust/krasnokamen-plugin-sdk/README.md) and the
  [manifest and lifecycle contract](plugins/CONTRACT.md).
- Rust examples: [first-plugin](../sdk/rust/examples/first-plugin) (one command),
  [hello](../sdk/rust/examples/hello) (host test fixture) and
  [sky-creeper](../sdk/rust/examples/sky-creeper).
- Real packages: [krasnokamen-default-plugins](https://github.com/kaiserproger/krasnokamen-default-plugins).
- [Experimental offline Bukkit/Forge converter](plugins/CONVERTER.md): a small,
  explicitly bounded subset, not general compatibility.
