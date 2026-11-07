# Your first plugin in 10 minutes

Build a server-only Rust component that logs when enabled, greets joining players
and answers `/hello`. Allow extra time for the first Rust dependency download and
server content installation. Use a Solaris checkout matching the server: SDK and
WIT are generated together and 0.8.0 components must match that server's WIT.

## Prerequisites

Install Rust and Cargo with rustup, Git, and a built `solaris-v2` server with its
vanilla content cache ([server setup](../OPERATING.md)). No `cargo-component` or
`wasm-tools` is needed: the repository's `plugin_component` example encodes the
component. The first encoder build compiles host dependencies; it is not the
server. Commands below use a POSIX shell. Set `SOLARIS` to the **absolute path**
of your matching checkout and `SERVER` to your built server binary; keep both
available throughout. Use an isolated test server and world, not production.

```sh
export SOLARIS="$(pwd)"
export SERVER="$SOLARIS/target/release/solaris-v2"
rustup target add wasm32-unknown-unknown
mkdir -p /tmp/solaris-plugin-tutorial
cd /tmp/solaris-plugin-tutorial
cargo new --lib hello
```

Run the first two exports from the Solaris checkout; if your server is elsewhere,
set `SERVER` to its absolute path instead. `hello` must not already exist in the
tutorial directory. Replace `hello/Cargo.toml` with:

```sh
cat > hello/Cargo.toml <<EOF
[package]
name = "hello"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
solaris-plugin-sdk = { path = "$SOLARIS/sdk/rust/solaris-plugin-sdk" }
wit-bindgen = "0.62"
EOF
```

Write `hello/src/lib.rs`:

```sh
cat > hello/src/lib.rs <<'EOF'
use solaris_plugin_sdk::{
    export_plugin, log, message_player, Command, Config, Event, EventContext,
    Failure, InitContext, LogLevel, Plugin,
};

#[derive(Default)]
struct Hello;

impl Plugin for Hello {
    fn init(&mut self, _config: &Config, _context: &InitContext) -> Result<Vec<Command>, Failure> {
        log(LogLevel::Info, "hello plugin ready");
        Ok(Vec::new())
    }

    fn on_events(&mut self, _context: &EventContext, events: &[Event]) -> Result<Vec<Command>, Failure> {
        let mut commands = Vec::new();
        for event in events {
            match event {
                Event::PlayerJoined(joined) => commands.push(message_player(
                    &joined.player, format!("Welcome! {}", joined.name),
                )),
                Event::CommandInvoked(invoked) if invoked.name == "hello" => commands.push(
                    message_player(&invoked.player, "Hello from a WASM plugin."),
                ),
                _ => {}
            }
        }
        Ok(commands)
    }
}

export_plugin!(Hello);
EOF
```

Create the deployment manifest. `player.joined` is a subscription; command
invocations are routed because the manifest owns `hello`. No capability is
needed for these messages. A console does not dispatch player plugin commands.

```sh
mkdir -p plugins/hello
cat > plugins/hello/plugin.toml <<'EOF'
id = "hello"
name = "Hello"
version = "0.1.0"
api = "0.8.0"
events = ["player.joined"]
player_commands = ["hello"]
EOF
```

Build the core module and encode it as a component (the server cannot load the
unencoded module):

```sh
cargo build --manifest-path hello/Cargo.toml --target wasm32-unknown-unknown --release
cargo run --manifest-path "$SOLARIS/crates/mc-plugin-host/Cargo.toml" --example plugin_component -- \
  "$PWD/hello/target/wasm32-unknown-unknown/release/hello.wasm" "$PWD/plugins/hello/plugin.wasm"
```

Caches and compilation can take longer than ten minutes on the first run.

For a local test, create this isolated configuration. It uses the checkout's
licensed content cache, a new world, loopback and offline authentication. If
your cache is elsewhere, change `vanilla_data_dir` to its absolute path; see
[content setup](../OPERATING.md#vanilla-content-cache).

```sh
cat > server.toml <<EOF
[network]
bind_address = "127.0.0.1"
port = 25565
[auth]
online_mode = false
[plugins]
directory = "/tmp/solaris-plugin-tutorial/plugins"
[data]
world_dir = "/tmp/solaris-plugin-tutorial/world"
vanilla_data_dir = "$SOLARIS/data/vanilla"
EOF
```

Start it:

```sh
"$SERVER" --config /tmp/solaris-plugin-tutorial/server.toml
```

Look for `hello plugin ready`. Join with a compatible player client: chat shows
`Welcome! <name>`. Send `/hello` as that player and see `Hello from a WASM plugin.`
Use offline mode only on a private test server; it does not authenticate players.

## If it does not work

- `can't find crate for std`: install the target with the rustup command above.
- SDK path or WIT missing: use the complete matching checkout, not a copied SDK
  directory; the SDK generates bindings from `crates/mc-script/wit`.
- Invalid component or wrong world: encode the core module, and rebuild SDK,
  component and server from matching WIT versions.
- Duplicate id or command, unknown manifest field, capability or event: remove
  the conflict or fix `plugin.toml`; startup fails closed, it does not skip it.
- `/hello` in console is not a player command; use a connected player. If it
  still fails, inspect server logs for a trapped or retired component and restart
  after fixing it. See [deployment diagnostics](OPERATIONS.md#diagnosing-a-deployment).
