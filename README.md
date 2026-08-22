# Minecraft 26.2 MultiBot Pro

Native Minecraft Java **26.2** multi-bot controller built with [Azalea](https://github.com/azalea-rs/azalea). Each bot can use a different SOCKS5 proxy, so the target server sees different outbound IP addresses without ViaProxy.

> Use only on a server you own or have explicit permission to test. Public proxies are unreliable and may inspect traffic. Offline/cracked accounts have no password encryption and must never reuse a real password.

## What is preserved

- Native Minecraft Java 26.2 protocol
- Offline/cracked usernames
- Multiple simultaneous bots
- A different SOCKS5 connection per bot
- Automatic public proxy fetching and 60-second refresh
- Dead-proxy tracking and rotation on reconnect
- Global 6.5-second join spacing
- Automatic reconnect queue
- Fixed-count and infinite bot generation
- Random realistic usernames
- Following nearby players
- Random wandering when no player is nearby
- Optional nearby attacks
- Automatic `/register` and `/login` detection
- Global and individual chat commands
- Repeated chat command with cancellation
- Visible player-name scan and specific-name join
- Inventory dropping for a specifically queued stolen name
- Stay timer
- Runtime terminal controls
- SRV/hostname and explicit `host:port` addressing

## Requirements

- Windows/Linux/macOS: rustup (the repository pins `nightly-2026-07-22`, the Azalea-compatible toolchain)
- Termux: its patched `rust` package; the documented build sets `RUSTUP_TOOLCHAIN=nightly-termux` and `RUSTC_BOOTSTRAP=1` because rustup is unsupported on Android and Azalea checks for a nightly channel
- Git
- Approximately 3-6 GB free storage for the first Rust/Azalea build
- A 64-bit Android device for Termux

The first build is large and can take 10-40 minutes depending on the device. Later starts use the compiled release binary and are much faster.

## Termux installation

Install the current Termux application from F-Droid or GitHub, not the obsolete Play Store build.

```bash
pkg update -y && pkg upgrade -y
pkg install -y git rust clang pkg-config openssl
git clone https://github.com/Smile-B14/26.2-Multibot.git
cd 26.2-Multibot
RUSTUP_TOOLCHAIN=nightly-termux RUSTC_BOOTSTRAP=1 cargo build --release
./target/release/multibot-26-2
```

Later starts:

```bash
cd ~/26.2-Multibot
./target/release/multibot-26-2
```

Update the project and rebuild:

```bash
cd ~/26.2-Multibot
git pull
RUSTUP_TOOLCHAIN=nightly-termux RUSTC_BOOTSTRAP=1 cargo build --release
./target/release/multibot-26-2
```

If Android kills Termux in the background, disable battery optimization for Termux and acquire a wake lock:

```bash
termux-wake-lock
```

Closing Termux or Android killing its process disconnects the bots. Use `tmux` if you only need the terminal session to survive closing the visible window:

```bash
pkg install -y tmux
tmux new -s multibot
./target/release/multibot-26-2
```

Detach with `Ctrl+B`, then `D`. Return with:

```bash
tmux attach -t multibot
```

## Windows CMD installation

1. Install Git: <https://git-scm.com/download/win>
2. Install Rust using `rustup-init.exe`: <https://rustup.rs/>
3. Install **Desktop development with C++** from Visual Studio Build Tools if Rust reports a missing linker.
4. Open a new Command Prompt.

```bat
git clone https://github.com/Smile-B14/26.2-Multibot.git
cd 26.2-Multibot
cargo build --release
target\release\multibot-26-2.exe
```

Later starts:

```bat
cd 26.2-Multibot
target\release\multibot-26-2.exe
```

Update and rebuild:

```bat
cd 26.2-Multibot
git pull
cargo build --release
target\release\multibot-26-2.exe
```

## Linux/macOS installation

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
git clone https://github.com/Smile-B14/26.2-Multibot.git
cd 26.2-Multibot
cargo build --release
./target/release/multibot-26-2
```

## Startup questions

```text
Server IP / hostname (include :port if needed):
Bot names separated by commas (blank = generated):
How many bots initially? (0 = infinite):
Minutes to stay (0 = until quit):
```

Examples:

```text
play.example.com
```

```text
play.example.com:25570
```

For names:

```text
SmileBot1,SmileBot2,SmileBot3
```

If names are blank, valid 3-16 character offline usernames are generated automatically. If the requested count is larger than the supplied list, additional names are generated.

## Terminal commands

```text
all <message>
one <name> <message>
list

add <count>
stopspawn
stopjoin
startjoin

rejoin all
rejoin <name>
restart all
restart <name>

spam <interval_ms> <message>
stopspam

steal on
steal off
steal <username>

ai on
ai off
hit on
hit off
logs on
logs off

auth on
auth off
authpass <password>

resetban
resetplayer <name>
resetall

version
help
quit
```

### Authentication

For 30 seconds after login, system/plugin messages are checked for registration and login cues. Ordinary player-style `<name> message` chat is ignored.

Default runtime password:

```text
thematic
```

Commands sent when detected:

```text
/register thematic thematic
/login thematic
```

Change it before joining additional bots:

```text
authpass A_Different_Test_Password
```

Do not use a real account password on an offline server.

### Proxy behavior

The controller downloads SOCKS5 addresses from ProxyScrape. Each bot receives its own next available proxy. A failed bot marks that proxy dead locally, queues the username again and reconnects it through another proxy.

If the provider returns no proxies, the controller uses a direct connection temporarily and continues refreshing once per minute.

Public proxy limitations:

- Many addresses are dead, slow or already banned.
- A listed proxy is not guaranteed to produce a unique IP.
- The server can still rate-limit usernames, accounts or behavior.
- `resetban` only clears local dead-proxy state; it does not bypass an active server ban.

### Steal-name behavior

`steal on` scans the tab lists visible to connected bots and queues previously unseen usernames. `steal <username>` queues one specific valid username. Bots created this way drop inventory items after login.

This only works on offline-mode servers that allow duplicate/offline identity behavior. It cannot impersonate an authenticated Microsoft account on an online-mode server.

## Runtime resilience

Recoverable failures are handled without panicking:

- Invalid or unresolvable server addresses ask for the address again.
- Proxy downloads have a 20-second timeout, HTTP status validation and a 2 MiB response limit.
- Failed proxy refreshes keep the current pool and retry after 60 seconds.
- Dead proxies automatically become eligible again after 10 minutes.
- Infinite spawning keeps only one generated bot waiting, preventing an unbounded queue.
- Startup and `add` accept up to 10,000 bots per request to prevent accidental memory exhaustion.
- AI errors on unloaded entities skip that tick instead of stopping the bot handler.
- Closed or failed terminal input triggers a clean swarm shutdown.

Android can still terminate Termux for low memory, battery management or a forced app close. Native dependency bugs and operating-system failures cannot be caught reliably by application-level error handling.

## Verification

Run formatting, tests and a release build:

```bash
cargo fmt --check
cargo test
cargo build --release
```

GitHub Actions runs the same checks on every push and also compiles with the stable-plus-`RUSTC_BOOTSTRAP` path used by Termux.

## Updating for future Minecraft versions

Minecraft protocol updates require Azalea support. Check Azalea's supported version before updating the pinned dependency. After Azalea supports the new version, update the four Azalea Git revision entries in `Cargo.toml`, then run:

```bash
cargo update
cargo test
cargo build --release
```

`Cargo.toml` deliberately pins the exact Azalea revision, so a future dependency update cannot silently break a working 26.2 build. No bot framework can support an unreleased protocol automatically; once Azalea implements a new protocol, update the pinned revisions and re-run CI.

## Troubleshooting

### Build killed in Termux

Android likely ran out of memory. Close other applications and limit parallel compilation:

```bash
RUSTUP_TOOLCHAIN=nightly-termux RUSTC_BOOTSTRAP=1 CARGO_BUILD_JOBS=1 cargo build --release
```

### Linker or C compiler missing

Termux:

```bash
pkg install -y clang pkg-config
```

Windows: install Visual Studio Build Tools with **Desktop development with C++**, then reopen CMD.

### Bots repeatedly fail to connect

Enable logs:

```text
logs on
```

Public proxies commonly fail. Allow the refresh and rotation queue to continue, verify the server address, and test one direct connection if necessary.

### Server says unsupported version

This project is pinned to Azalea's native Minecraft Java 26.2 implementation. Make sure you built the current repository revision:

```bash
git pull
cargo clean
cargo build --release
```

## License

MIT. Azalea is separately licensed under MIT.

