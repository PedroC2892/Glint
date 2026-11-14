// SPDX-License-Identifier: GPL-3.0-or-later
//! The settings page: grouped cards of rows, driven by its own row focus
//! (not egui's widget focus) so keyboard navigation can never get trapped.

use crate::config::Settings;
use crate::draw::{self, CARD_FILL, CARD_STROKE, FG, MUTED, SEL_FILL};
use crate::ui::{BAR_H, Glint};
use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align, Color32, FontId, Id, Key, Layout, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    UiBuilder, Vec2,
};
use std::path::Path;

const ROW_H: f32 = 40.0;
const ROW_SUB_H: f32 = 48.0;
const OFF: Color32 = Color32::from_rgb(70, 70, 76);
const OFF_RING: Color32 = Color32::from_rgb(110, 110, 118);
const SIDE: f32 = 20.0;

/// One focusable row of the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SRow {
    KeepRunning,
    Accent,
    Custom,
    Opacity,
    Hidden,
    Outside,
    Root(usize),
    AddFolder,
    Online,
    Reset,
}

/// Number of rows currently shown (extra roots only appear with "outside home").
pub(crate) fn row_count(st: &Settings) -> usize {
    6 + if st.outside_home {
        st.extra_roots.len() + 1
    } else {
        0
    } + 2
}

/// The row at a position of the focus order; `None` past the end.
pub(crate) fn row_at(st: &Settings, idx: usize) -> Option<SRow> {
    match idx {
        0 => Some(SRow::KeepRunning),
        1 => Some(SRow::Accent),
        2 => Some(SRow::Custom),
        3 => Some(SRow::Opacity),
        4 => Some(SRow::Hidden),
        5 => Some(SRow::Outside),
        _ => {
            let mut k = idx - 6;
            if st.outside_home {
                if k < st.extra_roots.len() {
                    return Some(SRow::Root(k));
                }
                if k == st.extra_roots.len() {
                    return Some(SRow::AddFolder);
                }
                k -= st.extra_roots.len() + 1;
            }
            match k {
                0 => Some(SRow::Online),
                1 => Some(SRow::Reset),
                _ => None,
            }
        }
    }
}

/// Moves the focused row without wrapping.
pub(crate) fn move_row(cur: usize, len: usize, delta: i32) -> usize {
    (cur as i64 + delta as i64).clamp(0, len.saturating_sub(1) as i64) as usize
}

/// Steps opacity to the next 5% mark in `dir`, within 50..=100%.
pub(crate) fn step_opacity(v: f32, dir: i32) -> f32 {
    let steps = v * 20.0;
    let next = if dir > 0 {
        (steps + 1e-3).floor() + 1.0
    } else {
        (steps - 1e-3).ceil() - 1.0
    };
    next.clamp(10.0, 20.0) / 20.0
}

/// Next (or previous) accent preset, wrapping; a custom colour jumps to an end.
pub(crate) fn cycle_preset(current: [u8; 3], dir: i32) -> [u8; 3] {
    let p = draw::ACCENT_PRESETS;
    let n = p.len() as i32;
    match p.iter().position(|(_, c)| *c == current) {
        Some(i) => p[(i as i32 + dir.signum()).rem_euclid(n) as usize].1,
        None if dir > 0 => p[0].1,
        None => p[p.len() - 1].1,
    }
}

/// Parses "#RRGGBB" (the hash is optional).
pub(crate) fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub(crate) fn format_hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn hex_id() -> Id {
    Id::unique("settings_hex")
}

fn root_id() -> Id {
    Id::unique("settings_new_root")
}

/// One-line galley that ends in an ellipsis when it does not fit.
fn galley(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    color: Color32,
    mono: bool,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let font = if mono {
        FontId::monospace(size)
    } else {
        FontId::proportional(size)
    };
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping {
        max_width: max_w.max(10.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// Per-frame state shared by the row helpers.
struct Cx {
    focus: usize,
    accent: Color32,
    scroll: bool,
    pressed: Option<usize>,
    n: usize,
}

/// Allocates a row inside a card, draws hover and focus, and returns the
/// row index, the padded content rect and the row response.
fn shell(ui: &mut egui::Ui, cx: &mut Cx, h: f32, last: bool) -> (usize, Rect, egui::Response) {
    let idx = cx.n;
    cx.n += 1;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::CLICK);
    let inner = rect.shrink2(Vec2::new(4.0, 2.0));
    let p = ui.painter();
    if idx == cx.focus {
        p.rect_filled(inner, 8, SEL_FILL);
        p.rect_stroke(
            inner,
            8,
            Stroke::new(1.0, cx.accent.gamma_multiply(0.7)),
            egui::StrokeKind::Inside,
        );
        if cx.scroll {
            ui.scroll_to_rect(rect.expand2(Vec2::new(0.0, 8.0)), None);
        }
    } else if ui.rect_contains_pointer(rect) {
        p.rect_filled(inner, 8, SEL_FILL.gamma_multiply(0.45));
    }
    if !last {
        p.hline(
            rect.left() + 12.0..=rect.right() - 12.0,
            rect.bottom() - 0.5,
            Stroke::new(1.0, CARD_STROKE),
        );
    }
    if resp.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
        cx.pressed = Some(idx);
    }
    (idx, rect.shrink2(Vec2::new(12.0, 0.0)), resp)
}

/// Title (and optional subtitle) at the left of a row, vertically centred.
fn labels(ui: &egui::Ui, c: Rect, reserve: f32, title: &str, sub: Option<&str>) {
    let w = c.width() - reserve;
    let t = galley(ui, title, 13.5, FG, false, w);
    let s = sub.map(|s| galley(ui, s, 11.5, MUTED, false, w));
    let total = t.size().y + s.as_ref().map_or(0.0, |s| s.size().y + 2.0);
    let y = c.center().y - total / 2.0;
    let ty = t.size().y;
    ui.painter().galley(Pos2::new(c.left(), y), t, FG);
    if let Some(s) = s {
        ui.painter()
            .galley(Pos2::new(c.left(), y + ty + 2.0), s, MUTED);
    }
}

fn card(ui: &mut egui::Ui, h: f32, add: impl FnOnce(&mut egui::Ui)) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    ui.painter().rect_filled(rect, 10, CARD_FILL);
    ui.painter().rect_stroke(
        rect,
        10,
        Stroke::new(1.0, CARD_STROKE),
        egui::StrokeKind::Inside,
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.spacing_mut().item_spacing = Vec2::ZERO;
    add(&mut child);
}

fn heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(
            RichText::new(title.to_uppercase())
                .size(11.0)
                .strong()
                .color(MUTED),
        );
    });
    ui.add_space(5.0);
}

/// Paints the pill switch at the right edge of a row.
fn switch(ui: &egui::Ui, c: Rect, id: &str, on: bool, accent: Color32) {
    let t = ui.ctx().animate_bool(Id::unique(id), on);
    let r = Rect::from_center_size(
        Pos2::new(c.right() - 18.0, c.center().y),
        Vec2::new(36.0, 20.0),
    );
    let p = ui.painter();
    p.rect_filled(r, 10, OFF.lerp_to_gamma(accent, t));
    p.rect_stroke(
        r,
        10,
        Stroke::new(1.0, OFF_RING.gamma_multiply(1.0 - t)),
        egui::StrokeKind::Inside,
    );
    let x = r.left() + 10.0 + 16.0 * t;
    p.circle_filled(Pos2::new(x, r.center().y), 8.0, Color32::WHITE);
}

impl Glint {
    /// Applies a change: clean up values, schedule the debounced save, restyle.
    fn touch(&mut self, ctx: &egui::Context, restyle: bool) {
        self.settings.sanitize();
        self.settings_dirty = Some(ctx.input(|i| i.time));
        if restyle {
            self.restyle(ctx);
        }
    }

    /// Moves the page's navigation keys out of the raw input, so egui's own
    /// focus movement and Enter/Space "clicks" never see them. Text rows keep
    /// Left/Right/Enter/Space for editing; Up/Down/Tab are always taken.
    pub(crate) fn take_nav_keys(&mut self, raw: &mut egui::RawInput) {
        let text = matches!(
            row_at(&self.settings, self.settings_row),
            Some(SRow::Custom | SRow::AddFolder)
        );
        let queue = &mut self.nav_keys;
        raw.events.retain(|e| {
            let egui::Event::Key {
                key,
                pressed,
                modifiers: m,
                ..
            } = e
            else {
                return true;
            };
            let plain = !(m.ctrl || m.alt || m.command || m.mac_cmd);
            let take = match key {
                Key::ArrowUp | Key::ArrowDown => plain && !m.shift,
                Key::Tab => plain,
                Key::ArrowLeft | Key::ArrowRight | Key::Enter | Key::Space => {
                    plain && !m.shift && !text
                }
                _ => false,
            };
            if take && *pressed {
                queue.push((*key, m.shift));
            }
            !take
        });
    }

    /// Settings-page keys, applied before any widget runs.
    pub(crate) fn settings_keys(&mut self, ctx: &egui::Context) {
        let n = row_count(&self.settings);
        self.settings_row = self.settings_row.min(n - 1);
        for (key, shift) in std::mem::take(&mut self.nav_keys) {
            let row = row_at(&self.settings, self.settings_row);
            let text = matches!(row, Some(SRow::Custom | SRow::AddFolder));
            let (dir, act) = match key {
                Key::ArrowLeft if !text => (-1, false),
                Key::ArrowRight if !text => (1, false),
                Key::Enter | Key::Space if !text => (0, true),
                Key::ArrowUp | Key::ArrowDown | Key::Tab => (0, false),
                _ => continue,
            };
            let step = match key {
                Key::ArrowUp => -1,
                Key::ArrowDown => 1,
                Key::Tab if shift => -1,
                Key::Tab => 1,
                _ => 0,
            };
            if step != 0 {
                let moved = move_row(self.settings_row, n, step);
                if moved != self.settings_row {
                    self.settings_row = moved;
                    self.scroll_row = true;
                }
                continue;
            }
            self.adjust_row(ctx, row, dir, act);
        }
        // Text fields get egui focus only while their row is the focused one;
        // nothing else may keep it.
        let row = row_at(&self.settings, self.settings_row);
        ctx.memory_mut(|m| {
            for (id, want) in [
                (hex_id(), row == Some(SRow::Custom)),
                (root_id(), row == Some(SRow::AddFolder)),
            ] {
                if want {
                    m.request_focus(id);
                } else {
                    m.surrender_focus(id);
                }
            }
            if let Some(f) = m.focused()
                && f != hex_id()
                && f != root_id()
            {
                m.surrender_focus(f);
            }
        });
    }

    /// Left/Right (`dir`) or Enter/Space (`act`) on a row.
    fn adjust_row(&mut self, ctx: &egui::Context, row: Option<SRow>, dir: i32, act: bool) {
        match row {
            Some(SRow::KeepRunning) if act => {
                self.settings.keep_running ^= true;
                self.touch(ctx, false);
            }
            Some(SRow::Accent) if dir != 0 || act => {
                self.settings.accent =
                    cycle_preset(self.settings.accent, if dir != 0 { dir } else { 1 });
                self.touch(ctx, true);
            }
            Some(SRow::Opacity) if dir != 0 => {
                self.settings.opacity = step_opacity(self.settings.opacity, dir);
                self.touch(ctx, true);
            }
            Some(SRow::Hidden) if act => {
                self.settings.show_hidden ^= true;
                self.touch(ctx, false);
            }
            Some(SRow::Outside) if act => {
                self.settings.outside_home ^= true;
                self.touch(ctx, false);
            }
            Some(SRow::Online) if act => {
                self.settings.online ^= true;
                self.touch(ctx, false);
            }
            Some(SRow::Root(k)) if act => {
                self.settings.extra_roots.remove(k);
                self.touch(ctx, false);
            }
            Some(SRow::Reset) if act => {
                self.settings = Settings::default();
                self.touch(ctx, true);
            }
            _ => {}
        }
    }

    /// The settings page: title bar plus a scrolling body. Every change marks the
    /// settings dirty; the write to disk is debounced in `window_logic`.
    pub(crate) fn settings_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        ui.allocate_ui_with_layout(
            Vec2::new(full.width(), BAR_H),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Settings").size(20.0).color(FG));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(20.0);
                    ui.label(
                        RichText::new("↑↓ move · ←→ adjust · Enter toggle · Esc back")
                            .monospace()
                            .size(12.0)
                            .color(MUTED),
                    );
                });
            },
        );
        ui.painter().hline(
            full.x_range(),
            full.top() + BAR_H,
            Stroke::new(1.0, draw::HAIRLINE),
        );
        let body = Rect::from_min_max(
            Pos2::new(full.left() + SIDE, full.top() + BAR_H + 1.0),
            Pos2::new(full.right() - SIDE, full.bottom() - 6.0),
        );
        self.settings_row = self.settings_row.min(row_count(&self.settings) - 1);
        let mut cx = Cx {
            focus: self.settings_row,
            accent: self.theme.accent,
            scroll: std::mem::take(&mut self.scroll_row),
            pressed: None,
            n: 0,
        };
        let (mut changed, mut restyle) = (false, false);
        let mut msgs: Vec<String> = Vec::new();
        let differs = self.settings.index_differs(&self.index_snapshot);
        let (st, hex, new_root) = (&mut self.settings, &mut self.hex, &mut self.new_root);
        let accent = cx.accent;
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body);
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let cx = &mut cx;

                heading(ui, "General");
                card(ui, ROW_SUB_H, |ui| {
                    let (_, c, r) = shell(ui, cx, ROW_SUB_H, true);
                    labels(
                        ui,
                        c,
                        56.0,
                        "Keep Glint running in the background",
                        Some("Opens instantly. Off: starts on demand and frees its memory when closed"),
                    );
                    switch(ui, c, "sw_keep_running", st.keep_running, accent);
                    if r.clicked() {
                        st.keep_running ^= true;
                        changed = true;
                    }
                });

                heading(ui, "Appearance");
                card(ui, ROW_H * 2.0 + ROW_SUB_H, |ui| {
                    // Accent presets.
                    let (_, c, _) = shell(ui, cx, ROW_H, false);
                    labels(ui, c, 170.0, "Accent colour", None);
                    for (n, (name, rgb)) in draw::ACCENT_PRESETS.iter().enumerate() {
                        let centre = Pos2::new(c.right() - 9.0 - (draw::ACCENT_PRESETS.len() - 1 - n) as f32 * 26.0, c.center().y);
                        let resp = ui.interact(Rect::from_center_size(centre, Vec2::splat(24.0)), Id::unique(("accent", n)), Sense::CLICK);
                        ui.painter().circle_filled(centre, 9.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        if st.accent == *rgb {
                            ui.painter().circle_stroke(centre, 12.0, Stroke::new(2.0, Color32::WHITE));
                        }
                        if resp.on_hover_text(*name).clicked() {
                            st.accent = *rgb;
                            (changed, restyle) = (true, true);
                        }
                    }

                    // Custom colour: hex field plus the picker swatch.
                    let (_, c, _) = shell(ui, cx, ROW_H, false);
                    labels(ui, c, 190.0, "Custom colour", None);
                    let mut right = ui.new_child(UiBuilder::new().max_rect(c).layout(Layout::right_to_left(Align::Center)));
                    right.spacing_mut().item_spacing.x = 8.0;
                    right.spacing_mut().interact_size = Vec2::new(26.0, 22.0);
                    if color_button(&mut right, &mut st.accent) {
                        (changed, restyle) = (true, true);
                    }
                    if !right.memory(|m| m.has_focus(hex_id())) {
                        let f = format_hex(st.accent);
                        if *hex != f {
                            *hex = f;
                        }
                    }
                    let edit = egui::TextEdit::singleline(hex)
                        .id(hex_id())
                        .font(FontId::monospace(13.0))
                        .char_limit(8)
                        .desired_width(76.0)
                        .margin(Vec2::new(6.0, 4.0));
                    if right.add(edit).changed()
                        && let Some(rgb) = parse_hex(hex)
                        && rgb != st.accent
                    {
                        st.accent = rgb;
                        (changed, restyle) = (true, true);
                    }

                    // Opacity slider.
                    let (_, c, _) = shell(ui, cx, ROW_SUB_H, true);
                    labels(ui, c, 240.0, "Window opacity", Some("Needs a compositor below 100%"));
                    let value = galley(ui, &format!("{:.0}%", st.opacity * 100.0), 13.0, FG, false, 40.0);
                    let vw = value.size().x;
                    ui.painter().galley(Pos2::new(c.right() - vw, c.center().y - value.size().y / 2.0), value, FG);
                    let (right_x, cy) = (c.right() - 48.0, c.center().y);
                    let left_x = right_x - 160.0;
                    let hit = Rect::from_min_max(Pos2::new(left_x - 8.0, cy - 12.0), Pos2::new(right_x + 8.0, cy + 12.0));
                    let resp = ui.interact(hit, Id::unique("opacity_slider"), Sense::CLICK | Sense::DRAG);
                    if resp.is_pointer_button_down_on()
                        && let Some(pos) = resp.interact_pointer_pos()
                    {
                        let t = ((pos.x - left_x) / 160.0).clamp(0.0, 1.0);
                        let v = ((0.5 + 0.5 * t) * 20.0).round() / 20.0;
                        if (v - st.opacity).abs() > 1e-4 {
                            st.opacity = v;
                            (changed, restyle) = (true, true);
                        }
                    }
                    let knob_x = left_x + 160.0 * ((st.opacity - 0.5) / 0.5).clamp(0.0, 1.0);
                    let p = ui.painter();
                    p.rect_filled(Rect::from_min_max(Pos2::new(left_x, cy - 2.0), Pos2::new(right_x, cy + 2.0)), 2, OFF);
                    p.rect_filled(Rect::from_min_max(Pos2::new(left_x, cy - 2.0), Pos2::new(knob_x, cy + 2.0)), 2, accent);
                    p.circle_filled(Pos2::new(knob_x, cy), 7.0, Color32::WHITE);
                });

                heading(ui, "Files");
                let roots = if st.outside_home { st.extra_roots.len() + 1 } else { 0 };
                card(ui, ROW_SUB_H * 2.0 + ROW_H * roots as f32, |ui| {
                    let (_, c, r) = shell(ui, cx, ROW_SUB_H, false);
                    labels(ui, c, 56.0, "Show hidden files", Some(".ssh, .gnupg and .git are never indexed; hidden files are never previewed"));
                    switch(ui, c, "sw_hidden", st.show_hidden, accent);
                    if r.clicked() {
                        st.show_hidden ^= true;
                        changed = true;
                    }

                    let (_, c, r) = shell(ui, cx, ROW_SUB_H, roots == 0);
                    labels(ui, c, 56.0, "Index folders outside home", Some("Unreadable folders are skipped"));
                    switch(ui, c, "sw_outside", st.outside_home, accent);
                    if r.clicked() {
                        st.outside_home ^= true;
                        changed = true;
                    }

                    if st.outside_home {
                        let mut remove = None;
                        for k in 0..st.extra_roots.len() {
                            let (_, c, _) = shell(ui, cx, ROW_H, false);
                            let path = st.extra_roots[k].display().to_string();
                            let g = galley(ui, &path, 12.5, FG, true, c.width() - 80.0);
                            ui.painter().galley(Pos2::new(c.left(), c.center().y - g.size().y / 2.0), g, FG);
                            let b = Rect::from_center_size(Pos2::new(c.right() - 28.0, c.center().y), Vec2::new(56.0, 26.0));
                            let resp = ui.interact(b, Id::unique(("remove", k)), Sense::CLICK);
                            let col = if resp.hovered() { FG } else { MUTED };
                            if resp.hovered() {
                                ui.painter().rect_filled(b, 6, SEL_FILL);
                            }
                            ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, "Remove", FontId::proportional(12.0), col);
                            if resp.clicked() {
                                remove = Some(k);
                            }
                        }
                        if let Some(k) = remove {
                            st.extra_roots.remove(k);
                            changed = true;
                        }
                        let (_, c, _) = shell(ui, cx, ROW_H, true);
                        let edit = egui::TextEdit::singleline(new_root)
                            .id(root_id())
                            .font(FontId::monospace(12.5))
                            .hint_text("/path/to/folder")
                            .desired_width(c.width())
                            .margin(Vec2::new(8.0, 5.0));
                        let resp = ui.put(Rect::from_center_size(c.center(), Vec2::new(c.width(), 28.0)), edit);
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if enter && !new_root.trim().is_empty() {
                            match crate::ui::validate_root(new_root, &st.extra_roots) {
                                Ok(root) => {
                                    if root == Path::new("/") || root == Path::new("/usr") {
                                        msgs.push("Indexing system folders is slow and fills the index".into());
                                    }
                                    st.extra_roots.push(root);
                                    new_root.clear();
                                    changed = true;
                                }
                                Err(e) => msgs.push(e),
                            }
                        }
                    }
                });

                heading(ui, "Search");
                card(ui, ROW_SUB_H, |ui| {
                    let (_, c, r) = shell(ui, cx, ROW_SUB_H, true);
                    labels(ui, c, 56.0, "Online lookups", Some("Definitions and exchange rates"));
                    switch(ui, c, "sw_online", st.online, accent);
                    if r.clicked() {
                        st.online ^= true;
                        changed = true;
                    }
                });

                // Reset button at the left, a note at the right.
                ui.add_space(12.0);
                let idx = cx.n;
                cx.n += 1;
                let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
                let pill = Rect::from_min_size(row.min, Vec2::new(138.0, 34.0));
                let resp = ui.interact(pill, Id::unique("reset"), Sense::CLICK);
                let focused = idx == cx.focus;
                let p = ui.painter();
                p.rect_filled(pill, 8, if focused { SEL_FILL } else if resp.hovered() { SEL_FILL.gamma_multiply(0.6) } else { CARD_FILL });
                p.rect_stroke(pill, 8, Stroke::new(1.0, if focused { accent.gamma_multiply(0.7) } else { CARD_STROKE }), egui::StrokeKind::Inside);
                p.text(pill.center(), egui::Align2::CENTER_CENTER, "Reset to defaults", FontId::proportional(13.0), FG);
                if focused && cx.scroll {
                    ui.scroll_to_rect(row.expand2(Vec2::new(0.0, 8.0)), None);
                }
                if resp.is_pointer_button_down_on() {
                    cx.pressed = Some(idx);
                }
                if resp.clicked() {
                    *st = Settings::default();
                    (changed, restyle) = (true, true);
                }
                if differs {
                    let g = galley(ui, "Changes apply when you leave settings", 11.5, MUTED, false, row.width() - 160.0);
                    let w = g.size().x;
                    ui.painter().galley(Pos2::new(row.right() - w, row.center().y - g.size().y / 2.0), g, MUTED);
                }
                ui.add_space(8.0);
            });
        });
        if let Some(i) = cx.pressed {
            self.settings_row = i;
        }
        if changed {
            self.touch(&ctx, restyle);
        }
        if let Some(m) = msgs.last() {
            self.toast(&ctx, m);
        }
    }
}

/// egui's colour picker button; true when the colour changed.
fn color_button(ui: &mut egui::Ui, rgb: &mut [u8; 3]) -> bool {
    ui.color_edit_button_srgb(rgb).changed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_steps_on_the_grid() {
        assert_eq!(step_opacity(0.92, 1), 0.95);
        assert_eq!(step_opacity(0.92, -1), 0.90);
        assert_eq!(step_opacity(0.90, 1), 0.95);
        assert_eq!(step_opacity(0.90, -1), 0.85);
        assert_eq!(step_opacity(1.0, 1), 1.0);
        assert_eq!(step_opacity(0.5, -1), 0.5);
    }

    #[test]
    fn presets_cycle_and_wrap() {
        let p = draw::ACCENT_PRESETS;
        assert_eq!(cycle_preset(p[0].1, 1), p[1].1);
        assert_eq!(cycle_preset(p[0].1, -1), p[p.len() - 1].1);
        assert_eq!(cycle_preset(p[p.len() - 1].1, 1), p[0].1);
        assert_eq!(cycle_preset([1, 2, 3], 1), p[0].1);
        assert_eq!(cycle_preset([1, 2, 3], -1), p[p.len() - 1].1);
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#3478F6"), Some([0x34, 0x78, 0xF6]));
        assert_eq!(parse_hex(" 3478f6 "), Some([0x34, 0x78, 0xF6]));
        assert_eq!(parse_hex("#3478F"), None);
        assert_eq!(parse_hex("#34789G"), None);
        assert_eq!(parse_hex("#3478F6A"), None);
        assert_eq!(format_hex([0x34, 0x78, 0xF6]), "#3478F6");
        assert_eq!(parse_hex(&format_hex([1, 2, 3])), Some([1, 2, 3]));
    }

    #[test]
    fn rows_do_not_wrap() {
        assert_eq!(move_row(0, 8, -1), 0);
        assert_eq!(move_row(7, 8, 1), 7);
        assert_eq!(move_row(3, 8, 1), 4);
        assert_eq!(move_row(0, 0, 1), 0);
    }

    #[test]
    fn visible_rows_follow_settings() {
        let mut st = Settings {
            extra_roots: vec!["/mnt".into(), "/media".into()],
            ..Settings::default()
        };
        st.outside_home = false;
        assert_eq!(row_count(&st), 8);
        assert_eq!(row_at(&st, 0), Some(SRow::KeepRunning));
        assert_eq!(row_at(&st, 1), Some(SRow::Accent));
        assert_eq!(row_at(&st, 6), Some(SRow::Online));
        assert_eq!(row_at(&st, 7), Some(SRow::Reset));
        assert_eq!(row_at(&st, 8), None);
        st.outside_home = true;
        assert_eq!(row_count(&st), 11);
        assert_eq!(row_at(&st, 0), Some(SRow::KeepRunning));
        assert_eq!(row_at(&st, 1), Some(SRow::Accent));
        assert_eq!(row_at(&st, 6), Some(SRow::Root(0)));
        assert_eq!(row_at(&st, 7), Some(SRow::Root(1)));
        assert_eq!(row_at(&st, 8), Some(SRow::AddFolder));
        assert_eq!(row_at(&st, 9), Some(SRow::Online));
        assert_eq!(row_at(&st, 10), Some(SRow::Reset));
        assert_eq!(row_at(&st, 11), None);
    }
}
