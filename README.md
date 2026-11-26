# rust-openconnect-gui

A minimal Rust/egui GUI for `openconnect` on Linux — connect to your Cisco
AnyConnect VPN without opening a terminal, with the password saved in your
keyring so connecting is one click (or zero, with auto-connect).

It is a **thin wrapper around the `openconnect` CLI**: the GUI never runs as
root and never reimplements the protocol. It spawns
`sudo -n /usr/bin/openconnect <server> --user=<you> --passwd-on-stdin`, feeds
the password over stdin, tails the child's output into a log pane, and stops
the tunnel with `sudo -n pkill -INT -x openconnect` (a clean gateway logout,
same as Ctrl-C).

```
GUI (your user) ── spawns & monitors ──▶ sudo -n openconnect … (root)
   password ← gnome-keyring (Secret Service)
   output   → ~/.local/state/rust-openconnect-gui/openconnect.log
```

## Features

- **Saved password** in the freedesktop Secret Service (`gnome-keyring`),
  encrypted at rest, unlocked at login — stored via the `keyring` crate,
  sent to openconnect over stdin, never on the command line.
- **Certificate pinning**: on the first connection the server's certificate
  is accepted automatically and its fingerprint pinned (trust-on-first-use,
  like SSH); every later connection is fully unattended. If the server's
  certificate legitimately changes, delete the `servercert` line from
  `~/.config/rust-openconnect-gui/config.toml`.
- **Auto-connect at launch** — zero-touch VPN on login.
- **Survives GUI exit**: openconnect writes to a log file (not a pipe), so
  closing the app does **not** drop the VPN; reopening the GUI re-attaches
  to the running tunnel and shows its log.
- **Prompt answers**: a small answer box feeds openconnect's stdin, for
  gateways that ask a second question (2FA/OTP) after the password.
- **Live log pane** with everything openconnect prints, plus in-app
  diagnostics (`Preflight`) and a `--check` CLI mode.

## One-time setup

The GUI needs to start/stop openconnect as root without a password prompt.
Create `/etc/sudoers.d/openconnect-gui`:

```sh
echo "$USER ALL=(root) NOPASSWD: /usr/bin/openconnect *, /usr/bin/pkill -INT -x openconnect" \
  | sudo install -m440 /dev/stdin /etc/sudoers.d/openconnect-gui
sudo visudo -cf /etc/sudoers.d/openconnect-gui   # must say: parsed OK
```

(The app shows a **Setup** panel with the same line and a copy button —
but only when the rule is missing; on a configured machine it stays out
of the way. On a single-user machine this grants nothing your sudo
password doesn't already grant; to tighten it, replace `*` with the
exact arguments your connection uses.)

Everything else works out of the box on a system with `openconnect`, `sudo`
and a running Secret Service (`gnome-keyring-daemon --components=secrets`).

## Run / install

```sh
cargo run --release          # from source
cargo run --release -- --check   # headless preflight: sudoers, keyring, profile
```

Install as an Arch package (shows up in the app launcher):

```sh
makepkg -si
```

### Prebuilt binary (any Linux with glibc)

Download `rust-openconnect-gui-linux-amd64.tar.gz` from
[Releases](https://github.com/Coder-Mehedi/rust-openconnect-gui/releases), then:

```sh
tar xzf rust-openconnect-gui-linux-amd64.tar.gz
sudo install -Dm755 rust-openconnect-gui /usr/local/bin/rust-openconnect-gui
```

That gives you the `rust-openconnect-gui` command (verify with
`rust-openconnect-gui --check`). It will **not** show up in `pacman -Q` —
it's a plain file, not a package — and it only appears in your app launcher
if you also install the desktop entry and icon (included in the tarball
since v0.1.2):

```sh
sudo install -Dm644 rust-openconnect-gui.desktop /usr/share/applications/rust-openconnect-gui.desktop
sudo install -Dm644 rust-openconnect-gui.svg /usr/share/icons/hicolor/scalable/apps/rust-openconnect-gui.svg
```

If the app doesn't show in the launcher after that: launchers list
`.desktop` entries, not binaries, so the desktop file must be installed —
and KDE caches the application list, so on Plasma run `kbuildsycoca6`
afterwards (then search for “OpenConnect”). `rust-openconnect-gui
--check` reports whether the desktop entry is in a location launchers
read.

On a fresh machine you also need the pieces the GUI drives:
`sudo pacman -S openconnect`, a running Secret Service (gnome-keyring) for
password saving, and the one-time sudoers rule from [Setup](#one-time-setup)
below. `rust-openconnect-gui --check` reports which of these are missing —
including the desktop entry itself.

### Arch Linux (AUR)

Once published on the AUR:

```sh
yay -S rust-openconnect-gui   # or paru -S rust-openconnect-gui
```

## First connection

1. Fill in Server, Username, Password; tick *Save password in keyring*
   (and *Auto-connect at launch* if you want zero-touch starts).
2. Click **Connect**. The server's certificate is accepted and pinned
   automatically on this first connection; from then on connecting is a
   single click (or zero, with auto-connect).

## Files

| Path | Purpose |
| --- | --- |
| `~/.config/rust-openconnect-gui/config.toml` | server, username, pinned fingerprint, options |
| `~/.local/state/rust-openconnect-gui/openconnect.log` | openconnect output (rotated at 5 MiB) |
| Secret Service entry `rust-openconnect-gui / vpn-password` | the password |

## Security notes

- The password lives in your login keyring, not in a plaintext file.
- `/usr/bin/openconnect *` in sudoers includes openconnect's `--script`
  flag, which executes a script as root — anything running as your user
  could therefore open a VPN silently. That is the standard trade for a
  single-user desktop; you already have full sudo.
- The log file contains everything openconnect prints, which never includes
  the password (it is read from stdin).
- Certificate trust is automatic on first contact (TOFU). The fingerprint is
  pinned and any later mismatch fails the connection — a legitimate server
  certificate change means removing the `servercert` line from the config.
