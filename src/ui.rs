//! The search window: query handling, results, keyboard, window behaviour.

use crate::calc;
use crate::config::{self, Settings};
use crate::draw::{self, FG, HAIRLINE, MUTED};
use crate::index::{self, AppEntry, FileEntry, FileInfo, Kind};
use crate::open::{self, Definition, Fetch};
use eframe::egui::{self, Align, FontId, Key, Layout, Modifiers, Pos2, Rect, RichText, Sense, Stroke, UiBuilder, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const WIDTH: f32 = 680.0;
pub const BAR_H: f32 = 54.0;
const MAX_H: f32 = 520.0;
const LIST_W: f32 = 350.0;
/// Words that make the "Glint Settings" result appear (typed prefixes count).
const SETTINGS_WORDS: &[&str] = &["glint settings", "settings", "preferences", "prefs", "config", "configuration", "options"];
/// How long a settings change waits before it is written to disk.
const SAVE_DELAY: f64 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Search,
    Settings,
}

// Row heights, shared by the list drawing and the panel-height estimate.
pub const CARD_H: f32 = 62.0;
pub const ROW_H: f32 = 44.0;
pub const WEB_H: f32 = 34.0;
pub const HEAD_H: f32 = 38.0;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Cat {
    Calculator,
    Conversion,
    TopHit,
    Web,
    Definition,
    Applications,
    Settings,
    Documents,
    Folders,
    Images,
    Music,
    Movies,
    Files,
}

impl Cat {
    pub fn title(self) -> &'static str {
        match self {
            Cat::Calculator => "Calculator",
            Cat::Conversion => "Conversion",
            Cat::TopHit => "Top Hit",
            Cat::Web => "Suggestions",
            Cat::Definition => "Definition",
            Cat::Applications => "Applications",
            Cat::Settings => "System Settings",
            Cat::Documents => "Documents",
            Cat::Folders => "Folders",
            Cat::Images => "Images",
            Cat::Music => "Music",
            Cat::Movies => "Movies",
            Cat::Files => "Other Files",
        }
    }

    /// Big card rows without a section header.
    pub fn is_card(self) -> bool {
        matches!(self, Cat::Calculator | Cat::Conversion | Cat::TopHit)
    }

    #[allow(dead_code)]
    pub fn row_height(self) -> f32 {
        match self {
            _ if self.is_card() => CARD_H,
            Cat::Web => WEB_H,
            _ => ROW_H,
        }
    }

    /// How many sections files can land in (see `of_file`).
    const FILE_SECTIONS: usize = 6;

    fn of_file(p: &Path, is_dir: bool) -> Cat {
        match index::kind(p, is_dir) {
            Kind::Folder => Cat::Folders,
            Kind::Document => Cat::Documents,
            Kind::Image => Cat::Images,
            Kind::Music => Cat::Music,
            Kind::Movie => Cat::Movies,
            Kind::Other => Cat::Files,
        }
    }
}

#[derive(Clone)]
pub enum Item {
    App(usize),
    File(PathBuf, bool),
    Calc { expr: String, value: String },
    Convert { result: String, detail: String },
    PendingRates,
    Define(String),
    Web(String),
    WebQuery(String),
    GlintSettings,
}

#[derive(Clone)]
pub struct Row {
    pub cat: Cat,
    pub item: Item,
}

impl Row {
    pub fn row_height(&self) -> f32 {
        match (&self.cat, &self.item) {
            (Cat::TopHit, Item::File(..)) => 76.0,
            _ if self.cat.is_card() => CARD_H,
            (Cat::Web, _) => WEB_H,
            (_, Item::File(..)) => 58.0,
            _ => ROW_H,
        }
    }
}

pub struct Glint {
    pub query: String,
    pub rows: Vec<Row>,
    pub sel: usize,
    pub scroll_to_sel: bool,
    pub apps: Vec<AppEntry>,
    pub icons: crate::icons::IconTheme,
    mode: Mode,
    pub settings: Settings,
    /// Settings when the page was opened, to tell whether a re-index is needed.
    pub(crate) index_snapshot: Settings,
    cancel: Arc<AtomicBool>,
    pub(crate) new_root: String,
    /// Time of the last change not yet written to disk.
    pub(crate) settings_dirty: Option<f64>,
    /// Focused row of the settings page (index into the visible rows).
    pub(crate) settings_row: usize,
    /// Scroll the focused row into view on the next draw.
    pub(crate) scroll_row: bool,
    /// Text of the custom colour field.
    pub(crate) hex: String,
    /// Keys to inject in screenshot mode, one per frame from frame 20.
    key_script: Vec<Key>,
    pub files: Arc<Mutex<Vec<FileEntry>>>,
    pub files_done: Arc<AtomicBool>,
    seen_files: usize,
    pub history: HashMap<String, u32>,
    pub rates: Arc<Mutex<Option<Fetch<HashMap<String, f64>>>>>,
    rates_ready: bool,
    pub defs: Arc<Mutex<HashMap<String, Fetch<Definition>>>>,
    pub info: Option<FileInfo>,
    pub toast: Option<(String, f64)>,
    /// Sections the person expanded with "Show More", and those that have more.
    pub expanded: HashSet<Cat>,
    pub more: HashSet<Cat>,
    pub show_preview: bool,
    last_query: String,
    pub subtitles: HashMap<PathBuf, String>,
    pub utc_offset: i64,
    height: f32,
    focus_tries: u32,
    last_auto_refresh: f64,
    centered: bool,
    was_focused: bool,
    pub screenshot: Option<PathBuf>,
    pub theme: draw::Theme,
    frame: u32,
}

#[allow(dead_code)]
pub type Lookout = Glint;

impl Glint {
    fn new(cc: &eframe::CreationContext, screenshot: Option<PathBuf>, query: String) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.global_style_mut(|s| {
            s.interaction.selectable_labels = false;
        });
        let (settings, err) = config::load();
        let theme = draw::Theme::new(settings.accent, settings.opacity);
        draw::apply_accent(&cc.egui_ctx, theme.accent);
        let mut me = Self {
            query,
            rows: Vec::new(),
            sel: 0,
            scroll_to_sel: false,
            apps: index::load_apps(),
            icons: crate::icons::IconTheme::load(),
            mode: Mode::Search,
            index_snapshot: settings.clone(),
            settings,
            cancel: Arc::new(AtomicBool::new(false)),
            new_root: String::new(),
            settings_dirty: None,
            settings_row: 0,
            scroll_row: false,
            hex: String::new(),
            key_script: Vec::new(),
            files: Arc::new(Mutex::new(Vec::new())),
            files_done: Arc::new(AtomicBool::new(false)),
            seen_files: 0,
            history: index::load_history(),
            rates: Arc::new(Mutex::new(None)),
            rates_ready: false,
            defs: Arc::new(Mutex::new(HashMap::new())),
            info: None,
            toast: None,
            expanded: HashSet::new(),
            more: HashSet::new(),
            show_preview: false,
            last_query: String::new(),
            subtitles: HashMap::new(),
            utc_offset: index::utc_offset(),
            height: BAR_H,
            focus_tries: 0,
            last_auto_refresh: 0.0,
            centered: false,
            was_focused: false,
            screenshot,
            theme,
            frame: 0,
        };
        #[cfg(target_os = "linux")]
        if me.screenshot.is_none()
            && let Some(w) = crate::x11focus::window_id(cc)
        {
            crate::x11focus::activate_in_background(w);
        }
        if let Some(e) = err {
            me.toast(&cc.egui_ctx, &e);
        }
        me.spawn_index(&cc.egui_ctx);
        me.refresh(&cc.egui_ctx);
        me
    }

    /// (Re)start the background file index with the current settings.
    fn spawn_index(&mut self, ctx: &egui::Context) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.files = Arc::new(Mutex::new(Vec::new()));
        self.files_done = Arc::new(AtomicBool::new(false));
        self.seen_files = 0;
        self.subtitles.clear();
        let (f, d, c, cancel) = (self.files.clone(), self.files_done.clone(), ctx.clone(), self.cancel.clone());
        let opts = index::IndexOpts::from_settings(&self.settings);
        std::thread::spawn(move || index::index_files(opts, f, d, cancel, move || c.request_repaint()));
    }

    pub(crate) fn restyle(&mut self, ctx: &egui::Context) {
        self.theme = draw::Theme::new(self.settings.accent, self.settings.opacity);
        draw::apply_accent(ctx, self.theme.accent);
    }

    fn enter_settings(&mut self) {
        self.index_snapshot = self.settings.clone();
        self.show_preview = false;
        self.mode = Mode::Settings;
        self.settings_row = 0;
        self.scroll_row = true;
    }

    fn save_settings(&mut self, ctx: &egui::Context) {
        self.settings_dirty = None;
        if let Err(e) = self.settings.save() {
            self.toast(ctx, &format!("Could not save settings: {e}"));
        }
    }

    fn leave_settings(&mut self, ctx: &egui::Context) {
        if self.settings_dirty.is_some() {
            self.save_settings(ctx);
        }
        if self.settings.index_differs(&self.index_snapshot) {
            self.spawn_index(ctx);
        }
        self.mode = Mode::Search;
        // A stale unfocused state must not close the popover right away.
        self.was_focused = false;
        self.focus_tries = 0;
        self.refresh(ctx);
    }

    fn boost(&self, key: &str) -> i32 {
        self.history.get(key).map_or(0, |n| (*n as i32 * 4).min(30))
    }

    /// Rebuild the result list for the current query.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        let raw = self.query.trim().to_string();
        let q = raw.to_lowercase();
        let mut rows = Vec::new();
        self.info = None;
        self.sel = 0;
        self.scroll_to_sel = true;
        if q != self.last_query {
            self.expanded.clear();
            self.last_query = q.clone();
        }
        if q.is_empty() {
            self.rows = rows;
            return;
        }

        if let Some(v) = calc::calculate(&raw) {
            rows.push(Row { cat: Cat::Calculator, item: Item::Calc { expr: raw.clone(), value: calc::format_number(v) } });
        }
        let rates = match &*self.rates.lock().unwrap() {
            Some(Fetch::Ready(r)) => r.clone(),
            _ => HashMap::new(),
        };
        self.rates_ready = !rates.is_empty();
        match calc::convert(&raw, &rates) {
            Some(calc::Conversion::Done { result, detail }) => {
                rows.push(Row { cat: Cat::Conversion, item: Item::Convert { result, detail } })
            }
            Some(calc::Conversion::NeedsRates) if self.settings.online => {
                self.start_rates(ctx);
                rows.push(Row { cat: Cat::Conversion, item: Item::PendingRates });
            }
            Some(calc::Conversion::NeedsRates) | None => {}
        }

        // (score, category, item, name length for tie-breaks)
        let mut hits: Vec<(i32, Cat, Item, usize)> = Vec::new();
        for (i, a) in self.apps.iter().enumerate() {
            let s = index::score(&a.name_lc, &q)
                .max(index::score(&a.generic_lc, &q) - 15)
                .max(if q.len() >= 3 && a.keywords_lc.contains(&q) { 30 } else { 0 });
            if s > 0 {
                let cat = if a.settings { Cat::Settings } else { Cat::Applications };
                // Apps lead, as Spotlight's top hit usually is an app.
                hits.push((s + 20 + self.boost(&format!("app:{}", a.id)), cat, Item::App(i), a.name.len()));
            }
        }
        if q.len() >= 3 && (q.starts_with("glint") || SETTINGS_WORDS.iter().any(|w| w.starts_with(q.as_str()))) {
            hits.push((90, Cat::Applications, Item::GlintSettings, 14));
        }
        {
            // Hundreds of thousands of names are scored per keystroke, so this
            // loop allocates nothing: hits are (score, name length, index).
            let files = self.files.lock().unwrap();
            self.seen_files = files.len();
            let boosts: HashMap<&str, i32> = self
                .history
                .iter()
                .filter_map(|(k, n)| Some((k.strip_prefix("file:")?, (*n as i32 * 4).min(30))))
                .collect();
            let mut file_hits: Vec<(i32, u32, u32)> = Vec::new();
            for (i, f) in files.iter().enumerate() {
                let s = index::score(&f.name_lc, &q);
                if s > 0 {
                    let boost = match boosts.is_empty() {
                        true => 0,
                        false => f.path.to_str().and_then(|p| boosts.get(p)).copied().unwrap_or(0),
                    };
                    let s = s - i32::from(f.depth.min(12)) + boost - if f.hidden { 10 } else { 0 };
                    file_hits.push((s, f.name_lc.len() as u32, i as u32));
                }
            }
            file_hits.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            // Keep only what the sections can show (+2: one may become the top
            // hit, one more reveals "Show More"); stop once every section is full.
            let mut taken: HashMap<Cat, usize> = HashMap::new();
            let mut full = 0;
            for (s, len, i) in file_hits {
                let f = &files[i as usize];
                let cat = Cat::of_file(&f.path, f.is_dir);
                let cap = if self.expanded.contains(&cat) { 32 } else { 5 };
                let n = taken.entry(cat).or_default();
                if *n < cap {
                    *n += 1;
                    if *n == cap {
                        full += 1;
                    }
                    hits.push((s, cat, Item::File(f.path.clone(), f.is_dir), len as usize));
                }
                if full == Cat::FILE_SECTIONS {
                    break;
                }
            }
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.3.cmp(&b.3)));

        rows.push(Row { cat: Cat::Web, item: Item::Web(raw.clone()) });
        if !raw.is_empty() {
            if q == "notes" {
                rows.push(Row { cat: Cat::Web, item: Item::WebQuery("notes app".to_string()) });
                rows.push(Row { cat: Cat::Web, item: Item::WebQuery("notes from underground".to_string()) });
                rows.push(Row { cat: Cat::Web, item: Item::WebQuery("noted aromas".to_string()) });
            } else {
                rows.push(Row { cat: Cat::Web, item: Item::WebQuery(format!("{raw} app")) });
            }
        }
        if self.settings.online && (2..=40).contains(&raw.len()) && raw.chars().all(|c| c.is_ascii_alphabetic()) {
            rows.push(Row { cat: Cat::Definition, item: Item::Define(raw.to_lowercase()) });
        }

        self.more.clear();
        if let Some(top) = hits.first().cloned() {
            rows.push(Row { cat: Cat::TopHit, item: top.2 });
            let mut per_cat: HashMap<Cat, usize> = HashMap::new();
            for (_, cat, item, _) in hits.into_iter().skip(1) {
                let n = per_cat.entry(cat).or_default();
                let cap = if self.expanded.contains(&cat) { 30 } else { 3 };
                if *n < cap {
                    *n += 1;
                    rows.push(Row { cat, item });
                } else {
                    self.more.insert(cat);
                }
            }
        }
        rows.sort_by_key(|r| r.cat); // Stable: keeps score order inside a section.
        self.rows = rows;
    }

    /// Height the result list needs, so the window fits its content.
    pub fn content_height(&self) -> f32 {
        let mut h = 16.0;
        let mut last = None;
        for r in &self.rows {
            if !r.cat.is_card() && last != Some(r.cat) {
                h += HEAD_H;
            }
            last = Some(r.cat);
            h += r.row_height();
        }
        h
    }

    pub fn expand(&mut self, ctx: &egui::Context, cat: Cat) {
        self.expanded.insert(cat);
        let sel = self.sel;
        self.refresh(ctx);
        self.sel = sel;
        self.scroll_to_sel = false;
    }

    fn start_rates(&self, ctx: &egui::Context) {
        let mut st = self.rates.lock().unwrap();
        if matches!(*st, Some(Fetch::Loading | Fetch::Ready(_))) {
            return;
        }
        *st = Some(Fetch::Loading);
        let (slot, ctx) = (self.rates.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = open::fetch_rates();
            *slot.lock().unwrap() = Some(match r {
                Ok(m) => Fetch::Ready(m),
                Err(e) => Fetch::Failed(e),
            });
            ctx.request_repaint();
        });
    }

    /// Fetched only when the person selects the Definition row.
    pub fn start_definition(&self, ctx: &egui::Context, word: &str) {
        let mut defs = self.defs.lock().unwrap();
        if defs.contains_key(word) {
            return;
        }
        defs.insert(word.to_string(), Fetch::Loading);
        let (slot, ctx, word) = (self.defs.clone(), ctx.clone(), word.to_string());
        std::thread::spawn(move || {
            let r = match open::fetch_definition(&word) {
                Ok(d) => Fetch::Ready(d),
                Err(e) => Fetch::Failed(e),
            };
            slot.lock().unwrap().insert(word, r);
            ctx.request_repaint();
        });
    }

    fn record(&mut self, key: String) {
        *self.history.entry(key).or_default() += 1;
        index::save_history(&self.history);
    }

    pub(crate) fn toast(&mut self, ctx: &egui::Context, msg: &str) {
        self.toast = Some((msg.to_string(), ctx.input(|i| i.time) + 2.0));
    }

    fn copy_selected(&mut self, ctx: &egui::Context) {
        let text = match self.rows.get(self.sel).map(|r| &r.item) {
            Some(Item::App(i)) => self.apps[*i].name.clone(),
            Some(Item::File(p, _)) => p.to_string_lossy().into_owned(),
            Some(Item::Calc { value, .. }) => value.replace(',', ""),
            Some(Item::Convert { result, .. }) => result.clone(),
            Some(Item::Define(w)) => w.clone(),
            Some(Item::Web(q)) => open::web_url(q),
            Some(Item::WebQuery(q)) => q.clone(),
            Some(Item::GlintSettings) => "Glint Settings".to_string(),
            _ => return,
        };
        ctx.copy_text(text);
        self.toast(ctx, "Copied to clipboard");
    }

    /// Open the selected row. `reveal` shows files in their folder instead.
    pub fn activate(&mut self, ctx: &egui::Context, reveal: bool) {
        let Some(row) = self.rows.get(self.sel).cloned() else { return };
        let result = match &row.item {
            Item::App(i) if reveal => open::reveal(&self.apps[*i].path),
            Item::App(i) => {
                let r = open::launch_app(&self.apps[*i]);
                self.record(format!("app:{}", self.apps[*i].id));
                r
            }
            Item::File(p, _) if reveal => open::reveal(p),
            Item::File(p, _) => {
                self.record(format!("file:{}", p.display()));
                open::xdg_open(&p.to_string_lossy())
            }
            Item::Calc { .. } | Item::Convert { .. } => {
                // Stay open: on X11 the clipboard lives with the window that owns it.
                self.copy_selected(ctx);
                return;
            }
            Item::PendingRates => return,
            Item::GlintSettings => {
                self.enter_settings();
                return;
            }
            Item::Define(w) => open::xdg_open(&format!("https://en.wiktionary.org/wiki/{w}")),
            Item::Web(q) => open::xdg_open(&open::web_url(q)),
            Item::WebQuery(q) => open::xdg_open(&open::web_url(q)),
        };
        match result {
            Ok(()) if self.screenshot.is_none() => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Ok(()) => {}
            Err(e) => self.toast(ctx, &format!("Could not open: {e}")),
        }
    }

    fn jump_category(&mut self, down: bool) {
        let Some(cur) = self.rows.get(self.sel).map(|r| r.cat) else { return };
        let idx = if down {
            self.rows.iter().position(|r| r.cat > cur)
        } else {
            let start = self.rows.iter().position(|r| r.cat == cur).unwrap_or(0);
            let prev = self.rows[..start].last().map(|r| r.cat);
            prev.and_then(|p| self.rows.iter().position(|r| r.cat == p))
        };
        if let Some(i) = idx {
            self.sel = i;
            self.scroll_to_sel = true;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Esc closes an open colour picker first; only then does it leave settings.
        let popup = self.mode == Mode::Settings && egui::Popup::is_any_open(ctx);
        let (toggle, esc) = ctx
            .input_mut(|i| (i.consume_key(Modifiers::COMMAND, Key::Comma), !popup && i.consume_key(Modifiers::NONE, Key::Escape)));
        if self.mode == Mode::Settings {
            if toggle || esc {
                self.leave_settings(ctx);
            } else if !popup {
                self.settings_keys(ctx);
            }
            return;
        }
        if toggle {
            self.enter_settings();
            return;
        }
        let n = self.rows.len();
        let mut k = [false; 10];
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Y)) {
            // Quick Look style preview beside the list.
            self.show_preview = !self.show_preview;
        }
        ctx.input_mut(|i| {
            k[0] = i.consume_key(Modifiers::COMMAND, Key::ArrowDown);
            k[1] = i.consume_key(Modifiers::COMMAND, Key::ArrowUp);
            k[2] = i.consume_key(Modifiers::COMMAND, Key::Enter) || i.consume_key(Modifiers::COMMAND, Key::R);
            k[3] = i.consume_key(Modifiers::COMMAND, Key::B);
            k[4] = i.consume_key(Modifiers::COMMAND, Key::L);
            k[5] = i.consume_key(Modifiers::NONE, Key::ArrowDown) || i.consume_key(Modifiers::NONE, Key::Tab);
            k[6] = i.consume_key(Modifiers::NONE, Key::ArrowUp) || i.consume_key(Modifiers::SHIFT, Key::Tab);
            k[7] = i.consume_key(Modifiers::NONE, Key::Enter);
            k[8] = esc;
            k[9] = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
        });
        let [cdown, cup, reveal, web, define, down, up, enter, esc, copy] = k;
        if n > 0 && down {
            self.sel = (self.sel + 1).min(n - 1);
            self.scroll_to_sel = true;
        }
        if n > 0 && up {
            self.sel = self.sel.saturating_sub(1);
            self.scroll_to_sel = true;
        }
        if cdown {
            self.jump_category(true);
        }
        if cup {
            self.jump_category(false);
        }
        if web && let Some(i) = self.rows.iter().position(|r| r.cat == Cat::Web) {
            self.sel = i;
            self.activate(ctx, false);
        }
        if define && let Some(i) = self.rows.iter().position(|r| r.cat == Cat::Definition) {
            self.sel = i;
            self.scroll_to_sel = true;
        }
        if enter {
            self.activate(ctx, false);
        }
        if reveal {
            self.activate(ctx, true);
        }
        if copy && n > 0 {
            // A text selection inside the search field still wins: the text
            // field handles the same copy event after this.
            self.copy_selected(ctx);
        }
        if esc {
            if !self.query.is_empty() {
                self.query.clear();
                self.refresh(ctx);
            } else if self.screenshot.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Window bookkeeping: size, position, focus, background updates, screenshot.
    fn window_logic(&mut self, ctx: &egui::Context) {
        // The window keeps one size (some window managers ignore resizes);
        // only the panel drawn inside it grows with the results.
        self.height = match () {
            _ if self.mode == Mode::Settings => MAX_H,
            _ if self.rows.is_empty() => BAR_H,
            _ if self.show_preview => MAX_H,
            _ => (BAR_H + self.content_height()).min(MAX_H).round(),
        };
        let (monitor, focused) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().focused));
        // Window managers may refuse focus to a window that opens on its own;
        // ask for it until we have it, so typing goes straight into the bar.
        // Gives up after ~4 seconds, so an unfocused window doesn't redraw forever.
        if self.screenshot.is_none() && !self.was_focused && focused != Some(true) && self.focus_tries < 120 {
            self.focus_tries += 1;
            if self.focus_tries % 6 == 1 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if !self.centered && let Some(m) = monitor {
            self.centered = true;
            let pos = Pos2::new(((m.x - WIDTH) / 2.0).max(0.0), (m.y * 0.22).round());
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        // Dismiss like a popover once focus moves elsewhere.
        if self.screenshot.is_none() {
            match focused {
                Some(true) => self.was_focused = true,
                Some(false) if self.was_focused && self.mode == Mode::Search => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                _ => {}
            }
        }
        // Keep results fresh while the file index fills up or rates arrive.
        let rates_ready = matches!(&*self.rates.lock().unwrap(), Some(Fetch::Ready(_)));
        // While indexing, new files arrive in many small batches: re-search at
        // most twice a second instead of after every batch.
        let now = ctx.input(|i| i.time);
        if let Some(t) = self.settings_dirty {
            if now - t > SAVE_DELAY {
                self.save_settings(ctx);
            } else {
                ctx.request_repaint_after(Duration::from_millis(((SAVE_DELAY - (now - t)) * 1000.0) as u64 + 20));
            }
        }
        // The list is hidden in settings mode; `leave_settings` refreshes it.
        let in_search = self.mode == Mode::Search && !self.query.is_empty();
        let grown = in_search && self.files.lock().unwrap().len() != self.seen_files && now - self.last_auto_refresh > 0.5;
        if in_search && (grown || rates_ready != self.rates_ready) {
            self.last_auto_refresh = now;
            let sel = self.sel;
            self.refresh(ctx);
            self.sel = sel.min(self.rows.len().saturating_sub(1));
            self.scroll_to_sel = false;
        }
        if !self.files_done.load(Ordering::Relaxed) {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        if let Some((_, until)) = self.toast
            && ctx.input(|i| i.time) < until
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if let Some(path) = self.screenshot.clone() {
            self.frame += 1;
            ctx.request_repaint();
            if self.frame == 120 {
                // The captured surface can keep its old, larger size after the
                // window shrinks, so keep only the rows the window really has.
                let rows = (self.height * ctx.pixels_per_point()).round() as usize;
                ctx.request_screenshot(move |img| {
                    let code = match draw::save_png(&path, &img, rows) {
                        Ok(()) => 0,
                        Err(e) => {
                            eprintln!("screenshot failed: {e}");
                            1
                        }
                    };
                    std::process::exit(code);
                });
            }
        }
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), BAR_H), Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(16.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
            draw::magnifier(ui.painter(), r, MUTED);
            ui.add_space(10.0);
            let edit = egui::TextEdit::singleline(&mut self.query)
                .font(FontId::proportional(22.0))
                .frame(egui::Frame::NONE)
                .hint_text(RichText::new("Search").size(22.0).color(MUTED))
                .text_color(FG)
                .desired_width(ui.available_width() - 20.0);
            let resp = ui.add(edit);
            // Only while the window itself has focus: re-requesting it every
            // frame on an unfocused window rebuilds the X11 input-method
            // context each frame, and then no keyboard (or focus) event ever
            // reaches the app.
            if ui.input(|i| i.focused) && !resp.has_focus() {
                resp.request_focus();
            }
            if resp.changed() {
                self.refresh(&ctx);
            }
        });
    }

    /// Short status (copied, indexing) in a pill at the bottom right.
    fn status(&self, ui: &egui::Ui, full: Rect) {
        let text = match &self.toast {
            Some((msg, until)) if ui.input(|i| i.time) < *until => msg.clone(),
            _ if !self.files_done.load(Ordering::Relaxed) => {
                format!("Indexing files… {}", self.files.lock().unwrap().len())
            }
            _ => return,
        };
        let p = ui.painter();
        let galley = p.layout_no_wrap(text, FontId::proportional(12.0), FG);
        let size = galley.size() + Vec2::new(20.0, 10.0);
        let rect = Rect::from_min_size(full.max - size - Vec2::splat(12.0), size);
        p.rect_filled(rect, 10, egui::Color32::from_rgba_premultiplied(16, 16, 18, 230));
        p.galley(rect.min + Vec2::new(10.0, 5.0), galley, FG);
    }
}

impl eframe::App for Glint {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    /// Closing within the save debounce must not lose settings changes.
    fn on_exit(&mut self) {
        if self.settings_dirty.is_some()
            && let Err(e) = self.settings.save()
        {
            eprintln!("Could not save settings: {e}");
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Scripted keys (screenshot checks only), one per frame from frame 20.
        if self.screenshot.is_some()
            && let Some(&key) = self.frame.checked_sub(20).and_then(|i| self.key_script.get(i as usize))
        {
            let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
            ctx.input_mut(|i| i.events.push(event));
        }
        self.handle_keys(&ctx);
        self.window_logic(&ctx);

        let window = ui.max_rect();
        let full = Rect::from_min_size(window.min, Vec2::new(window.width(), self.height.min(window.height())));
        draw::panel(ui.painter(), full, self.theme.bg);
        if self.mode == Mode::Settings {
            ui.scope_builder(UiBuilder::new().max_rect(full), |ui| self.settings_page(ui));
            self.status(ui, full);
            return;
        }
        // The see-through area below the panel acts as "outside": a click there closes.
        let below = Rect::from_min_max(Pos2::new(window.left(), full.bottom()), window.max);
        if below.height() > 1.0
            && ui.interact(below, ui.scope_id().with("outside"), Sense::click()).clicked()
            && self.screenshot.is_none()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), BAR_H));
        ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| self.search_bar(ui));
        if self.rows.is_empty() || full.height() < BAR_H + 40.0 {
            return;
        }
        let top = full.top() + BAR_H;
        let line = Stroke::new(1.0, HAIRLINE);
        ui.painter().hline(full.x_range(), top, line);
        let list_right = if self.show_preview { full.left() + LIST_W } else { full.right() };
        let list_rect = Rect::from_min_max(Pos2::new(full.left(), top + 1.0), Pos2::new(list_right, full.bottom() - 8.0));
        ui.scope_builder(UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect);
            self.list(ui);
        });
        if self.show_preview {
            ui.painter().vline(list_rect.right(), list_rect.y_range(), line);
            let prev_rect = Rect::from_min_max(
                Pos2::new(list_rect.right() + 20.0, top + 4.0),
                Pos2::new(full.right() - 20.0, full.bottom() - 14.0),
            );
            ui.scope_builder(UiBuilder::new().max_rect(prev_rect), |ui| {
                ui.set_clip_rect(prev_rect.expand(4.0));
                self.preview(ui);
            });
        }
        self.status(ui, full);
    }
}

pub fn run() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let mut screenshot = None;
    let mut query = String::new();
    let mut open_settings = false;
    let mut keys: Vec<Key> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--settings" => open_settings = true,
            "--keys" => keys = args.next().unwrap_or_default().split(',').filter_map(|k| parse_key(k.trim())).collect(),
            "--query" => query = args.next().unwrap_or_default(),
            "-h" | "--help" => {
                println!("glint [--query TEXT] [--settings] [--screenshot FILE.png [--keys Down,Right,...]]");
                return Ok(());
            }
            _ => {}
        }
    }
    if screenshot.is_some() && query.is_empty() && !open_settings {
        query = "Notes".to_string();
    }
    let shot = screenshot.is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Glint")
        .with_app_id("glint")
        .with_inner_size([WIDTH, MAX_H])
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(true);
    if shot {
        // Never steal focus from the person while a check takes the picture.
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    } else {
        viewport = viewport.with_always_on_top();
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // A small search panel doesn't need the discrete GPU. Waking it (the
    // default) made start-up twice as slow and every frame slower on laptops
    // with two GPUs. WGPU_POWER_PREF still overrides this.
    if std::env::var_os("WGPU_POWER_PREF").is_none()
        && let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup
    {
        setup.power_preference = eframe::wgpu::PowerPreference::LowPower;
    }
    #[cfg(target_os = "macos")]
    if shot {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        options.event_loop_builder = Some(Box::new(|b| {
            b.with_activate_ignoring_other_apps(false);
        }));
    }
    eframe::run_native("Glint", options, Box::new(move |cc| {
        let mut app = Glint::new(cc, screenshot, query);
        app.key_script = keys;
        if open_settings {
            app.enter_settings();
        }
        Ok(Box::new(app))
    }))
}

/// Key names accepted by `--keys`.
fn parse_key(name: &str) -> Option<Key> {
    Some(match name.to_ascii_lowercase().as_str() {
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "enter" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        _ => return None,
    })
}

/// Checks a folder typed into the settings page; returns its canonical path.
pub(crate) fn validate_root(input: &str, existing: &[PathBuf]) -> Result<PathBuf, String> {
    let p = Path::new(input.trim());
    if !p.is_absolute() {
        return Err("Enter an absolute path, such as /mnt/data".into());
    }
    if !p.is_dir() {
        return Err("That folder does not exist".into());
    }
    let c = p.canonicalize().map_err(|e| format!("Cannot read that folder: {e}"))?;
    if index::is_denied_system(&c) {
        return Err("System folders such as /proc, /sys and /dev cannot be indexed".into());
    }
    if existing.contains(&c) {
        return Err("That folder is already in the list".into());
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_root_rejects_bad_input() {
        assert!(validate_root("relative/dir", &[]).is_err());
        assert!(validate_root("/definitely/not/here", &[]).is_err());
        assert!(validate_root("/proc", &[]).is_err());
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        let ok = validate_root(tmp.to_str().unwrap(), &[]).unwrap();
        assert!(validate_root(tmp.to_str().unwrap(), &[ok]).is_err());
    }
}
