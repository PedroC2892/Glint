//! What can be found: installed applications, settings panels, files in the
//! home folder, plus usage history and file details for the preview.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub name_lc: String,
    pub generic: String,
    pub generic_lc: String,
    pub comment: String,
    pub keywords_lc: String,
    pub exec: String,
    pub icon: String,
    pub terminal: bool,
    pub settings: bool,
    pub path: PathBuf,
}

pub struct FileEntry {
    pub path: PathBuf,
    pub name_lc: String,
    pub is_dir: bool,
    pub depth: u8,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
}

/// $XDG_DATA_HOME first, then $XDG_DATA_DIRS, as the XDG spec orders them.
pub fn data_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::iter::once(data_home())
        .chain(dirs.split(':').map(PathBuf::from).filter(|p| p.is_absolute()))
        .collect()
}

pub fn load_apps() -> Vec<AppEntry> {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .map(|s| s.to_lowercase())
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut apps = Vec::new();
    for dir in data_dirs() {
        let root = dir.join("applications");
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "desktop") {
                    continue;
                }
                // Desktop file id: path below applications/ with '/' replaced by '-'.
                let id = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('/', "-");
                if !seen.insert(id.clone()) {
                    continue; // An earlier data dir overrides this one.
                }
                if let Some(app) = parse_desktop(&p, id, &desktops) {
                    apps.push(app);
                }
            }
        }
    }
    apps
}

fn parse_desktop(path: &Path, id: String, desktops: &[String]) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut kv = HashMap::new();
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
        } else if in_main && let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
        }
    }
    let get = |k: &str| kv.get(k).cloned().unwrap_or_default();
    let flag = |k: &str| kv.get(k).is_some_and(|v| v == "true");
    if get("Type") != "Application" || flag("Hidden") || get("Exec").is_empty() {
        return None;
    }
    let in_list = |k: &str| get(k).split(';').any(|d| desktops.contains(&d.to_lowercase()));
    if (kv.contains_key("OnlyShowIn") && !in_list("OnlyShowIn")) || in_list("NotShowIn") {
        return None;
    }
    // KDE panels carry no categories, so their settings launcher marks them.
    let exec = get("Exec");
    let launcher = exec.split_whitespace().next().unwrap_or_default();
    let settings_cat = get("Categories")
        .split(';')
        .any(|c| c == "Settings" || c == "X-GNOME-Settings-Panel")
        || ["systemsettings", "gnome-control-center", "kcmshell6", "kcmshell5"].contains(&launcher);
    // Hidden entries are kept only when they are settings panels.
    let settings = flag("NoDisplay") && settings_cat;
    if flag("NoDisplay") && !settings {
        return None;
    }
    let name = get("Name");
    if name.is_empty() {
        return None;
    }
    Some(AppEntry {
        id,
        name_lc: name.to_lowercase(),
        name,
        generic: get("GenericName"),
        generic_lc: get("GenericName").to_lowercase(),
        comment: get("Comment"),
        keywords_lc: get("Keywords").to_lowercase(),
        exec: get("Exec"),
        icon: get("Icon"),
        terminal: flag("Terminal"),
        settings,
        path: path.to_path_buf(),
    })
}

const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "venv", "site-packages", "snap"];
const MAX_FILES: usize = 400_000;

/// Walk the home folder, publishing entries in batches; calls `tick` after each batch.
// ponytail: full rescan on each start and names only (no file contents); an
// inotify-fed persistent index is the upgrade if homes get much larger.
pub fn index_files(out: Arc<Mutex<Vec<FileEntry>>>, done: Arc<AtomicBool>, tick: impl Fn()) {
    let mut stack = vec![(home(), 0u8)];
    let mut batch = Vec::new();
    let mut total = 0;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let is_dir = ft.is_dir();
            if is_dir && depth < 10 && !SKIP_DIRS.contains(&name.as_str()) {
                stack.push((e.path(), depth + 1));
            }
            batch.push(FileEntry { path: e.path(), name_lc: name.to_lowercase(), is_dir, depth });
            total += 1;
        }
        if batch.len() > 4000 || total >= MAX_FILES {
            out.lock().unwrap().append(&mut batch);
            tick();
        }
        if total >= MAX_FILES {
            break;
        }
    }
    out.lock().unwrap().append(&mut batch);
    done.store(true, Ordering::Relaxed);
    tick();
}

/// Name match score: exact > prefix > word prefix > initials > substring.
/// Runs for every indexed name on each keystroke, so it never allocates.
pub fn score(name_lc: &str, q: &str) -> i32 {
    let words = || name_lc.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    if !name_lc.contains(q) {
        // Only initials ("vsc" for "visual studio code") can match now.
        let mut initials = words().filter_map(|w| w.chars().next());
        let is_initials = q.chars().count() >= 2 && q.chars().all(|c| initials.next() == Some(c));
        return if is_initials { 50 } else { 0 };
    }
    if name_lc == q {
        100
    } else if name_lc.starts_with(q) {
        80
    } else if words().any(|w| w.starts_with(q)) {
        60
    } else {
        40
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Document,
    Image,
    Music,
    Movie,
    Other,
}

const DOC_EXT: &[&str] = &[
    "pdf", "txt", "md", "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt", "pptx", "odp",
    "csv", "epub", "tex", "html", "json", "toml", "yaml", "yml", "log",
];
const IMG_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tif", "tiff", "heic"];
const MUSIC_EXT: &[&str] = &["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac"];
const MOVIE_EXT: &[&str] = &["mp4", "mkv", "webm", "mov", "avi", "m4v"];
const TEXT_EXT: &[&str] = &[
    "txt", "md", "csv", "json", "toml", "yaml", "yml", "log", "rs", "py", "js", "ts", "c", "h",
    "cpp", "sh", "html", "css", "xml", "ini", "conf", "tex", "go", "java",
];

pub fn ext_lc(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn kind(p: &Path, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let e = ext_lc(p);
    let e = e.as_str();
    if IMG_EXT.contains(&e) {
        Kind::Image
    } else if DOC_EXT.contains(&e) {
        Kind::Document
    } else if MUSIC_EXT.contains(&e) {
        Kind::Music
    } else if MOVIE_EXT.contains(&e) {
        Kind::Movie
    } else {
        Kind::Other
    }
}

pub fn kind_text(path: &Path, is_dir: bool) -> String {
    let e = ext_lc(path);
    let up = e.to_uppercase();
    match (kind(path, is_dir), e.as_str()) {
        (Kind::Folder, _) => "Folder".into(),
        (_, "pdf") => "PDF document".into(),
        (_, "txt") => "Plain text".into(),
        (_, "md") => "Markdown document".into(),
        (_, "") => "Document".into(),
        (Kind::Image, _) => format!("{up} image"),
        (Kind::Music, _) => format!("{up} audio"),
        (Kind::Movie, _) => format!("{up} video"),
        _ => format!("{up} document"),
    }
}

// --- usage history: `<count>\t<key>` per line, key `app:<id>` or `file:<path>`

fn history_path() -> PathBuf {
    let glint_path = data_home().join("glint").join("history.tsv");
    let legacy_path = data_home().join("lookout").join("history.tsv");
    if !glint_path.exists() && legacy_path.exists() {
        if let Some(parent) = glint_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::copy(&legacy_path, &glint_path);
    }
    glint_path
}

pub fn load_history() -> HashMap<String, u32> {
    std::fs::read_to_string(history_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (n, k) = l.split_once('\t')?;
            Some((k.to_string(), n.parse().ok()?))
        })
        .collect()
}

pub fn save_history(h: &HashMap<String, u32>) {
    let path = history_path();
    let body: String = h
        .iter()
        .filter(|(k, _)| !k.contains('\n'))
        .map(|(k, n)| format!("{n}\t{k}\n"))
        .collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, body);
}

// --- file details for the preview pane

pub struct FileInfo {
    pub path: PathBuf,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub items: Option<usize>,
    pub text: Option<String>,
}

pub fn file_info(path: &Path, is_dir: bool) -> FileInfo {
    let meta = std::fs::metadata(path).ok();
    let mut text = None;
    if !is_dir && TEXT_EXT.contains(&ext_lc(path).as_str()) && let Ok(f) = std::fs::File::open(path) {
        let mut buf = Vec::new();
        let _ = f.take(3000).read_to_end(&mut buf);
        text = Some(String::from_utf8_lossy(&buf).into_owned());
    }
    FileInfo {
        path: path.to_path_buf(),
        size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
        modified: meta.as_ref().and_then(|m| m.modified().ok()),
        created: meta.as_ref().and_then(|m| m.created().ok()),
        items: is_dir.then(|| std::fs::read_dir(path).map(|r| r.count()).unwrap_or(0)),
        text,
    }
}

pub fn format_size(b: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { format!("{b} bytes") } else { format!("{v:.1} {}", units[i]) }
}

/// Local UTC offset in seconds, asked once from `date` (0 if unavailable).
// ponytail: one offset for all dates, so DST shifts dates from the other
// half of the year by an hour; read /etc/localtime rules if that matters.
pub fn utc_offset() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output();
    let s = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let (sign, digits) = match s.strip_prefix('-') {
        Some(d) => (-1, d),
        None => (1, s.trim_start_matches('+')),
    };
    match (digits.get(0..2).and_then(|h| h.parse::<i64>().ok()), digits.get(2..4).and_then(|m| m.parse::<i64>().ok())) {
        (Some(h), Some(m)) if digits.len() == 4 => sign * (h * 3600 + m * 60),
        _ => 0,
    }
}

/// Day number to (year, month, day), after Howard Hinnant's civil_from_days.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "today, 9:47 pm", "yesterday, 8:02 am" or "28/03/2024, 8:33 am" in local time.
pub fn format_time(t: SystemTime, offset: i64, now: SystemTime) -> String {
    let local = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) + offset;
    let (secs, now_secs) = (local(t), local(now));
    let (day, today) = (secs.div_euclid(86_400), now_secs.div_euclid(86_400));
    let minutes = secs.rem_euclid(86_400) / 60;
    let (h, m) = (minutes / 60, minutes % 60);
    let clock = format!("{}:{m:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "am" } else { "pm" });
    match today - day {
        0 => format!("today, {clock}"),
        1 => format!("yesterday, {clock}"),
        _ => {
            let (y, mo, d) = civil(day);
            format!("{d:02}/{mo:02}/{y}, {clock}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_and_dates() {
        assert_eq!(score("firefox", "firefox"), 100);
        assert_eq!(score("firefox", "fire"), 80);
        assert_eq!(score("gnu image manipulation", "image"), 60);
        assert_eq!(score("visual studio code", "vsc"), 50);
        assert_eq!(score("libreoffice", "office"), 40);
        assert_eq!(score("terminal", "xyz"), 0);
        let at = |s: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let t = at(1_700_000_000); // 2023-11-14 22:13:20 UTC
        assert_eq!(format_time(t, 0, at(1_800_000_000)), "14/11/2023, 10:13 pm");
        assert_eq!(format_time(t, 0, t), "today, 10:13 pm");
        assert_eq!(format_time(t, 3 * 3600, t), "today, 1:13 am");
        assert_eq!(format_time(t, 0, at(1_700_000_000 + 86_400)), "yesterday, 10:13 pm");
        assert_eq!(format_size(1_234_567), "1.2 MB");
    }
}
