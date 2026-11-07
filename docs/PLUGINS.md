# Plugins

Solaris plugins are WebAssembly components implementing `solaris:plugin@0.8.0`,
not Bukkit/Paper, Forge or Fabric JARs. The server loads components; it does
not compile their source. Vanilla clients can use server-only plugins.

## Players and operators

- [Deploy, configure, protect zones, manage graves and diagnose](plugins/OPERATIONS.md).
- [Install the optional Solaris Loader](SOLARIS_LOADER.md) for client bundles.

## Plugin developers

- [Your first plugin in 10 minutes](plugins/QUICKSTART.md).
- [SDK reference and limitations](plugins/SDK.md), [Rust SDK](../sdk/rust/solaris-plugin-sdk/README.md) and the
  [manifest and lifecycle contract](plugins/CONTRACT.md).
- Rust examples: [hello](../sdk/rust/examples/hello) and
  [sky-creeper](../sdk/rust/examples/sky-creeper).
- Real packages: [solaris-default-plugins](https://github.com/kaiserproger/solaris-default-plugins).
- [Experimental offline Bukkit/Forge converter](plugins/CONVERTER.md): a small,
  explicitly bounded subset, not general compatibility.
