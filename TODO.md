# TODO

Open work for Glint, roughly in priority order.

## Memory

- [ ] **Phase F1 (approved):** call `mallopt(M_ARENA_MAX, 2)` at start-up, and
      `shrink_to_fit` + `malloc_trim(0)` after each index and on hide (glibc
      only, no new crate). Expected saving: 30–60 MB. Measure `Private_Dirty`
      before and after, and keep the ~25 ms show latency on X11.
- [ ] Compact index: a string arena with `u32` offsets (expected −20–30 MB).
- [ ] Call `ctx.forget_all_images()` on hide.
- [ ] Drop the unused emoji font.

## Platform

- [ ] Wayland: measure show latency in a real session with `GLINT_TRACE=1`.
      (It already works in a real Plasma Wayland session.)
- [ ] Test on GNOME Wayland.

## Search

- [ ] A calculator query such as `2*3` is also read as a wildcard pattern and
      can list names like `2023`. The calculator result still shows first;
      decide whether to skip the pattern when the query is a valid expression.
- [ ] An exact-match "Settings" folder outranks Glint Settings.

## Settings page

- [ ] Test the mouse paths on the settings page by hand.

## Build and release

- [ ] `cargo fmt --check` fails on `src/config.rs` and `src/draw.rs`. Format
      them, then add a `cargo fmt --check` step to CI.
- [ ] Dependabot PR #1 (`x11rb` 0.13 → 0.14): rebase and check for API changes.
- [ ] Sign releases or add build provenance
      (`actions/attest-build-provenance`); `SHA256SUMS` only protects the
      download, not its origin.

## Housekeeping

- [ ] Delete the merged branches `fix/settings-ui`, `feat/settings` and
      `perf/instant-open`.
