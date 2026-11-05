# Windows and macOS client QA

The QA protocol requires every advertised OS to be attacked
([QA_AGENT_PROTOCOL.md](../QA_AGENT_PROTOCOL.md), "Every advertised variant").
On the owner's Linux machine Windows runs in a local VM; macOS has no local
procedure.

## Windows 11 VM (dockur)

A QEMU/KVM Windows 11 guest in Docker (`dockurr/windows`). Needs `/dev/kvm`
(user in the `kvm` group) and Docker; no VirtualBox.

Create once:

```sh
mkdir -p ~/winvm/storage ~/winvm/shared
docker create --name solaris-win --dns 1.1.1.1 --dns 8.8.8.8 \
  --device=/dev/kvm --device=/dev/net/tun --cap-add NET_ADMIN \
  -p 127.0.0.1:8006:8006 -p 127.0.0.1:13389:3389/tcp -p 127.0.0.1:13389:3389/udp \
  -e VERSION=11 -e RAM_SIZE=8G -e CPU_CORES=4 -e DISK_SIZE=64G \
  -e USERNAME=<vm-user> -e PASSWORD=<vm-password> \
  -v ~/winvm/storage:/storage -v ~/winvm/shared:/data \
  --stop-timeout 120 dockurr/windows
docker start solaris-win
```

- `docker run -d` with the device flags failed silently: create, then start.
- Without explicit `--dns` the container could not resolve hosts. Host port 3389
  is taken by GNOME remote desktop, hence RDP on 13389. The owner holds the VM
  login.
- Microsoft blocks the automated ISO request from some regions; dockur then
  falls back to a static link on `software-static.download.prss.microsoft.com`,
  which works once the owner's proxy covers `microsoft.com` (exact-match proxies
  also need `www.`, `vlscppe.`, `software-static.download.prss.` and
  `ov-df.microsoft.com`). Third-party ISO mirrors behind Cloudflare Turnstile are
  useless for curl. aria2 resumes a partial download after stop/start; the
  unattended install takes 20-30 minutes.
- Screen: noVNC at `http://127.0.0.1:8006`, or RDP at `127.0.0.1:13389`. The
  host's `~/winvm/shared` is `\\host.lan\Data` inside the guest.
- The VM costs about 8 GB RAM and CPU: stop it when done
  (`docker stop -t 30 solaris-win`), and before a host reboot. The VM may stay on
  the root disk (owner exception).

## Building the Windows client

Cross-build in a client worktree with the mingw toolchain kept in the core
checkout's ignored `.analysis/mingw` (no system mingw):
`cargo build --release --locked --target x86_64-pc-windows-gnu --workspace`
(apply the zlib CFLAGS workaround from the client's `release.yml`). The exes
import only system DLLs. Package them in the previous release zip layout
(`LICENSE`, `README`, `icon.png`, `plugins/`, `solaris-v2.exe`) with the new
`solaris-client.exe` and `solaris-launcher.exe` swapped in.

## Driving the guest

Agents never open a headed browser or viewer on the owner's desktop.
[`tools/dev/winqa/cs-daemon.js`](../../tools/dev/winqa/cs-daemon.js) is the one
persistent headless-Firefox (Playwright) page; its protocol is in
[`tools/dev/winqa/RECIPE.md`](../../tools/dev/winqa/RECIPE.md) and summarised in
the QA protocol's "Windows-guest driver":

- Run it as a named service; send one JSON request line per `id`
  (`waitconn|shot|click|key|type|quit`) and match the reply by `id` in its
  output. Never `wait` on the daemon: it never exits.
- Before any input, `waitconn` until the page has the `noVNC_connected` class.
  Finding the canvas or seeing no "Connecting/Failed" text is not a connection;
  an empty page once passed that way (2026-10-24). After `docker restart` the
  page reconnects: `waitconn` again.
- Screenshots are 1440x1000 page pixels equal to guest coordinates; click them
  directly and do not remap through the canvas rectangle again.
- **The VNC path drops Shift.** Type lowercase only, no `:`, `"`, `_` or other
  shifted characters; use rooted paths without a drive letter (`\sc`, not
  `C:\sc`); SMB paths are case-insensitive. Drive dialogs by keyboard
  (Win+R, type, screenshot-verify, Enter); in the security "Run" prompt use
  `ArrowLeft` then `Space` (mouse clicks land about 40 px low). `cmd /k` keeps a
  console open for result screenshots.

## What to check

- Copy the zip to the Desktop and use Explorer "Extract All" so Mark-of-the-Web
  applies as for real users. For 150% scaling set 1920x1080 first; at 1280x800
  Windows offers only 100/125%.
- Launch from `cmd` with stderr captured
  (`solaris-client.exe --username <name> 2> client-err.txt`).
- The test server runs `solaris-auth`: `/register <pw> <pw>` within 60 s of the
  first join or the server kicks with "Authentication timeout" (not a network
  bug); later joins `/login <pw>`. Server-side evidence: the unit journal on the
  test host, grep the username; `exit=keepalive-timeout` versus a clean EOF.
- The VM has no GPU: D3D11 hardware device creation fails and the client falls
  back to WARP (check that capture creates a device before deeper QA). Software
  GL lags 5-10 s and loses press-and-release in one frame: confirm input failures
  on Linux before calling them product bugs. `solaris-launcher.exe` needs
  OpenGL 2+ and fails in the VM; skip the launcher check there.

Minimum gate before a Windows build goes to players:

1. Startup reaches the title screen with no resource-path error.
2. At least 5 minutes in a world (walk, break and place, open inventory) with no
   disconnect.
3. Inventory: a stack follows the cursor and drops into another slot; Q drops;
   changes persist after closing.
4. Screen sharing between a native owner on the server and the Windows viewer
   shows real changing frames, both directions if possible.
5. Disconnect returns to the title screen without "Not responding".

Any failure keeps the GitHub release a draft (`gh release edit <tag>
--draft=true`).

Windows-specific defects found this way: Windows paths produced resource names
with `\` (`block\acacia_door_bottom`) and the client failed at texture load
(2026-10-23); polling the play socket with a 10 ms receive timeout caused random
`os error 997`/`10053` disconnects, because a timed-out blocking Winsock receive
leaves the connection indeterminate (fixed with a dedicated blocking reader
thread, 2026-10-24).

## macOS

There is no macOS machine in the owner's setup. The client repository carries
the manual arm64 packaged startup/UI gate
`packaging/macos-runtime-gate.py`, run on a macOS runner after the release
workflow builds the signed app archive. It exercises launcher bootstrap and the
title UI only (no server, gameplay or OS input). A macOS claim beyond that
gate is blocked coverage, not a pass.
