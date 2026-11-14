# Plugins

Krasnokamen plugins are WebAssembly components implementing `krasnokamen:plugin@0.9.0`,
not Bukkit/Paper, Forge or Fabric JARs. The server loads components; it does
not compile their source. Vanilla clients can use server-only plugins.

## Players and operators

- [Deploy, configure, protect zones, manage graves and diagnose](plugins/OPERATIONS.md).
- [Install the optional Krasnokamen Loader](KRASNOKAMEN_LOADER.md) for client bundles.

## Plugin developers

- [Your first plugin in 10 minutes](plugins/QUICKSTART.md).
- [SDK reference and limitations](plugins/SDK.md), [Rust SDK](../sdk/rust/krasnokamen-plugin-sdk/README.md) and the
  [manifest and lifecycle contract](plugins/CONTRACT.md).
- Rust examples: [hello](../sdk/rust/examples/hello) and
  [sky-creeper](../sdk/rust/examples/sky-creeper).
- Real packages: [krasnokamen-default-plugins](https://github.com/kaiserproger/krasnokamen-default-plugins).
- [Experimental offline Bukkit/Forge converter](plugins/CONVERTER.md): a small,
  explicitly bounded subset, not general compatibility.
