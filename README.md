# Lookout

A fast, keyboard-driven search launcher for the Linux desktop, inspired by the
system-wide search bar found on macOS Sequoia. Press a shortcut, start typing,
press Enter.

Built with Rust and [egui](https://github.com/emilk/egui)/eframe.

## Features

- **Applications and settings** – finds installed apps and settings panels
  (GNOME and KDE) from their freedesktop `.desktop` entries, with their icons.
- **Files and folders** – searches the names of everything in your home folder
  (hidden folders, `node_modules`, `target` and similar are skipped), grouped
  into Documents, Folders, Images, Music, Movies and Other Files, each showing
  its kind and last-modified date.
- **Top hit** – the best match is shown first; things you open often rank higher.
- **Calculator** – `12*(3+4)`, `2^10`, `sqrt(2)`, `2pi`, `sin(1)`…
- **Unit conversion** – length, mass, volume, time, data, speed, area and
  temperature: `10 km to mi`, `100 f in c`, `5 ft`.
- **Currency conversion** – `100 usd to eur`, `$20`, `€15 in gbp`, using
  today's exchange rates.
- **Definitions** – English word definitions.
- **Web search** – any query can be sent to your browser.
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
| `Esc` | Clear the search, then close |

Clicking outside the panel, or switching to another window, closes Lookout.

## Build and run

Requires a recent stable Rust toolchain (edition 2024).

```sh
cargo build --release
./target/release/lookout
```

Options:

- `--query TEXT` – start with a search already typed in.
- `--screenshot FILE.png` – draw about 120 frames, save the window as a PNG and
  exit, without taking keyboard focus (used for automated visual checks).

Run the tests with `cargo test`.

## Install

```sh
./install.sh             # build, then install for the current user
./install.sh --uninstall # remove it again
```

The script needs no root access. It installs the binary to `~/.local/bin`, plus
a menu entry and an icon under `$XDG_DATA_HOME` (default `~/.local/share`). On
KDE Plasma it asks for the `Meta+Space` shortcut. On other desktops, bind a
shortcut of your choice to `~/.local/bin/lookout` in your keyboard settings.

## Privacy

Everything is searched locally. Only two features use the network, both over
HTTPS:

- **Currency conversion** downloads the public USD exchange-rate table from
  [open.er-api.com](https://open.er-api.com). Your query is never sent.
- **Definitions** send the selected word to the
  [Free Dictionary API](https://dictionaryapi.dev), and only when you select
  the Definition row, never while you type.

Web searches open [DuckDuckGo](https://duckduckgo.com) in your browser.
Usage counts that improve ranking are stored locally in
`~/.local/share/lookout/history.tsv`.

## Performance notes

- The file index is built in a background thread. While it fills up, results
  refresh at most twice a second.
- Scoring a name allocates nothing, and only the results that can be shown are
  kept, so a keystroke over ~150,000 files takes a few tens of milliseconds.
- Lookout asks for the low-power GPU, so laptops with two GPUs don't wake
  the discrete one for a small panel. Set `WGPU_POWER_PREF=high` to override.
- Dependencies are optimised even in debug builds.

## Platform notes

Developed and tested on Linux with KDE Plasma on X11. On X11, Lookout asks the
window manager for keyboard focus the way a taskbar does, so it works even with
focus-stealing prevention. Wayland and other desktops should work but are less
tested.

## Project layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Entry point |
| `src/ui.rs` | Window, search logic, keyboard handling |
| `src/draw.rs` | Result list, preview and icon drawing |
| `src/index.rs` | Apps, file index, history, dates |
| `src/icons.rs` | Desktop icon theme and file-type icons |
| `src/calc.rs` | Calculator, unit and currency conversion |
| `src/open.rs` | Launching apps and files, online lookups |
| `src/x11focus.rs` | Keyboard focus on X11 |
| `assets/lookout.svg` | App icon |
| `install.sh` | Per-user installer |
