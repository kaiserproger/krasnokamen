# Solaris Rust plugin SDK

This crate implements the guest side of `solaris:plugin@0.8.0`. It generates
bindings from the WIT in the same Solaris checkout; keep SDK, WIT and server
versions matched. It is part of the independent `sdk/rust` workspace.

- [Build your first plugin](../../../docs/plugins/QUICKSTART.md): complete Cargo
  project, manifest, join event, command, component encoding and deployment.
- [SDK reference](../../../docs/plugins/SDK.md): supported APIs, capabilities,
  sandbox, persistence and explicitly missing Bukkit/Paper surfaces.
- [Manifest and lifecycle contract](../../../docs/plugins/CONTRACT.md).
- [Hello](../examples/hello/src/lib.rs) is also a host test fixture with extra
  modes; [sky-creeper](../examples/sky-creeper/src/lib.rs) demonstrates a native
  mob blueprint.

Implement `Plugin` on a `Default` type and invoke `export_plugin!(YourType)`.
`configure` and `init` run in separate stores; return commands from callbacks
and handle their typed results instead of assuming direct effects. Deploy an
encoded WebAssembly component, not the core wasm module.
