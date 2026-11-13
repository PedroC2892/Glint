# Glint — Progress

Last updated: 2026-11-13

Glint is a keyboard-driven, Spotlight-style search launcher for the Linux
desktop, written in Rust with eframe/egui 0.37.

## Status at a glance

| Branch | Contents | State |
|---|---|---|
| `main` | Launcher + settings page (first version) + fast install profile | Pushed |
| `fix/settings-ui` | Settings redesign + keyboard focus fixes | Pushed, **not merged** — waiting for a real-keyboard check |

Tests: 22 passing, 1 ignored benchmark (`bench_index_home`). Clippy: one
pre-existing `type_complexity` warning in `src/ui.rs`.

## Done

### Launcher core (before the settings work)

- Search for installed applications and settings panels (`.desktop` files,
  XDG data dirs, `OnlyShowIn`/`NotShowIn`, KDE settings modules).
- File and folder search in the home folder, results grouped by kind
  (Documents, Folders, Images, Music, Movies, Other).
- Calculator, unit and currency conversion (live rates from open.er-api.com).
- Word definitions (Free Dictionary API), fetched only when the row is selected.
- Web search suggestions (DuckDuckGo).
- Quick Look–style preview pane (`Ctrl+Y`), usage history that boosts ranking.
- Desktop icon theme lookup with painted fallbacks.
- Transparent black "Sequoia" style panel; full location shown for files.
- `install.sh`: per-user install (XDG paths), icon suite, `.desktop` entry,
  Super+Space shortcut (GNOME via gsettings, KDE via `X-KDE-Shortcuts`),
  `--uninstall`.

### Settings page — first version (`main`, commit `fac1e7f`)

- Open with `Ctrl+,`, by searching "settings" / "preferences" / "glint",
  with `glint --settings`, or from the desktop entry's "Glint Settings" action.
- Settings stored in `$XDG_CONFIG_HOME/glint/config.json`
  (default `~/.config/glint/config.json`), new module `src/config.rs`:
  defaults for missing fields, sanitizing, 1 MiB size limit, broken files
  reported and never overwritten, atomic save (temp file + rename),
  debounced while editing and flushed on exit.
- Appearance: accent colour (presets + custom) applied live; panel opacity
  50–100%.
- Files: optional hidden files; optional folders outside home (default
  `/media`, `/mnt`, `/run/media`), canonicalized, deduplicated, nested roots
  dropped, unreadable folders skipped.
- Search: "Online lookups" switch disables definition and exchange-rate
  requests.
- Re-indexing only when leaving settings with changed index options; the
  previous index thread is cancelled.

### Keyboard navigation (`main`, commit `6e444d0`)

- First attempt using egui's built-in focus. Superseded by the redesign on
  `fix/settings-ui` (see below), because focus got stuck on the opacity slider.

### Faster install (`main`, commit `906bbdd`)

- `install.sh` builds a new `local` cargo profile (release without LTO,
  incremental). Rebuild after a code change: **24.7 s → ~1 s**.
- No measurable cost: indexer 316 ms vs 311 ms, binary ~25 MB in both.
  `cargo build --release` still produces the LTO build.
- Script honours `CARGO_TARGET_DIR`; without cargo it falls back to an
  existing `local` or `release` binary. No `cargo clean` (it would undo the
  incremental speed-up).

### Settings redesign (`fix/settings-ui`, commits `f7c55f5`, `9b43e6a`)

- Root cause of the stuck slider: egui's Slider has a value field that, once
  focused, consumes ArrowUp/ArrowDown; egui's directional focus also picks
  widgets by screen position.
- New page in `src/settings_page.rs`, macOS-style: section cards
  (Appearance, Files, Search), title/subtitle rows with right-aligned
  controls, painted toggle switches, slider without a value field, hex field
  for custom colours, focused row highlighted like a search result.
- Own row-based focus model: Up/Down/Tab move between rows (no wrap),
  Left/Right adjust (accent presets, opacity ±5%), Enter/Space toggle or
  activate, Esc goes back. Text rows (hex colour, add folder) keep
  Left/Right/Enter/Space for editing, but Up/Down/Tab always leave them.
- Navigation keys are removed from the raw input (`raw_input_hook`) before
  egui sees them, so egui's hidden focus and "Enter as click" cannot
  interfere; settings hit areas are non-focusable.
- `--keys Down,Right,...` replays key presses in screenshot mode only, for
  automated checks that don't take focus from the desktop.
- Unit tests for row movement, opacity steps, preset cycling and hex parsing.

## Security

- Never indexed, even with hidden files on: `.ssh`, `.gnupg`,
  `.password-store`, `.pki`, `.git`, `.cache`, `~/.local/share/keyrings`,
  `~/.local/share/Trash`.
- Pseudo filesystems rejected: `/proc`, `/sys`, `/dev`, `/run` (except
  `/run/media`), also when typed into the JSON by hand.
- Hidden files are never previewed (any path component starting with `.`).
- Symlinks are not followed while indexing; no config value is executed.

## Performance

- Indexer: one path allocation per entry instead of two, name checks before
  `file_type()`, visible trees first and hidden folders last so the 400k cap
  never drops real files.
- Package caches and runtimes skipped by default via `skip_dirs` (`.npm`,
  `.cargo`, `.rustup`, `flatpak`, `fnm`, `pnpm`, `pipx`, …).
- Benchmark (release, this machine): hidden off ~150k entries in ~165 ms;
  hidden on ~358k entries in ~465 ms (no longer hits the cap).
- Result refreshes pause while settings are open; search path gained no
  allocations.

## Open items

- **Confirm with a real keyboard** that Enter on the opacity row no longer
  breaks navigation (`./install.sh` on `fix/settings-ui`), then merge
  `fix/settings-ui` into `main`.
- Untested by hand: mouse paths (slider drag, swatch clicks, colour picker),
  typing in the hex and add-folder fields, closing the window right after a
  change.
- Optional: real-key test harness in a virtual X display (Xvfb + XTest), not
  run yet because it needs approval.
- Ranking: an exact-match folder named "Settings" outranks the Glint Settings
  result (pre-existing scoring behaviour).
- The repository has never been run through `rustfmt`; a one-off formatting
  commit would make future diffs cleaner.
