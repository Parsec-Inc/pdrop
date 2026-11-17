# ◉ pdrop

**pdrop** is an [AirDrop](https://support.apple.com/explore/new/airdrop)-style sharing tool for your local network, wrapped in a live **radar TUI**.

Run `pdrop` on two or more machines on the same network and they appear as blips on each other's radar. Select a peer and drop files, free-form text, or your clipboard to them — they get a prompt, accept, and it lands in their `~/Downloads` (or their clipboard). No accounts, no servers, no cloud: everything travels directly over the LAN.

```
 ◉ pdrop   visível como kami          2 por perto
 ───────────────────────────────╮  │ TRANSFERÊNCIAS
      ~~~ ~~~                   │  │ ↑ notes.md  para laptop
   ~~~~~~~~   (o.o)             │  │   ━━━━━━━━────────  58%  4.2 MB
  ~~~~~~~~~~~  laptop           │  │
   ~~~~~~~~   [___]             │  │
      ~~~ ~~~                   │  │
        você                    │  │
 ───────────────────────────────╯  │
 selecionado laptop                │
 ←→ escolher  enter enviar  v clipboard  b ponte  q sair
```

![](docs/assets/image.png)

## What it can do

- **Send files** — type or paste a path (drag-and-drop into the terminal works too; `~`, quotes, and `file://` URLs are cleaned up automatically).
- **Send text** — anything that isn't an existing file path is sent as a free-form message.
- **Clipboard share** (`v`) — push your whole clipboard to a peer once; it lands in their clipboard with the same type (text, or images with `wl-clipboard`/`xclip`).
- **Clipboard bridge** (`b`) — keep it on and *everything you copy* flows to the peer automatically, like a shared clipboard. The recipient approves once per session; repeat copies arrive without asking.
- **Live radar** — animated sweeping radar with blips for each peer, colored by their chosen accent, orbiting an ASCII icon, pulsing when the sweep passes. Ping-based RTT places close peers on the inner ring.
- **Identity** — pick your display color (`c`), ASCII icon (`i`), and name (`r`); all persisted in `~/.config/pdrop` and broadcast so others see them.
- **Accept/refuse prompts** — incoming transfers ask y/n in-app, plus a terminal bell and a desktop notification (`notify-send` on Linux, `osascript` on macOS) so you notice even when the terminal is in the background.
- **Safe by construction** — incoming file names are stripped to the last path component, existing files are never overwritten (`photo.jpg`, `photo (1).jpg`, …), failed writes delete the partial file, text messages are capped at 1 MiB and clipboard payloads at 64 MiB.

## Requirements

- A Rust toolchain (**Rust 2024 edition**, so Rust 1.85+) — only for building.
- Linux or macOS. (Anything with `cargo` should work; clipboard support is wired for Wayland/X11/macOS.)
- Optional clipboard helpers, auto-detected in this order:
  - macOS: `pbpaste`/`pbcopy` (built in, always present)
  - Wayland: `wl-clipboard` (`wl-copy`/`wl-paste`)
  - X11: `xclip`, then `xsel` as fallback

Without any clipboard tool, file and text transfers still work — the app just shows *"sem clipboard"* ("no clipboard") for clipboard features:

```bash
# Debian/Ubuntu
sudo apt install wl-clipboard    # Wayland
sudo apt install xclip           # X11
# Arch
sudo pacman -S wl-clipboard      # or: xclip
# Fedora
sudo dnf install wl-clipboard    # or: xclip
```

## Install

Pre-built packages are attached to each [release](https://github.com/Parsec-Inc/pdrop/releases):

| Platform | Install |
|---|---|
| Fedora / RHEL | `sudo dnf install ./pdrop-*.rpm` (COPR: `sudo dnf copr enable parsec-inc/pdrop`) |
| Debian / Ubuntu | `sudo apt install ./pdrop_*.deb` |
| Arch | `sudo pacman -U pdrop-*.pkg.tar.zst` (AUR: `yay -S pdrop`) |
| Snap | `sudo snap install pdrop` |
| macOS | `brew install Parsec-Inc/tap/pdrop` |
| Windows | run `pdrop-*.msi` |

Maintainers: see [docs/PACKAGING.md](docs/PACKAGING.md). Not every channel above is published yet.

### From source

```bash
git clone <this repo>
cd pdrop
cargo install --path .
```

`cargo install` puts the binary in `~/.cargo/bin` — make sure that's on your `PATH`.

### Static binary (musl)

The repo has already been built with the musl target, producing a fully static Linux binary with no runtime dependencies:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# result: target/x86_64-unknown-linux-musl/release/pdrop
```

### Run without installing

```bash
cargo run --release -- --name kami
```

## Usage

```bash
pdrop [--name NAME] [--peer IP]... [--calm]
```

| Flag | Meaning |
|------|---------|
| `--name`, `-n NAME` | Display name for this session only (overrides the saved one). |
| `--peer`, `-p IP` | Also send discovery beacons directly to this IP. Repeat the flag for several peers. Useful on networks that filter broadcast/multicast — just point it at the other machine's IP. |
| `--calm` | Slower, quieter animation. Also slows down further while you're typing or a prompt is pending. |

With no valid arguments, it prints the usage line. Everything else is done inside the TUI.

### Starting out

1. Run `pdrop` on both machines (same network).
2. Each appears on the other's radar within a couple of seconds. Peers fade out after ~6 s of silence and disappear; new ones announce with an expanding ring.
3. Move the selection with `←/→` (or `h/l`, `Tab`).

### Keys

| Key | Action |
|-----|--------|
| `←/→`, `h/l`, `Tab`/`shift+Tab` | Select peer (or browse the open color/icon picker) |
| `enter`, `space` or `s` | Open the send prompt for the selected peer (confirms inside a picker) |
| *(drag & drop)* | Dropping a file on the window sends it to the selected peer |
| `v` | Send current clipboard once |
| `b` | Toggle the clipboard bridge to the selected peer |
| `x` | Cut all clipboard bridges others opened *toward* you |
| `c` | Pick your accent color (8-color palette, ←→ to browse, `enter` to confirm) |
| `i` | Pick your radar icon (11 ASCII glyphs, or just the dot) |
| `r` | Rename yourself (max 32 chars, saved persistently) |
| `y` / `n` | Accept / refuse an incoming transfer or bridge request |
| `esc` | Cancel the open prompt or picker |
| `q`, `ctrl+c` | Quit |

The bottom bar shows context-sensitive hints for whatever is happening (prompt open, color picker, pending incoming transfer, …).

### Where things go

- **Received files** → `~/Downloads` (created if missing; never overwritten).
- **Received text/clipboard** → your clipboard (requires one of the clipboard tools above).
- **Your settings** (name, color, icon) → `~/.config/pdrop/{nome,cor,icone}` as plain text.

## How it works

Everything is peer-to-peer on the LAN; there is no server anywhere.

- **Discovery** — a beacon `PDROP1|<id>|<port>|<color+icon>|<name>` is broadcast over UDP port **47474** every 1.5 s. Because many Wi-Fi APs filter broadcast but pass unicast, pdrop also sweeps every host in its IPv4 subnets (up to /22) with 2 ms pacing, and `--peer` adds unicast targets. When a machine sees a *new* beacon it replies with its own beacon in unicast, so only one side needs to know the other's address. Peers also exchange `PING`/`PONG` to measure smoothed RTT.
- **Transfer** — a TCP connection to the peer's port **47475** (falls back to an ephemeral port, which the beacon advertises). Protocol is deliberately simple, one line then bytes:

  ```
  PDROP1\t<kind>\t<size>\t<sender>\t<label>\n     kind: f=file, t=text, c=clipboard once, b=clipboard bridge
  recipient replies a single byte: Y or N
  ...payload bytes...
  ```

  The color+icon ride in the same beacon field (`color | icon << 3`), so older builds that mask with `% 8` still show the right color.
- **Clipboard bridge** — the sender polls its clipboard every 400 ms and pushes any *new* text (its own last-written content is remembered to avoid echo loops). It stops by itself if the peer refuses, drops the connection, or leaves the network.
- **Notifications** — a `\x07` bell plus `notify-send`/`osascript`, with sender and file name passed as *arguments* (never interpolated into a command line) since they come from the network.

## Privacy & safety notes

- Anyone on the same network can see your pdrop beacon (name, color, icon) — that's how discovery works. Transfers are explicit and always require the recipient's y/n (except bridge copies after a one-time grant, which you can cut with `x`).
- The clipboard bridge sends **everything you copy** while it's on. Password managers usually bypass the system clipboard, but double-check before bridging on a shared network.
- Incoming file names are sanitized (no paths, no control characters) and payloads are size-capped, but as with any LAN transfer tool, run it among people you trust.

## Troubleshooting

| Symptom | Try this |
|---------|----------|
| Peers never appear | Both sides must be on the same subnet. Firewalls often block UDP 47474/TCP 47475 — allow them, or use `--peer <other-ip>` on at least one side. Some guest/enterprise Wi-Fi isolates clients (AP isolation); no LAN tool can work across that. |
| Radar shows the peer but sends fail | TCP 47475 is likely blocked, or the peer already has something listening there. |
| "sem clipboard" flash | Install `wl-clipboard` (Wayland) or `xclip` (X11); on macOS this shouldn't happen. |
| Clipboard bridge stops on its own | That's by design when the peer refuses, disconnects, or leaves — the header will say *ponte encerrada*. |
| Two instances on the same machine | Fine on Linux (SO_REUSEPORT); they'll see each other. |

## Project layout

```
src/
├── main.rs   App state, key handling, settings persistence, notifications, TUI event loop
├── net.rs    Discovery (UDP sweep/broadcast), TCP transfer protocol, clipboard bridge
├── ui.rs     The ratatui radar: Braille canvas, blips, palette, icons, footer hints
└── clip.rs   Text clipboard via system tools (wl-clipboard / xclip / xsel / pbcopy)
```

Zero-heavy-dependency by design: only `ratatui` (UI), `socket2` (broadcast/reuse sockets), and `if-addrs` (subnet scan).

## Building & testing

```bash
cargo build --release
cargo test        # loopback file + text transfer test, name sanitization
```

## Status & license

v0.1.0 — early, single-purpose, and it works. Known gaps: no folder transfer (deliberately rejected for now), no encryption on the wire, discovery is IPv4-only.

No license file yet — add one before sharing publicly.
