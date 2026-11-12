//! Drawing: the result list, the preview pane and small painted icons
//! (no image assets needed), plus saving the screenshot.

use crate::index::{self, Kind};
use crate::open::Fetch;
use crate::ui::{Cat, Item, Lookout};
use eframe::egui::{self, Color32, CornerRadius, FontId, Label, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Vec2};
use std::path::Path;

// Slate-blue glass, light text (colours are premultiplied where translucent).
pub const BG: Color32 = Color32::from_rgba_premultiplied(40, 45, 60, 242);
pub const BORDER: Color32 = Color32::from_rgba_premultiplied(48, 48, 48, 48);
pub const FG: Color32 = Color32::from_rgb(242, 244, 248);
pub const MUTED: Color32 = Color32::from_rgb(168, 174, 188);
pub const ACCENT: Color32 = Color32::from_rgb(10, 100, 220);
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
const SEL_FILL: Color32 = Color32::from_rgba_premultiplied(30, 32, 36, 34);
const SEL_STROKE: Color32 = Color32::from_rgba_premultiplied(62, 62, 62, 62);
const CARD_FILL: Color32 = Color32::from_rgba_premultiplied(14, 15, 18, 16);
const HEAD: Color32 = Color32::from_rgb(196, 201, 212);

fn text(t: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(t).size(size).color(color)
}

/// The rounded glass panel behind everything.
pub fn panel(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 14, BG);
    // A faint top sheen, like light on glass.
    let sheen = Rect::from_min_max(r.min, Pos2::new(r.right(), r.top() + 70.0f32.min(r.height())));
    p.rect_filled(sheen, CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }, Color32::from_rgba_premultiplied(5, 6, 8, 6));
    p.rect_stroke(r.shrink(0.5), 14, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
}

impl Lookout {
    fn icon(&mut self, ui: &egui::Ui, rect: Rect, item: &Item) {
        let p = ui.painter();
        match item {
            Item::App(i) => {
                let i = *i;
                let apps = &self.apps;
                let path = self.icons.entry(i).or_insert_with(|| index::find_icon(&apps[i].icon)).clone();
                match path {
                    Some(path) => egui::Image::new(format!("file://{}", path.display())).paint_at(ui, rect),
                    None => {
                        let name = &self.apps[i].name;
                        let letter = name.chars().next().unwrap_or('?').to_uppercase().to_string();
                        tile(p, rect, letter_color(name), &letter);
                    }
                }
            }
            Item::File(_, true) => folder(p, rect),
            Item::File(path, false) => {
                let e = index::ext_lc(path);
                let color = match index::kind(path, false) {
                    Kind::Image => Color32::from_rgb(76, 175, 120),
                    Kind::Music => Color32::from_rgb(232, 72, 120),
                    Kind::Movie => Color32::from_rgb(140, 90, 230),
                    _ if e == "pdf" => Color32::from_rgb(220, 70, 60),
                    _ => Color32::from_rgb(120, 130, 150),
                };
                page(p, rect, color, &e);
            }
            Item::Calc { .. } => tile(p, rect, Color32::from_rgb(242, 150, 30), "="),
            Item::Convert { .. } | Item::PendingRates => tile(p, rect, Color32::from_rgb(40, 160, 160), "⇄"),
            Item::Define(_) => tile(p, rect, Color32::from_rgb(170, 60, 50), "Aa"),
            Item::Web(_) => globe(p, rect),
        }
    }

    fn row_title(&self, item: &Item) -> String {
        match item {
            Item::App(i) => self.apps[*i].name.clone(),
            Item::File(p, _) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Item::Calc { value, .. } => format!("= {value}"),
            Item::Convert { result, .. } => result.clone(),
            Item::PendingRates => "Converting…".into(),
            Item::Define(w) => w.clone(),
            Item::Web(q) => q.clone(),
        }
    }

    /// Second line under a result: kind and date, the sum, the definition…
    fn subtitle(&mut self, item: &Item) -> String {
        match item {
            Item::App(i) if self.apps[*i].settings => "System Settings".into(),
            Item::App(_) => "Application".into(),
            Item::File(p, is_dir) => {
                let offset = self.utc_offset;
                self.subtitles
                    .entry(p.clone())
                    .or_insert_with(|| {
                        let kind = index::kind_text(p, *is_dir);
                        match std::fs::metadata(p).and_then(|m| m.modified()) {
                            Ok(t) => format!("{kind} · Modified {}", index::format_time(t, offset, std::time::SystemTime::now())),
                            Err(_) => kind,
                        }
                    })
                    .clone()
            }
            Item::Calc { expr, .. } => expr.clone(),
            Item::Convert { detail, .. } => detail.clone(),
            Item::PendingRates => match &*self.rates.lock().unwrap() {
                Some(Fetch::Failed(e)) => format!("Exchange rates are unavailable: {e}"),
                _ => "Getting today’s exchange rates…".into(),
            },
            Item::Define(w) => match self.defs.lock().unwrap().get(w) {
                Some(Fetch::Ready(d)) => d.meanings.iter().flat_map(|(_, v)| v.first()).next().cloned().unwrap_or_default(),
                Some(Fetch::Failed(e)) => e.clone(),
                Some(Fetch::Loading) => "Looking it up…".into(),
                None => "Select to look up the definition".into(),
            },
            Item::Web(_) => String::new(),
        }
    }

    pub fn list(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(8.0);
            let mut last: Option<Cat> = None;
            let rows = self.rows.clone();
            for (i, row) in rows.iter().enumerate() {
                let first_in_section = last != Some(row.cat);
                if first_in_section && !row.cat.is_card() {
                    self.section_header(ui, &ctx, row.cat);
                }
                last = Some(row.cat);
                let selected = i == self.sel;
                if selected && let Item::Define(w) = &row.item {
                    self.start_definition(&ctx, w); // Only on purpose, never while typing.
                }
                let h = row.cat.row_height();
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
                let card = row.cat.is_card();
                let inner = if card { rect.shrink2(Vec2::new(12.0, 5.0)) } else { rect.shrink2(Vec2::new(10.0, 1.0)) };
                let p = ui.painter();
                if card {
                    p.rect_filled(inner, 10, if selected { SEL_FILL } else { CARD_FILL });
                    p.rect_stroke(inner, 10, Stroke::new(1.0, if selected { SEL_STROKE } else { HAIRLINE }), egui::StrokeKind::Inside);
                } else if selected {
                    p.rect_filled(inner, 7, SEL_FILL);
                    p.rect_stroke(inner, 7, Stroke::new(1.0, SEL_STROKE), egui::StrokeKind::Inside);
                } else {
                    if resp.hovered() {
                        p.rect_filled(inner, 7, CARD_FILL);
                    }
                    let prev_selected = i > 0 && i - 1 == self.sel;
                    if !first_in_section && !prev_selected && row.cat != Cat::Web {
                        p.hline(inner.left() + 4.0..=inner.right() - 4.0, rect.top(), Stroke::new(1.0, HAIRLINE));
                    }
                }
                let icon_size = match row.cat {
                    _ if card => 34.0,
                    Cat::Web => 16.0,
                    _ => 28.0,
                };
                let icon = Rect::from_center_size(Pos2::new(inner.left() + 10.0 + icon_size / 2.0, inner.center().y), Vec2::splat(icon_size));
                if row.cat == Cat::Web {
                    magnifier(ui.painter(), icon.shrink(1.0), MUTED);
                    let arrow = Pos2::new(inner.right() - 12.0, inner.center().y);
                    ui.painter().text(arrow, egui::Align2::CENTER_CENTER, "↗", FontId::proportional(13.0), MUTED);
                } else {
                    self.icon(ui, icon, &row.item);
                }
                let right = inner.right() - if row.cat == Cat::Web { 28.0 } else { 10.0 };
                let text_rect = Rect::from_min_max(Pos2::new(icon.right() + 11.0, inner.top()), Pos2::new(right, inner.bottom()));
                let title = self.row_title(&row.item);
                let sub = self.subtitle(&row.item);
                let (ts, ss) = if card { (16.0, 12.5) } else { (13.5, 11.5) };
                let layout = egui::Layout::top_down(egui::Align::Min);
                ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect).layout(layout), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let lines = if sub.is_empty() { ts } else { ts + ss + 6.0 };
                    ui.add_space(((text_rect.height() - lines) / 2.0 - 1.0).max(0.0));
                    ui.add(Label::new(text(title, ts, FG)).truncate());
                    if !sub.is_empty() {
                        ui.add(Label::new(text(sub, ss, MUTED)).truncate());
                    }
                });
                if selected && self.scroll_to_sel {
                    resp.scroll_to_me(None);
                    self.scroll_to_sel = false;
                }
                if resp.clicked() {
                    self.sel = i;
                }
                if resp.double_clicked() {
                    self.sel = i;
                    self.activate(&ctx, false);
                }
            }
            ui.add_space(8.0);
        });
    }

    /// "Folders ……… Show More ↗" with a hairline under it.
    fn section_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, cat: Cat) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), crate::ui::HEAD_H), Sense::hover());
        let inner = rect.shrink2(Vec2::new(12.0, 0.0));
        let base = inner.bottom() - 8.0;
        let p = ui.painter();
        p.text(Pos2::new(inner.left(), base), egui::Align2::LEFT_BOTTOM, cat.title(), FontId::proportional(13.0), HEAD);
        p.hline(inner.x_range(), inner.bottom() - 2.0, Stroke::new(1.0, HAIRLINE));
        if self.more.contains(&cat) {
            let label = "Show More ↗";
            let galley = p.layout_no_wrap(label.into(), FontId::proportional(12.5), HEAD);
            let r = Rect::from_min_size(Pos2::new(inner.right() - galley.size().x, base - galley.size().y), galley.size());
            let resp = ui.interact(r.expand(4.0), ui.scope_id().with(("show-more", cat)), Sense::click());
            let color = if resp.hovered() { FG } else { HEAD };
            ui.painter().galley(r.min, galley, color);
            if resp.clicked() {
                self.expand(ctx, cat);
            }
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, item: &Item, title: &str, kind: &str) {
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
            self.icon(ui, r, item);
            ui.add_space(10.0);
            ui.add(Label::new(text(title, 18.0, FG).strong()).wrap());
            ui.label(text(kind, 12.5, MUTED));
            ui.add_space(10.0);
        });
        hairline(ui);
    }

    pub fn preview(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(row) = self.rows.get(self.sel).cloned() else { return };
        match &row.item {
            Item::App(i) => {
                let a = &self.apps[*i];
                let (name, generic, comment) = (a.name.clone(), a.generic.clone(), a.comment.clone());
                let (path, key) = (a.path.clone(), format!("app:{}", a.id));
                let kind = if a.settings { "System Settings" } else { "Application" };
                self.header(ui, &row.item, &name, kind);
                if !comment.is_empty() {
                    ui.add_space(8.0);
                    ui.add(Label::new(text(comment, 13.0, FG)).wrap());
                }
                ui.add_space(8.0);
                if !generic.is_empty() {
                    info_line(ui, "Type", &generic);
                }
                let n = self.history.get(&key).copied().unwrap_or(0);
                info_line(ui, "Opened", &format!("{n} time{} from here", if n == 1 { "" } else { "s" }));
                info_line(ui, "Entry", &path.to_string_lossy());
            }
            Item::File(path, is_dir) => self.file_preview(ui, &row.item, path, *is_dir),
            Item::Calc { expr, value } => {
                ui.add_space(40.0);
                ui.label(text(expr, 16.0, MUTED));
                ui.add(Label::new(text(format!("= {value}"), 38.0, FG).strong()).wrap());
                ui.add_space(16.0);
                ui.label(text("Press Enter or Ctrl+C to copy the result.", 12.5, MUTED));
            }
            Item::Convert { result, detail } => {
                ui.add_space(40.0);
                ui.add(Label::new(text(result, 34.0, FG).strong()).wrap());
                ui.add_space(6.0);
                ui.add(Label::new(text(detail, 13.0, MUTED)).wrap());
                ui.add_space(16.0);
                ui.label(text("Press Enter or Ctrl+C to copy the result.", 12.5, MUTED));
                if detail.contains('·') {
                    ui.label(text("Live rates from open.er-api.com, updated daily.", 11.0, MUTED));
                }
            }
            Item::PendingRates => {
                ui.add_space(40.0);
                let failed = match &*self.rates.lock().unwrap() {
                    Some(Fetch::Failed(e)) => Some(e.clone()),
                    _ => None,
                };
                match failed {
                    Some(e) => {
                        ui.label(text("Exchange rates are unavailable", 18.0, FG));
                        ui.add(Label::new(text(e, 12.0, MUTED)).wrap());
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(text("Getting today’s exchange rates…", 15.0, FG));
                        });
                    }
                }
            }
            Item::Define(word) => {
                self.start_definition(&ctx, word);
                let state = self.defs.lock().unwrap().get(word).cloned();
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(14.0);
                    match state {
                        Some(Fetch::Ready(d)) => {
                            ui.label(text(&d.word, 26.0, FG).strong());
                            if !d.phonetic.is_empty() {
                                ui.label(text(&d.phonetic, 14.0, MUTED));
                            }
                            for (pos, defs) in &d.meanings {
                                ui.add_space(10.0);
                                ui.label(text(pos, 13.0, MUTED).italics());
                                for (n, def) in defs.iter().enumerate() {
                                    ui.add(Label::new(text(format!("{}. {def}", n + 1), 13.5, FG)).wrap());
                                    ui.add_space(2.0);
                                }
                            }
                            ui.add_space(10.0);
                            ui.label(text("From the Free Dictionary API · Enter opens Wiktionary", 11.0, MUTED));
                        }
                        Some(Fetch::Failed(e)) => {
                            ui.label(text(word, 26.0, FG).strong());
                            ui.add(Label::new(text(e, 13.0, MUTED)).wrap());
                        }
                        _ => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(text("Looking it up…", 15.0, FG));
                            });
                        }
                    }
                });
            }
            Item::Web(q) => {
                self.header(ui, &row.item, &format!("“{q}”"), "Web search");
                ui.add_space(10.0);
                let hint = "Press Enter (or Ctrl+B from anywhere) to search the web in your browser.";
                ui.add(Label::new(text(hint, 13.0, FG)).wrap());
            }
        }
    }

    fn file_preview(&mut self, ui: &mut egui::Ui, item: &Item, path: &Path, is_dir: bool) {
        if self.info.as_ref().is_none_or(|f| f.path != path) {
            self.info = Some(index::file_info(path, is_dir));
        }
        let name = self.row_title(item);
        let kind = index::kind_text(path, is_dir);
        if index::kind(path, is_dir) == Kind::Image {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.add(
                    egui::Image::new(format!("file://{}", path.display()))
                        .max_size(Vec2::new(ui.available_width(), 220.0))
                        .corner_radius(6),
                );
                ui.add_space(6.0);
                ui.add(Label::new(text(&name, 16.0, FG).strong()).wrap());
                ui.label(text(&kind, 12.5, MUTED));
                ui.add_space(8.0);
            });
            hairline(ui);
        } else {
            self.header(ui, item, &name, &kind);
        }
        ui.add_space(8.0);
        let Some(info) = self.info.as_ref() else { return };
        if let Some(s) = info.size {
            info_line(ui, "Size", &index::format_size(s));
        }
        if let Some(n) = info.items {
            info_line(ui, "Contains", &format!("{n} item{}", if n == 1 { "" } else { "s" }));
        }
        let now = std::time::SystemTime::now();
        if let Some(t) = info.created {
            info_line(ui, "Created", &index::format_time(t, self.utc_offset, now));
        }
        if let Some(t) = info.modified {
            info_line(ui, "Modified", &index::format_time(t, self.utc_offset, now));
        }
        let home = index::home().to_string_lossy().into_owned();
        let where_ = path.parent().map(|p| p.to_string_lossy().replacen(&home, "~", 1));
        info_line(ui, "Where", &where_.unwrap_or_default());
        if let Some(body) = &info.text {
            ui.add_space(8.0);
            egui::Frame::new().fill(Color32::from_black_alpha(60)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    ui.add(Label::new(RichText::new(body).monospace().size(11.0).color(FG)).wrap());
                });
            });
        }
    }
}

fn hairline(ui: &mut egui::Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, HAIRLINE));
    ui.add_space(2.0);
}

fn info_line(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.add_sized([72.0, 16.0], Label::new(text(key, 12.5, MUTED)));
        ui.add(Label::new(text(value, 12.5, FG)).wrap());
    });
}

fn letter_color(name: &str) -> Color32 {
    let palette = [
        Color32::from_rgb(66, 133, 244),
        Color32::from_rgb(219, 68, 55),
        Color32::from_rgb(244, 160, 0),
        Color32::from_rgb(15, 157, 88),
        Color32::from_rgb(171, 71, 188),
        Color32::from_rgb(0, 172, 193),
    ];
    palette[name.bytes().map(usize::from).sum::<usize>() % palette.len()]
}

fn tile(p: &egui::Painter, r: Rect, color: Color32, label: &str) {
    p.rect_filled(r, CornerRadius::same((r.width() * 0.22) as u8), color);
    p.text(r.center(), egui::Align2::CENTER_CENTER, label, FontId::proportional(r.height() * 0.55), Color32::WHITE);
}

pub fn magnifier(p: &egui::Painter, r: Rect, color: Color32) {
    let s = r.width();
    let c = r.min + Vec2::splat(s * 0.42);
    p.circle_stroke(c, s * 0.3, Stroke::new(2.2, color));
    p.line_segment([c + Vec2::splat(s * 0.22), r.max - Vec2::splat(s * 0.06)], Stroke::new(2.6, color));
}

fn folder(p: &egui::Painter, r: Rect) {
    let s = r.width();
    let back = Color32::from_rgb(60, 140, 230);
    let front = Color32::from_rgb(98, 173, 247);
    let round = CornerRadius::same((s * 0.07) as u8);
    let tab = Rect::from_min_size(r.min + Vec2::new(s * 0.05, s * 0.14), Vec2::new(s * 0.4, s * 0.2));
    p.rect_filled(tab, round, back);
    let body = Rect::from_min_max(r.min + Vec2::new(s * 0.05, s * 0.24), r.max - Vec2::new(s * 0.05, s * 0.12));
    p.rect_filled(body, round, back);
    p.rect_filled(Rect::from_min_max(body.min + Vec2::new(0.0, s * 0.1), body.max), round, front);
}

fn page(p: &egui::Painter, r: Rect, accent: Color32, ext: &str) {
    let s = r.width();
    let sheet = Rect::from_min_max(r.min + Vec2::new(s * 0.16, s * 0.04), r.max - Vec2::new(s * 0.16, s * 0.04));
    let rr = (s * 0.06) as u8;
    p.rect_filled(sheet, CornerRadius::same(rr), Color32::from_rgb(246, 246, 248));
    let band = Rect::from_min_max(Pos2::new(sheet.left(), sheet.bottom() - s * 0.3), sheet.max);
    p.rect_filled(band, CornerRadius { nw: 0, ne: 0, sw: rr, se: rr }, accent);
    if s >= 30.0 && !ext.is_empty() {
        let label: String = ext.to_uppercase().chars().take(4).collect();
        p.text(band.center(), egui::Align2::CENTER_CENTER, label, FontId::proportional(s * 0.16), Color32::WHITE);
    }
    let rule = Stroke::new((s * 0.03).max(1.0), Color32::from_rgb(200, 200, 208));
    for k in 0..3 {
        let y = sheet.top() + s * (0.18 + 0.1 * k as f32);
        p.hline(sheet.left() + s * 0.1..=sheet.right() - s * 0.1, y, rule);
    }
}

fn globe(p: &egui::Painter, r: Rect) {
    let c = r.center();
    let rad = r.width() * 0.45;
    p.circle_filled(c, rad, Color32::from_rgb(52, 120, 246));
    let st = Stroke::new((r.width() * 0.05).max(1.0), Color32::from_white_alpha(200));
    p.circle_stroke(c, rad * 0.98, st);
    p.line_segment([c - Vec2::new(rad, 0.0), c + Vec2::new(rad, 0.0)], st);
    p.line_segment([c - Vec2::new(0.0, rad), c + Vec2::new(0.0, rad)], st);
    let ellipse: Vec<Pos2> = (0..=24)
        .map(|k| {
            let a = k as f32 / 24.0 * std::f32::consts::TAU;
            c + Vec2::new(rad * 0.45 * a.cos(), rad * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(ellipse, st));
}

/// Save the picture as an RGBA PNG, keeping at most `max_rows` rows.
pub fn save_png(path: &Path, img: &egui::ColorImage, max_rows: usize) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let [w, full_h] = img.size;
    let h = full_h.min(max_rows.max(1));
    let data: Vec<u8> = img.pixels[..w * h].iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(&data))
        .map_err(|e| e.to_string())
}
