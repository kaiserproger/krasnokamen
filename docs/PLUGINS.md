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

## Slow callbacks

One host thread delivers every durable event to every plugin, one callback at a
time, and a callback may run up to its fuel or wall-clock limit. Until the host
takes them, durable events (player commands, operation results, menu clicks,
graves) wait in the core, bounded at 1024 entries (events, player changes and
execution traces). While a quarter of that bound (256 entries) is waiting, the
core refuses new plugin commands: the player is told the plugin is backed up and
the command is not queued. A menu click in that state is dropped the same way
(the menu is redrawn, the plugin never sees the click). A refused command or
click is never delivered; the player retries. Both are accepted again once the
host catches up. Observations (joins, chat, block and kill events) are shed per
plugin and never count toward this bound. If the bound itself is passed by
events that cannot be refused (graves, guest command results), the core halts
the world fail closed and logs the backlog by event kind under the
`plugin.backlog` target.
