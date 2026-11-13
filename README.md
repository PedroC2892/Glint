# Glint

A fast, keyboard-driven search launcher for the Linux desktop, inspired by the
system-wide search bar found on macOS Sequoia. Press a shortcut (`Win + Space`),
start typing, press Enter.

Built with Rust and [egui](https://github.com/emilk/egui)/eframe.

## Features

- **Applications and settings** – finds installed apps and settings panels
  (GNOME and KDE) from their freedesktop `.desktop` entries, with their icons.
- **Files and folders** – searches the names of everything in your home folder
  (hidden folders, `node_modules`, `target` and similar are skipped), grouped
  into Documents, Folders, Images, Music, Movies and Other Files, each showing
  its kind and last-modified date.
- **Top hit** – the best match is shown first in a highlighted card; things you open often rank higher.
- **Suggestions** – web searches and query completions directly accessible from the search results.
- **Calculator** – `12*(3+4)`, `2^10`, `sqrt(2)`, `2pi`, `sin(1)`…
- **Unit conversion** – length, mass, volume, time, data, speed, area and
  temperature: `10 km to mi`, `100 f in c`, `5 ft`.
- **Currency conversion** – `100 usd to eur`, `$20`, `€15 in gbp`, using
  today's exchange rates.
- **Definitions** – English word definitions from Wiktionary / Dictionary API.
- **Web search** – any query can be sent directly to your browser.
- **Preview** – press `Ctrl+Y` for a side preview with file details, a text
  snippet or an image.
- **System icons** – every result uses your desktop's icon theme (KDE or
  GNOME setting, following the theme's `Inherits=` chain down to `hicolor`).
  File icons are picked by file type from the shared MIME database, the same
  way your file manager does it.

## Keyboard

| Key | Action |
| --- | --- |
| `↑` / `↓`, `Tab` | Move through results |
| `Ctrl+↑` / `Ctrl+↓` | Jump between sections |
| `Enter` | Open (or copy a calculation/conversion result) |
| `Ctrl+Enter`, `Ctrl+R` | Show the file in its folder |
| `Ctrl+C` | Copy the result, path or name |
| `Ctrl+B` | Search the web |
| `Ctrl+L` | Jump to the definition |
| `Ctrl+Y` | Toggle the preview |
| `Ctrl+,` | Open or close settings |
| `Esc` | Clear the search, then close |

Clicking outside the panel, or switching to another window, closes Glint.

## Settings

Open settings with `Ctrl+,`, by typing "settings", "preferences", or "glint" and
choosing **Glint Settings**, by running `glint --settings`, or from the
application menu's "Glint Settings" desktop action. Press `Esc` to go back.

Options include:

- **Appearance** – accent colour presets plus custom colour selection, and
  window opacity from 50% to 100% (requires a compositor).
- **Files** – toggle showing hidden files and folders, or index folders outside
  your home directory with a custom folder list (unreadable folders are skipped).
  Directories such as `.ssh`, `.gnupg`, `.password-store`, `.pki`, `.git`,
  `.cache`, keyrings, and Trash are never indexed, and hidden files are never
  previewed.
- **Search** – toggle online lookups (definitions and exchange rates).

Changes save automatically to `$XDG_CONFIG_HOME/glint/config.json` (default
`~/.config/glint/config.json`), and indexing changes apply when leaving settings.
Ignored directory names (`skip_dirs`) can be edited directly in the JSON file.
Package caches and runtimes (npm, cargo, rustup, flatpak, …) are skipped by default via `skip_dirs`.

## Launching with a Keyboard Shortcut (Win + Space)

Glint is designed to be summoned instantly with **Win + Space** (Super + Space):

1. **Automatic setup**: `./install.sh` automatically registers `Win + Space` / `Meta + Space`:
   - On **KDE Plasma**: registered via `X-KDE-Shortcuts=Meta+Space` in the desktop entry.
   - On **GNOME**: registered via `gsettings` custom media-keys keybinding.
2. **Manual setup in any desktop environment / Window Manager**:
   - Command: `~/.local/bin/glint`
   - Shortcut: `Super+Space` (or `Mod4+space`)
   - **Xfce**: *Settings → Keyboard → Application Shortcuts* → Add `~/.local/bin/glint` with `Super+Space`.
   - **i3 / Sway**: Add `bindsym $mod+space exec ~/.local/bin/glint` to your config.

## Build and Run

Requires a recent stable Rust toolchain (edition 2024).

```sh
cargo build --release
./target/release/glint
```

Options:

- `--query TEXT` – start with a search already typed in.
- `--settings` – open directly in settings view.
- `--screenshot FILE.png` – draw about 120 frames, save the window as a PNG and
  exit, without taking keyboard focus (used for automated visual checks).
- `--keys "Down,Down,Right"` – with `--screenshot`, inject these keys (Up, Down, Left, Right, Tab, Space, Enter, Escape), one per frame, to check keyboard navigation.

Run the tests with `cargo test`.

## Install

```sh
./install.sh             # build, then install for the current user and set up shortcuts
./install.sh --uninstall # remove it again
```

The script needs no root access. It installs the binary to `~/.local/bin`, plus
a menu entry and an icon under `$XDG_DATA_HOME` (default `~/.local/share`).
`install.sh` uses the `local` profile (no LTO, incremental) so reinstalls after
a code change take seconds, while `cargo build --release` produces the LTO build.

## Privacy

Everything is searched locally. Only two features use the network, both over
HTTPS:

- **Currency conversion** downloads the public USD exchange-rate table from
  [open.er-api.com](https://open.er-api.com). Your query is never sent.
- **Definitions** send the selected word to the
  [Free Dictionary API](https://dictionaryapi.dev), and only when you select
  the Definition row, never while you type.

Online lookups can be disabled in Settings. Sensitive directories (`.ssh`,
`.gnupg`, `.password-store`, `.pki`, `.git`, `.cache`, keyrings, and Trash) are
never indexed, and hidden files are never previewed.

Web searches open [DuckDuckGo](https://duckduckgo.com) in your browser.
Usage counts that improve ranking are stored locally in
`~/.local/share/glint/history.tsv`.

## Performance Notes

- The file index is built in a background thread. While it fills up, results
  refresh at most twice a second.
- Scoring a name allocates nothing, and only the results that can be shown are
  kept, so a keystroke over ~150,000 files takes a few tens of milliseconds.
- Glint asks for the low-power GPU, so laptops with two GPUs don't wake
  the discrete one for a small panel. Set `WGPU_POWER_PREF=high` to override.
- Dependencies are optimised even in debug builds.

## Platform Notes

Developed and tested on Linux with KDE Plasma and GNOME on X11 and Wayland. On X11, Glint asks the
window manager for keyboard focus the way a taskbar does, so it works even with
focus-stealing prevention.

## Project Layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Entry point |
| `src/ui.rs` | Window, search logic, keyboard handling |
| `src/draw.rs` | Result list, preview and icon drawing |
| `src/index.rs` | Apps, file index, history, dates |
| `src/config.rs` | Settings, defaults and validation |
| `src/icons.rs` | Desktop icon theme and file-type icons |
| `src/calc.rs` | Calculator, unit and currency conversion |
| `src/open.rs` | Launching apps and files, online lookups |
| `src/x11focus.rs` | Keyboard focus on X11 |
| `assets/glint.svg` | App icon (vector) |
| `assets/icons/` | App icon PNG suite (16x16 to 512x512) |
| `install.sh` | Per-user installer and shortcut configurator |
