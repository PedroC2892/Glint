//! The search window: query handling, results, keyboard, window behaviour.

use crate::calc;
use crate::config::{self, Settings};
use crate::draw::{self, FG, HAIRLINE, MUTED};
use crate::index::{self, AppEntry, FileEntry, FileInfo, Kind};
use crate::open::{self, Definition, Fetch};
use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align, FontId, Key, Layout, Modifiers, Pos2, Rect, RichText, Sense, Stroke, UiBuilder,
    Vec2,
};
#[cfg(target_os = "linux")]
use raw_window_handle::HasWindowHandle;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const WIDTH: f32 = 680.0;
pub const BAR_H: f32 = 54.0;
const MAX_H: f32 = 520.0;
const LIST_W: f32 = 350.0;
/// Words that make the "Glint Settings" result appear (typed prefixes count).
const SETTINGS_WORDS: &[&str] = &[
    "glint settings",
    "settings",
    "preferences",
    "prefs",
    "config",
    "configuration",
    "options",
];
/// How long a settings change waits before it is written to disk.
const SAVE_DELAY: f64 = 0.5;
/// Minimum seconds between background re-indexes on show.
const REINDEX_COOLDOWN: f64 = 30.0;

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

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

pub struct Args {
    pub screenshot: Option<PathBuf>,
    pub query: String,
    pub open_settings: bool,
    pub keys: Vec<Key>,
    pub daemon: bool,
    pub no_daemon: bool,
    pub quit: bool,
}

impl Args {
    pub fn parse() -> Self {
        let mut args_iter = std::env::args().skip(1);
        let mut a = Args {
            screenshot: None,
            query: String::new(),
            open_settings: false,
            keys: Vec::new(),
            daemon: false,
            no_daemon: false,
            quit: false,
        };
        while let Some(arg) = args_iter.next() {
            match arg.as_str() {
                "--screenshot" => {
                    a.screenshot = args_iter.next().map(PathBuf::from);
                    a.no_daemon = true; // --screenshot implies --no-daemon
                }
                "--settings" => a.open_settings = true,
                "--keys" => {
                    a.keys = args_iter
                        .next()
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|k| parse_key(k.trim()))
                        .collect();
                }
                "--query" => a.query = args_iter.next().unwrap_or_default(),
                "--daemon" => a.daemon = true,
                "--no-daemon" => a.no_daemon = true,
                "--quit" => a.quit = true,
                "-h" | "--help" => {
                    println!("glint [--query TEXT] [--settings] [--daemon] [--no-daemon] [--quit]");
                    println!("      [--screenshot FILE.png [--keys Down,Right,...]]");
                    std::process::exit(0);
                }
                _ => {}
            }
        }
        a
    }
}

// ---------------------------------------------------------------------------
// Shared daemon state that survives across hide/show cycles.
// ---------------------------------------------------------------------------

pub fn search_id() -> egui::Id {
    egui::Id::unique("search_bar_input")
}

/// Commands queued from the IPC listener thread to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UiCmd {
    Show(Option<Instant>),
    ShowSettings(Option<Instant>),
    ShowQuery(String, Option<Instant>),
    Quit,
}

pub type RatesSlot = Arc<Mutex<Option<Fetch<HashMap<String, f64>>>>>;
pub type DefsSlot = Arc<Mutex<HashMap<String, Fetch<Definition>>>>;

pub struct Warm {
    pub apps: Vec<AppEntry>,
    pub icons: crate::icons::IconTheme,
    pub files: Arc<Mutex<Vec<FileEntry>>>,
    pub files_done: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
    pub history: HashMap<String, u32>,
    pub rates: RatesSlot,
    pub defs: DefsSlot,
    pub settings: Settings,
    pub last_index_done: f64,
    pub config_mtime: Option<std::time::SystemTime>,
    pub apps_dir_mtimes: HashMap<PathBuf, Option<std::time::SystemTime>>,
}

impl Warm {
    pub fn new() -> Self {
        let (settings, _) = config::load();
        let apps = index::load_apps();
        let icons = crate::icons::IconTheme::load();
        let history = index::load_history();
        let apps_dir_mtimes = Glint::snapshot_app_dir_mtimes();
        let config_mtime = Glint::config_file_mtime();
        let files = Arc::new(Mutex::new(Vec::new()));
        let files_done = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));

        let opts = index::IndexOpts::from_settings(&settings);
        let f = files.clone();
        let d = files_done.clone();
        let c = cancel.clone();
        std::thread::spawn(move || index::index_files(opts, f, d, c, || {}));

        Self {
            apps,
            icons,
            files,
            files_done,
            cancel,
            history,
            rates: Arc::new(Mutex::new(None)),
            defs: Arc::new(Mutex::new(HashMap::new())),
            settings,
            last_index_done: 0.0,
            config_mtime,
            apps_dir_mtimes,
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
    hook_frame: u32,
    /// Navigation keys taken from the raw input for the settings page.
    pub(crate) nav_keys: Vec<(Key, bool)>,
    pub files: Arc<Mutex<Vec<FileEntry>>>,
    pub files_done: Arc<AtomicBool>,
    seen_files: usize,
    pub history: HashMap<String, u32>,
    pub rates: RatesSlot,
    rates_ready: bool,
    pub defs: DefsSlot,
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

    // --- daemon mode fields ---
    /// Whether this is a daemon (hide instead of close).
    daemon_mode: bool,
    /// Whether the window is currently hidden (daemon mode only).
    hidden: bool,
    /// Commands from the IPC listener thread.
    ui_cmds: Arc<Mutex<Vec<UiCmd>>>,
    /// Time the last index completed (for the 30 s cooldown).
    last_index_done: f64,
    /// Stored X11 window id for direct show/hide from the listener thread.
    #[cfg(target_os = "linux")]
    x11_window_id: Option<u32>,
    /// mtime of the config file at last load.
    config_mtime: Option<std::time::SystemTime>,
    /// mtimes of applications dirs at last load.
    apps_dir_mtimes: HashMap<PathBuf, Option<std::time::SystemTime>>,
    warm_slot: Arc<Mutex<Option<Warm>>>,
    pub is_wayland: bool,
    show_t0: Option<Instant>,
}

#[allow(dead_code)]
pub type Lookout = Glint;

impl Glint {
    #[allow(clippy::too_many_arguments)]
    fn from_warm(
        cc: &eframe::CreationContext,
        warm: Warm,
        screenshot: Option<PathBuf>,
        query: String,
        daemon_mode: bool,
        ui_cmds: Arc<Mutex<Vec<UiCmd>>>,
        warm_slot: Arc<Mutex<Option<Warm>>>,
        #[cfg(target_os = "linux")] shared_window_id: Arc<AtomicU32>,
        start_hidden: bool,
    ) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.global_style_mut(|s| {
            s.interaction.selectable_labels = false;
        });
        let theme = draw::Theme::new(warm.settings.accent, warm.settings.opacity);
        draw::apply_accent(&cc.egui_ctx, theme.accent);

        #[cfg(target_os = "linux")]
        let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            && matches!(
                cc.window_handle().map(|h| h.as_raw()),
                Ok(raw_window_handle::RawWindowHandle::Wayland(_))
            );
        #[cfg(not(target_os = "linux"))]
        let is_wayland = false;

        let mut x11_window_id = None;
        #[cfg(target_os = "linux")]
        if screenshot.is_none()
            && !is_wayland
            && let Some(w) = crate::x11focus::window_id(cc)
        {
            x11_window_id = Some(w);
            shared_window_id.store(w, Ordering::Relaxed);
            if !start_hidden {
                crate::x11focus::activate_in_background(w);
            }
        }

        let mut me = Self {
            query,
            rows: Vec::new(),
            sel: 0,
            scroll_to_sel: false,
            apps: warm.apps,
            icons: warm.icons,
            mode: Mode::Search,
            index_snapshot: warm.settings.clone(),
            settings: warm.settings,
            cancel: warm.cancel,
            new_root: String::new(),
            settings_dirty: None,
            settings_row: 0,
            scroll_row: false,
            hex: String::new(),
            key_script: Vec::new(),
            hook_frame: 0,
            nav_keys: Vec::new(),
            files: warm.files,
            files_done: warm.files_done,
            seen_files: 0,
            history: warm.history,
            rates: warm.rates,
            rates_ready: false,
            defs: warm.defs,
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
            daemon_mode,
            hidden: start_hidden,
            ui_cmds,
            last_index_done: warm.last_index_done,
            #[cfg(target_os = "linux")]
            x11_window_id,
            config_mtime: warm.config_mtime,
            apps_dir_mtimes: warm.apps_dir_mtimes,
            warm_slot,
            is_wayland,
            show_t0: None,
        };

        if start_hidden {
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        if !start_hidden {
            me.refresh(&cc.egui_ctx);
        }
        me
    }

    fn new(cc: &eframe::CreationContext, screenshot: Option<PathBuf>, query: String) -> Self {
        let warm = Warm::new();
        let ui_cmds = Arc::new(Mutex::new(Vec::new()));
        let warm_slot = Arc::new(Mutex::new(None));
        #[cfg(target_os = "linux")]
        let shared_window_id = Arc::new(AtomicU32::new(0));
        let mut app = Self::from_warm(
            cc,
            warm,
            screenshot,
            query,
            false,
            ui_cmds,
            warm_slot,
            #[cfg(target_os = "linux")]
            shared_window_id,
            false,
        );
        if let Some(err) = config::load().1 {
            app.toast(&cc.egui_ctx, &err);
        }
        app
    }

    /// (Re)start the background file index with the current settings.
    fn spawn_index(&mut self, ctx: &egui::Context) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.files = Arc::new(Mutex::new(Vec::new()));
        self.files_done = Arc::new(AtomicBool::new(false));
        self.seen_files = 0;
        self.subtitles.clear();
        let (f, d, c, cancel) = (
            self.files.clone(),
            self.files_done.clone(),
            ctx.clone(),
            self.cancel.clone(),
        );
        let opts = index::IndexOpts::from_settings(&self.settings);
        std::thread::spawn(move || {
            index::index_files(opts, f, d, cancel, move || c.request_repaint())
        });
    }

    /// Start a background re-index that keeps old results visible until done.
    /// Skips if the last index completed less than REINDEX_COOLDOWN seconds ago.
    fn spawn_refresh_index(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.last_index_done < REINDEX_COOLDOWN {
            return;
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));

        // New buffers that will be swapped in atomically when done.
        let new_files = Arc::new(Mutex::new(Vec::new()));
        let new_done = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let opts = index::IndexOpts::from_settings(&self.settings);
        let ctx2 = ctx.clone();

        // The old `self.files` and `self.files_done` stay in place so the UI
        // keeps showing previous results. We'll swap in `on_refresh_complete`.
        let old_files = self.files.clone();
        let old_done = self.files_done.clone();
        let nf = new_files.clone();
        let nd = new_done.clone();

        std::thread::spawn(move || {
            index::index_files(opts, nf.clone(), nd.clone(), cancel, || {});
            if nd.load(Ordering::Relaxed) {
                // Swap atomically: replace old data with new.
                let mut old = old_files.lock().unwrap();
                let mut new = nf.lock().unwrap();
                std::mem::swap(&mut *old, &mut *new);
                old_done.store(true, Ordering::Relaxed);
            }
            ctx2.request_repaint();
        });
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
            rows.push(Row {
                cat: Cat::Calculator,
                item: Item::Calc {
                    expr: raw.clone(),
                    value: calc::format_number(v),
                },
            });
        }
        let rates = match &*self.rates.lock().unwrap() {
            Some(Fetch::Ready(r)) => r.clone(),
            _ => HashMap::new(),
        };
        self.rates_ready = !rates.is_empty();
        match calc::convert(&raw, &rates) {
            Some(calc::Conversion::Done { result, detail }) => rows.push(Row {
                cat: Cat::Conversion,
                item: Item::Convert { result, detail },
            }),
            Some(calc::Conversion::NeedsRates) if self.settings.online => {
                self.start_rates(ctx);
                rows.push(Row {
                    cat: Cat::Conversion,
                    item: Item::PendingRates,
                });
            }
            Some(calc::Conversion::NeedsRates) | None => {}
        }

        // (score, category, item, name length for tie-breaks)
        let mut hits: Vec<(i32, Cat, Item, usize)> = Vec::new();
        for (i, a) in self.apps.iter().enumerate() {
            let s = index::score(&a.name_lc, &q)
                .max(index::score(&a.generic_lc, &q) - 15)
                .max(if q.len() >= 3 && a.keywords_lc.contains(&q) {
                    30
                } else {
                    0
                });
            if s > 0 {
                let cat = if a.settings {
                    Cat::Settings
                } else {
                    Cat::Applications
                };
                // Apps lead, as Spotlight's top hit usually is an app.
                hits.push((
                    s + 20 + self.boost(&format!("app:{}", a.id)),
                    cat,
                    Item::App(i),
                    a.name.len(),
                ));
            }
        }
        if q.len() >= 3
            && (q.starts_with("glint") || SETTINGS_WORDS.iter().any(|w| w.starts_with(q.as_str())))
        {
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
                        false => f
                            .path
                            .to_str()
                            .and_then(|p| boosts.get(p))
                            .copied()
                            .unwrap_or(0),
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

        rows.push(Row {
            cat: Cat::Web,
            item: Item::Web(raw.clone()),
        });
        if !raw.is_empty() {
            if q == "notes" {
                rows.push(Row {
                    cat: Cat::Web,
                    item: Item::WebQuery("notes app".to_string()),
                });
                rows.push(Row {
                    cat: Cat::Web,
                    item: Item::WebQuery("notes from underground".to_string()),
                });
                rows.push(Row {
                    cat: Cat::Web,
                    item: Item::WebQuery("noted aromas".to_string()),
                });
            } else {
                rows.push(Row {
                    cat: Cat::Web,
                    item: Item::WebQuery(format!("{raw} app")),
                });
            }
        }
        if self.settings.online
            && (2..=40).contains(&raw.len())
            && raw.chars().all(|c| c.is_ascii_alphabetic())
        {
            rows.push(Row {
                cat: Cat::Definition,
                item: Item::Define(raw.to_lowercase()),
            });
        }

        self.more.clear();
        if let Some(top) = hits.first().cloned() {
            rows.push(Row {
                cat: Cat::TopHit,
                item: top.2,
            });
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

    /// Hide the panel (daemon mode) or close it (one-shot mode).
    fn dismiss(&mut self, ctx: &egui::Context) {
        if self.daemon_mode {
            self.hide(ctx);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Open the selected row. `reveal` shows files in their folder instead.
    pub fn activate(&mut self, ctx: &egui::Context, reveal: bool) {
        let Some(row) = self.rows.get(self.sel).cloned() else {
            return;
        };
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
            Ok(()) if self.screenshot.is_none() => self.dismiss(ctx),
            Ok(()) => {}
            Err(e) => self.toast(ctx, &format!("Could not open: {e}")),
        }
    }

    fn jump_category(&mut self, down: bool) {
        let Some(cur) = self.rows.get(self.sel).map(|r| r.cat) else {
            return;
        };
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
        let (toggle, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Comma),
                !popup && i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
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
            k[2] = i.consume_key(Modifiers::COMMAND, Key::Enter)
                || i.consume_key(Modifiers::COMMAND, Key::R);
            k[3] = i.consume_key(Modifiers::COMMAND, Key::B);
            k[4] = i.consume_key(Modifiers::COMMAND, Key::L);
            k[5] = i.consume_key(Modifiers::NONE, Key::ArrowDown)
                || i.consume_key(Modifiers::NONE, Key::Tab);
            k[6] = i.consume_key(Modifiers::NONE, Key::ArrowUp)
                || i.consume_key(Modifiers::SHIFT, Key::Tab);
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
                self.dismiss(ctx);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Daemon mode: hide / show
    // -----------------------------------------------------------------------

    /// Hide the panel and flush any pending saves. In daemon mode only.
    fn hide(&mut self, ctx: &egui::Context) {
        if self.hidden {
            return;
        }
        self.hidden = true;

        // Flush pending saves immediately.
        if self.settings_dirty.is_some() {
            self.save_settings(ctx);
        }
        index::save_history(&self.history);

        // Record when the index last completed (for cooldown).
        if self.files_done.load(Ordering::Relaxed) {
            self.last_index_done = ctx.input(|i| i.time);
        }

        #[cfg(target_os = "linux")]
        if self.is_wayland {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // On X11: hide the window via ViewportCommand and also unmap directly
        // with x11rb for reliability (eframe may not process viewport commands
        // while the window is not being painted).
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        #[cfg(target_os = "linux")]
        if let Some(wid) = self.x11_window_id {
            crate::x11focus::unmap_window(wid);
        }
    }

    /// Show the panel, preparing it for fresh use (Spotlight behaviour).
    fn show(&mut self, ctx: &egui::Context, t0_opt: Option<Instant>) {
        let trace = std::env::var_os("GLINT_TRACE").is_some();
        if trace && let Some(t0) = t0_opt {
            eprintln!("[trace {} ms] show() start", t0.elapsed().as_millis());
        }
        self.show_t0 = t0_opt;
        if !self.hidden {
            return;
        }
        self.hidden = false;

        // Keep previous query selected so typing replaces it (Item 5).
        let id = search_id();
        let len = self.query.chars().count();
        let mut state = TextEditState::load(ctx, id).unwrap_or_default();
        let c0 = CCursor::new(0);
        let c1 = CCursor::new(len);
        state.cursor.set_char_range(Some(CCursorRange::two(c0, c1)));
        state.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));

        self.sel = 0;
        self.scroll_to_sel = true;
        self.show_preview = false;
        self.mode = Mode::Search;

        // Reset focus tracking for the new appearance.
        self.was_focused = false;
        self.focus_tries = 0;
        self.centered = false;

        // Map through eframe if not already mapped
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        #[cfg(target_os = "linux")]
        if !self.is_wayland
            && let Some(wid) = self.x11_window_id
        {
            crate::x11focus::map_and_activate(wid);
            if trace && let Some(t0) = t0_opt {
                eprintln!(
                    "[trace {} ms] map sent from show()",
                    t0.elapsed().as_millis()
                );
            }
        }

        // Reload config if the file changed on disk.
        self.maybe_reload_config(ctx);
        if trace && let Some(t0) = t0_opt {
            eprintln!(
                "[trace {} ms] maybe_reload_config done",
                t0.elapsed().as_millis()
            );
        }

        // Reload apps if any applications dir mtime changed.
        self.maybe_reload_apps();
        if trace && let Some(t0) = t0_opt {
            eprintln!(
                "[trace {} ms] maybe_reload_apps done",
                t0.elapsed().as_millis()
            );
        }

        // Re-index in background (keeping old results visible).
        self.spawn_refresh_index(ctx);
        if trace && let Some(t0) = t0_opt {
            eprintln!(
                "[trace {} ms] spawn_refresh_index done",
                t0.elapsed().as_millis()
            );
        }

        // Refresh results with current data.
        self.refresh(ctx);
        if trace && let Some(t0) = t0_opt {
            eprintln!("[trace {} ms] refresh done", t0.elapsed().as_millis());
        }

        ctx.request_repaint();
    }

    /// Process commands from the IPC listener thread.
    fn process_ui_cmds(&mut self, ctx: &egui::Context) {
        let cmds: Vec<UiCmd> = {
            let mut lock = self.ui_cmds.lock().unwrap();
            std::mem::take(&mut *lock)
        };
        for cmd in cmds {
            match cmd {
                UiCmd::Show(t0) => self.show(ctx, t0),
                UiCmd::ShowSettings(t0) => {
                    self.show(ctx, t0);
                    self.enter_settings();
                }
                UiCmd::ShowQuery(text, t0) => {
                    self.query = text;
                    self.show(ctx, t0);
                    self.refresh(ctx);
                }
                UiCmd::Quit => {
                    // Flush saves and actually exit.
                    if self.settings_dirty.is_some() {
                        self.save_settings(ctx);
                    }
                    index::save_history(&self.history);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Snapshot mtimes of all `applications` directories.
    fn snapshot_app_dir_mtimes() -> HashMap<PathBuf, Option<std::time::SystemTime>> {
        let mut map = HashMap::new();
        for dir in index::data_dirs() {
            let apps_dir = dir.join("applications");
            let mtime = std::fs::metadata(&apps_dir).and_then(|m| m.modified()).ok();
            map.insert(apps_dir, mtime);
        }
        map
    }

    /// Reload .desktop apps if any applications directory mtime changed.
    fn maybe_reload_apps(&mut self) {
        let current = Self::snapshot_app_dir_mtimes();
        if current != self.apps_dir_mtimes {
            self.apps = index::load_apps();
            self.apps_dir_mtimes = current;
        }
    }

    /// Get the config file mtime.
    fn config_file_mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(config::config_path())
            .and_then(|m| m.modified())
            .ok()
    }

    /// Reload settings from disk if the config file changed.
    fn maybe_reload_config(&mut self, ctx: &egui::Context) {
        let current = Self::config_file_mtime();
        if current != self.config_mtime {
            let (settings, err) = config::load();
            self.settings = settings;
            self.restyle(ctx);
            self.config_mtime = current;
            if let Some(e) = err {
                self.toast(ctx, &e);
            }
        }
    }

    /// Window bookkeeping: size, position, focus, background updates, screenshot.
    fn window_logic(&mut self, ctx: &egui::Context) {
        // Process any IPC commands first.
        if self.daemon_mode {
            self.process_ui_cmds(ctx);

            // While hidden, don't run any timers or do any work.
            if self.hidden {
                return;
            }
        }

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
        if self.screenshot.is_none()
            && !self.was_focused
            && focused != Some(true)
            && self.focus_tries < 120
        {
            self.focus_tries += 1;
            if self.focus_tries % 6 == 1 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if !self.centered
            && let Some(m) = monitor
        {
            self.centered = true;
            let pos = Pos2::new(((m.x - WIDTH) / 2.0).max(0.0), (m.y * 0.22).round());
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        // Dismiss like a popover once focus moves elsewhere.
        if self.screenshot.is_none() {
            match focused {
                Some(true) => self.was_focused = true,
                Some(false) if self.was_focused && self.mode == Mode::Search => self.dismiss(ctx),
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
                ctx.request_repaint_after(Duration::from_millis(
                    ((SAVE_DELAY - (now - t)) * 1000.0) as u64 + 20,
                ));
            }
        }
        // The list is hidden in settings mode; `leave_settings` refreshes it.
        let in_search = self.mode == Mode::Search && !self.query.is_empty();
        let grown = in_search
            && self.files.lock().unwrap().len() != self.seen_files
            && now - self.last_auto_refresh > 0.5;
        if in_search && (grown || rates_ready != self.rates_ready) {
            self.last_auto_refresh = now;
            let sel = self.sel;
            self.refresh(ctx);
            self.sel = sel.min(self.rows.len().saturating_sub(1));
            self.scroll_to_sel = false;
        }
        if !self.files_done.load(Ordering::Relaxed) {
            // Record when done for cooldown tracking.
            ctx.request_repaint_after(Duration::from_millis(400));
        } else if self.last_index_done == 0.0 {
            self.last_index_done = now;
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
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), BAR_H),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add_space(16.0);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                draw::magnifier(ui.painter(), r, MUTED);
                ui.add_space(10.0);
                let edit = egui::TextEdit::singleline(&mut self.query)
                    .id(search_id())
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
            },
        );
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
        p.rect_filled(
            rect,
            10,
            egui::Color32::from_rgba_premultiplied(16, 16, 18, 230),
        );
        p.galley(rect.min + Vec2::new(10.0, 5.0), galley, FG);
    }
}

impl eframe::App for Glint {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    /// Scripted keys (screenshot checks) enter here, one per frame from frame 20,
    /// so they travel exactly like real key presses.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.hook_frame += 1;
        if self.screenshot.is_some()
            && let Some(&key) = self
                .hook_frame
                .checked_sub(20)
                .and_then(|i| self.key_script.get(i as usize))
        {
            for pressed in [true, false] {
                raw.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            }
        }
        // Navigation keys of the settings page never reach egui's own focus handling.
        if self.mode == Mode::Settings && !egui::Popup::is_any_open(_ctx) {
            self.take_nav_keys(raw);
        }
    }

    /// Closing within the save debounce must not lose settings changes.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.settings_dirty.is_some()
            && let Err(e) = self.settings.save()
        {
            eprintln!("Could not save settings: {e}");
        }
        index::save_history(&self.history);

        let warm = Warm {
            apps: std::mem::take(&mut self.apps),
            icons: std::mem::take(&mut self.icons),
            files: self.files.clone(),
            files_done: self.files_done.clone(),
            cancel: self.cancel.clone(),
            history: std::mem::take(&mut self.history),
            rates: self.rates.clone(),
            defs: self.defs.clone(),
            settings: self.settings.clone(),
            last_index_done: self.last_index_done,
            config_mtime: self.config_mtime,
            apps_dir_mtimes: std::mem::take(&mut self.apps_dir_mtimes),
        };
        *self.warm_slot.lock().unwrap() = Some(warm);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keys(&ctx);
        self.window_logic(&ctx);

        // While hidden in daemon mode, skip all drawing.
        if self.hidden {
            return;
        }

        let window = ui.max_rect();
        let full = Rect::from_min_size(
            window.min,
            Vec2::new(window.width(), self.height.min(window.height())),
        );
        draw::panel(ui.painter(), full, self.theme.bg);
        if self.mode == Mode::Settings {
            ui.scope_builder(UiBuilder::new().max_rect(full), |ui| self.settings_page(ui));
            self.status(ui, full);
            self.trace_first_frame();
            return;
        }
        // The see-through area below the panel acts as "outside": a click there closes.
        let below = Rect::from_min_max(Pos2::new(window.left(), full.bottom()), window.max);
        if below.height() > 1.0
            && ui
                .interact(below, ui.scope_id().with("outside"), Sense::click())
                .clicked()
            && self.screenshot.is_none()
        {
            self.dismiss(&ctx);
        }

        let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), BAR_H));
        ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| self.search_bar(ui));
        if self.rows.is_empty() || full.height() < BAR_H + 40.0 {
            self.trace_first_frame();
            return;
        }
        let top = full.top() + BAR_H;
        let line = Stroke::new(1.0, HAIRLINE);
        ui.painter().hline(full.x_range(), top, line);
        let list_right = if self.show_preview {
            full.left() + LIST_W
        } else {
            full.right()
        };
        let list_rect = Rect::from_min_max(
            Pos2::new(full.left(), top + 1.0),
            Pos2::new(list_right, full.bottom() - 8.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect);
            self.list(ui);
        });
        if self.show_preview {
            ui.painter()
                .vline(list_rect.right(), list_rect.y_range(), line);
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
        self.trace_first_frame();
    }
}

impl Glint {
    fn trace_first_frame(&mut self) {
        if std::env::var_os("GLINT_TRACE").is_some()
            && let Some(t0) = self.show_t0.take()
        {
            eprintln!(
                "[trace {} ms] first frame painted",
                t0.elapsed().as_millis()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// One-shot mode: open once, close when done (original behaviour).
pub fn run_oneshot(args: Args) -> eframe::Result {
    let screenshot = args.screenshot.clone();
    let query = args.query.clone();
    let open_settings = args.open_settings;
    let keys = args.keys.clone();
    let shot = screenshot.is_some();
    if shot && query.is_empty() && !open_settings {
        return run_oneshot(Args {
            query: "Notes".into(),
            ..args
        });
    }
    let viewport = build_viewport(shot, false);
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    if shot {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        options.event_loop_builder = Some(Box::new(|b| {
            b.with_activate_ignoring_other_apps(false);
        }));
    }
    eframe::run_native(
        "Glint",
        options,
        Box::new(move |cc| {
            let mut app = Glint::new(cc, screenshot, query);
            app.key_script = keys;
            if open_settings {
                app.enter_settings();
            }
            Ok(Box::new(app))
        }),
    )
}

/// Daemon mode: start with IPC listener, hide/show instead of exit.
#[cfg(target_os = "linux")]
pub fn run_daemon(
    args: Args,
    listener: std::os::unix::net::UnixListener,
    sock_path: PathBuf,
) -> eframe::Result {
    use crate::ipc;

    let is_wayland_env = std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("WINIT_UNIX_BACKEND").as_deref() != Ok("x11");

    let mut start_hidden = args.daemon; // --daemon starts hidden
    let mut open_settings = args.open_settings;
    let mut query = args.query.clone();

    let warm = Warm::new();
    let warm_slot: Arc<Mutex<Option<Warm>>> = Arc::new(Mutex::new(Some(warm)));
    let shared_window_id = Arc::new(AtomicU32::new(0));

    let ui_cmds: Arc<Mutex<Vec<UiCmd>>> = Arc::new(Mutex::new(Vec::new()));
    let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let quit_requested = Arc::new(AtomicBool::new(false));

    let (ipc_tx, ipc_rx) = std::sync::mpsc::channel::<UiCmd>();

    // Start IPC listener thread.
    let cmds = ui_cmds.clone();
    let ctx_slot2 = ctx_slot.clone();
    let shared_win = shared_window_id.clone();
    let quit_flag = quit_requested.clone();
    ipc::start_listener(listener, move |cmd| {
        let trace = std::env::var_os("GLINT_TRACE").is_some();
        let t_recv = if trace { Some(Instant::now()) } else { None };
        if trace {
            eprintln!("[trace 0 ms] IPC message received in listener: {cmd:?}");
        }
        let is_show = matches!(
            cmd,
            ipc::Cmd::Show | ipc::Cmd::Settings | ipc::Cmd::Query(_)
        );
        if is_show {
            let wid = shared_win.load(Ordering::Relaxed);
            if wid != 0 {
                crate::x11focus::map_and_activate(wid);
                if trace && let Some(t0) = t_recv {
                    eprintln!(
                        "[trace {} ms] map and activate sent from listener",
                        t0.elapsed().as_millis()
                    );
                }
            }
        }
        let ui_cmd = match cmd {
            ipc::Cmd::Show => UiCmd::Show(t_recv),
            ipc::Cmd::Settings => UiCmd::ShowSettings(t_recv),
            ipc::Cmd::Quit => {
                quit_flag.store(true, Ordering::SeqCst);
                UiCmd::Quit
            }
            ipc::Cmd::Query(text) => UiCmd::ShowQuery(text, t_recv),
        };

        let slot = ctx_slot2.lock().unwrap();
        if let Some(ctx) = slot.as_ref() {
            cmds.lock().unwrap().push(ui_cmd);
            ctx.request_repaint();
        } else {
            let _ = ipc_tx.send(ui_cmd);
        }
    });

    let mut result = Ok(());
    let mut t_open: Option<Instant> = None;

    loop {
        // If on Wayland and starting hidden: don't open window yet; block on IPC channel.
        if is_wayland_env && start_hidden {
            match ipc_rx.recv() {
                Ok(UiCmd::Quit) | Err(_) => break,
                Ok(UiCmd::Show(t0)) => {
                    start_hidden = false;
                    t_open = t0;
                }
                Ok(UiCmd::ShowSettings(t0)) => {
                    start_hidden = false;
                    open_settings = true;
                    t_open = t0;
                }
                Ok(UiCmd::ShowQuery(text, t0)) => {
                    start_hidden = false;
                    query = text;
                    t_open = t0;
                }
            }
        }

        let warm = warm_slot.lock().unwrap().take().unwrap_or_else(Warm::new);
        let viewport = build_viewport(false, start_hidden);
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        };

        let c_slot = ctx_slot.clone();
        let w_slot = warm_slot.clone();
        let u_cmds = ui_cmds.clone();
        let s_win = shared_window_id.clone();
        let cur_query = query.clone();
        let cur_settings = open_settings;
        let cur_hidden = start_hidden;
        let cur_t0 = t_open.take();

        result = eframe::run_native(
            "Glint",
            options,
            Box::new(move |cc| {
                *c_slot.lock().unwrap() = Some(cc.egui_ctx.clone());

                let mut app = Glint::from_warm(
                    cc, warm, None, cur_query, true, u_cmds, w_slot, s_win, cur_hidden,
                );
                app.show_t0 = cur_t0;

                if cur_settings {
                    app.enter_settings();
                }

                Ok(Box::new(app))
            }),
        );

        // run_native returned. Clear active egui Context and drain leftover commands under the lock.
        let leftover = {
            let mut slot = ctx_slot.lock().unwrap();
            *slot = None;
            let mut q = ui_cmds.lock().unwrap();
            std::mem::take(&mut *q)
        };

        if quit_requested.load(Ordering::SeqCst) {
            break;
        }

        // On X11, run_native only returns when the process is quitting.
        if !is_wayland_env {
            break;
        }

        // On Wayland, "hide" closes the window so run_native returns.
        // Check if any show/settings/query command is pending in leftover.
        // Quit wins over everything else.
        let mut pending_cmd = None;
        for cmd in leftover {
            if matches!(cmd, UiCmd::Quit) {
                pending_cmd = Some(UiCmd::Quit);
                break;
            }
            pending_cmd = Some(cmd);
        }

        let mut next_cmd = if let Some(cmd) = pending_cmd {
            cmd
        } else {
            match ipc_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        };

        while let Ok(cmd) = ipc_rx.try_recv() {
            if matches!(cmd, UiCmd::Quit) {
                next_cmd = UiCmd::Quit;
                break;
            }
            next_cmd = cmd;
        }

        start_hidden = false;
        open_settings = false;
        query.clear();

        match next_cmd {
            UiCmd::Quit => break,
            UiCmd::Show(t0) => {
                t_open = t0;
            }
            UiCmd::ShowSettings(t0) => {
                open_settings = true;
                t_open = t0;
            }
            UiCmd::ShowQuery(text, t0) => {
                query = text;
                t_open = t0;
            }
        }
    }

    // Cleanup: remove the socket.
    let _ = std::fs::remove_file(&sock_path);

    result
}

fn build_viewport(screenshot: bool, hidden: bool) -> egui::ViewportBuilder {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Glint")
        .with_app_id("glint")
        .with_inner_size([WIDTH, MAX_H])
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(true);
    if hidden {
        viewport = viewport.with_visible(false);
    }
    if screenshot {
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    } else {
        viewport = viewport.with_always_on_top();
    }
    viewport
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
    let c = p
        .canonicalize()
        .map_err(|e| format!("Cannot read that folder: {e}"))?;
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

    #[test]
    fn search_selection_and_focus_state() {
        let ctx = egui::Context::default();
        let id = search_id();
        let query = "hello world";
        let len = query.chars().count();
        let mut state = TextEditState::load(&ctx, id).unwrap_or_default();
        let c0 = CCursor::new(0);
        let c1 = CCursor::new(len);
        state.cursor.set_char_range(Some(CCursorRange::two(c0, c1)));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));

        let loaded = TextEditState::load(&ctx, id).unwrap();
        assert_eq!(
            loaded.cursor.char_range(),
            Some(CCursorRange::two(CCursor::new(0), CCursor::new(11)))
        );
        assert!(ctx.memory(|m| m.has_focus(id)));
    }
}
