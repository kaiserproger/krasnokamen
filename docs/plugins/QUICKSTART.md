# Your first plugin

Build a server-only Rust component that answers `/hello`. Start from the small
[first-plugin template](../../sdk/rust/examples/first-plugin), not the much larger
`hello` host-test fixture. No API change or code generator is needed.

## Prerequisites

Install Rust and Cargo with rustup, Git, and a prebuilt `krasnokamen` server with
its content cache ([server setup](../OPERATING.md#startup-and-content)). Use a
complete checkout matching the server: SDK and WIT are generated together;
`krasnokamen:plugin@0.9.0` alone does not guarantee binary compatibility. Published
rc.5 artifacts predate the rename; do not mix their SDK/plugins with current main.

No `cargo-component` or `wasm-tools` is needed. The repository's `plugin_component`
example encodes the component; its first build compiles host dependencies, **not
the server**. Allow time for dependency downloads and compilation; this guide
makes no ten-minute first-build guarantee.

Commands use a POSIX shell. From your matching checkout, set `KRASNOKAMEN`, then
set `SERVER` to your matching prebuilt binary and `CONTENT` to an installed
canonical cache. Keep this shell open:

```sh
export KRASNOKAMEN="$(pwd)"
export SERVER=/absolute/path/to/krasnokamen
export CONTENT=/absolute/path/to/content/26.3
rustup target add wasm32-unknown-unknown
export TUTORIAL="$(mktemp -d "${TMPDIR:-/tmp}/krasnokamen-plugin-tutorial.XXXXXX")"
export CARGO_TARGET_DIR="$TUTORIAL/target"
mkdir -p "$TUTORIAL/plugins/hello"
cp "$KRASNOKAMEN/sdk/rust/examples/first-plugin/plugin.toml" "$TUTORIAL/plugins/hello/"
```

## Build and deploy

The template has three files: `Cargo.toml`, `src/lib.rs` and `plugin.toml`.
Read `src/lib.rs`: it returns a message for the session that invoked `/hello`,
not for a UUID that may belong to a new session after reconnect. Edit the reply
there to make your first change. `plugin.toml` owns the `hello` command root;
no subscription or capability is needed for this message.

```sh
cargo build --manifest-path "$KRASNOKAMEN/sdk/rust/Cargo.toml" \
  -p first-plugin --target wasm32-unknown-unknown --release
cargo run --manifest-path "$KRASNOKAMEN/crates/mc-plugin-host/Cargo.toml" \
  --example plugin_component -- \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/first_plugin.wasm" \
  "$TUTORIAL/plugins/hello/plugin.wasm"
```

Build the guest with `--release` every time, also while iterating: a debug
component is about 22 MB, and the server refuses any plugin artifact over
16 MiB (16,777,216 bytes) at startup. The release component is about 0.5 MB.
`--release` here compiles only your plugin, not the server. Setting the output
directory explicitly makes the encoder's input unambiguous, even if your shell
already had `CARGO_TARGET_DIR` set. Deploy the **encoded component**, not the
unencoded module. Never copy Cargo source files into a deployed package directory.

## Run and observe

Create an isolated loopback/offline server, with a new world. Set another free
port if 25565 is in use. All data and plugin paths below are absolute: relative
paths are resolved against the process working directory, not the config file.

```sh
cat > "$TUTORIAL/server.toml" <<EOF
[network]
bind_address = "127.0.0.1"
port = 25565
[auth]
online_mode = false
[plugins]
directory = "$TUTORIAL/plugins"
[data]
world_dir = "$TUTORIAL/world"
vanilla_data_dir = "$CONTENT"
EOF
"$SERVER" --config "$TUTORIAL/server.toml"
```

Join with a compatible player client and send `/hello`. The expected chat reply
is `Hello from a WASM plugin.` (or your edited text). A listening socket or a
successful build alone does not prove the command works. The console does not
dispatch player plugin commands. Use `stop` in the console to shut down cleanly.
After editing the plugin, stop, rebuild, encode and restart before checking again.
Offline mode does not authenticate players; keep this test on loopback.

## If it does not work

- `can't find crate for std`: install the target with the rustup command above.
- `carries a …-byte artifact, the bound is 16777216`: you deployed a debug
  build; rebuild with `--release` and encode the release module again.
- SDK path or WIT missing: use the complete matching checkout; the SDK generates
  bindings from `crates/mc-script/wit`.
- Invalid component or wrong world: encode the core module and use matching SDK,
  WIT and server revisions.
- Duplicate id/command, unknown manifest field, capability or event: fix the
  manifest conflict; startup fails closed rather than skipping the package.
- `/hello` fails: use a connected player, not the console. If an authentication
  plugin is deployed, log in first. Inspect logs for trapped or retired components
  and restart after fixing them. See [deployment diagnostics](OPERATIONS.md#diagnosing-a-deployment).

For joins, timers and durable operations, continue with the [SDK reference](SDK.md)
and the existing examples. This template deliberately implements only one command.
