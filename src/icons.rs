//! Icons from the desktop's own icon theme.
//!
//! Follows the freedesktop icon theme spec closely enough for a launcher: the
//! active theme (KDE `kdeglobals`, else GNOME `gsettings`), its `Inherits=`
//! chain, then `hicolor`. File icons come from the shared MIME database, so a
//! PDF gets the theme's PDF icon and a folder the theme's folder icon.

use crate::index;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct IconTheme {
    /// Icon directories in lookup order (theme chain, best sizes first).
    dirs: Vec<PathBuf>,
    cache: HashMap<String, Option<PathBuf>>,
    /// "pdf" -> "application/pdf"
    ext_mime: HashMap<String, String>,
    /// "text/x-markdown" -> "text-x-generic"
    generic: HashMap<String, String>,
}

fn icon_bases() -> Vec<PathBuf> {
    let mut bases = vec![index::home().join(".icons")];
    bases.extend(index::data_dirs().into_iter().map(|d| d.join("icons")));
    bases
}

/// Name of the icon theme the desktop uses.
fn current_theme() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| index::home().join(".config"));
    let kde = std::fs::read_to_string(config.join("kdeglobals"))
        .ok()
        .and_then(|text| {
            let mut in_icons = false;
            text.lines().find_map(|l| {
                let l = l.trim();
                if l.starts_with('[') {
                    in_icons = l == "[Icons]";
                    None
                } else if in_icons {
                    l.strip_prefix("Theme=").map(str::to_string)
                } else {
                    None
                }
            })
        });
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    if desktop.contains("kde") && kde.is_some() {
        return kde;
    }
    let gnome = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .trim_matches('\'')
                .to_string()
        })
        .filter(|s| !s.is_empty());
    gnome.or(kde)
}

fn theme_dirs(bases: &[PathBuf], name: &str) -> Vec<PathBuf> {
    bases
        .iter()
        .map(|b| b.join(name))
        .filter(|d| d.is_dir())
        .collect()
}

/// Rank an icon sub-directory: scalable first, then sizes near 64 px,
/// symbolic (one-colour) icons last.
fn rank(dir: &Path) -> u32 {
    let parts: Vec<String> = dir
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.iter().any(|p| p.contains("symbolic")) {
        return 1000;
    }
    if parts.iter().any(|p| p.starts_with("scalable")) {
        return 0;
    }
    let size = parts
        .iter()
        .find_map(|p| {
            p.split(['x', '@'])
                .next()
                .and_then(|n| n.parse::<i32>().ok())
        })
        .unwrap_or(16);
    1 + (size - 64).unsigned_abs()
}

/// Every two-level sub-directory of a theme (`48x48/apps`, `apps/48`, `places/scalable`).
fn icon_dirs(theme_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for a in std::fs::read_dir(theme_dir).into_iter().flatten().flatten() {
        if a.file_type().is_ok_and(|t| t.is_dir()) {
            for b in std::fs::read_dir(a.path()).into_iter().flatten().flatten() {
                if b.file_type().is_ok_and(|t| t.is_dir()) {
                    out.push(b.path());
                }
            }
        }
    }
    out.sort_by_key(|d| rank(d));
    out
}

fn inherits(theme_dirs: &[PathBuf]) -> Vec<String> {
    theme_dirs
        .iter()
        .filter_map(|d| std::fs::read_to_string(d.join("index.theme")).ok())
        .flat_map(|t| {
            t.lines()
                .find_map(|l| l.trim().strip_prefix("Inherits=").map(str::to_string))
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

impl IconTheme {
    pub fn load() -> Self {
        let bases = icon_bases();
        let mut queue: Vec<String> = current_theme().into_iter().collect();
        // The panel is black: prefer a theme's dark variant when there is one.
        if let Some(t) = queue.first().cloned()
            && !t.to_lowercase().contains("dark")
            && !theme_dirs(&bases, &format!("{t}-dark")).is_empty()
        {
            queue.insert(0, format!("{t}-dark"));
        }
        queue.extend(["breeze-dark", "breeze", "Adwaita"].map(String::from));
        // Depth-first over Inherits=, each theme once; hicolor goes last, as
        // the spec's final fallback.
        let mut order: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        while !queue.is_empty() {
            let t = queue.remove(0);
            if t == "hicolor" || !seen.insert(t.clone()) {
                continue;
            }
            let dirs = theme_dirs(&bases, &t);
            for (i, parent) in inherits(&dirs).into_iter().enumerate() {
                queue.insert(i, parent);
            }
            order.push(t);
        }
        order.push("hicolor".into());

        let mut dirs = Vec::new();
        for t in &order {
            for d in theme_dirs(&bases, t) {
                dirs.extend(icon_dirs(&d));
            }
        }
        dirs.push(PathBuf::from("/usr/share/pixmaps"));

        let mut ext_mime = HashMap::new();
        let mut generic = HashMap::new();
        for d in index::data_dirs() {
            // globs2 is sorted by weight, highest first: keep the first match.
            for line in std::fs::read_to_string(d.join("mime/globs2"))
                .unwrap_or_default()
                .lines()
            {
                let mut f = line.splitn(3, ':');
                if let (Some(_), Some(mime), Some(glob)) = (f.next(), f.next(), f.next())
                    && let Some(ext) = glob.strip_prefix("*.")
                    && !ext.contains(['*', '?', '['])
                {
                    ext_mime
                        .entry(ext.to_lowercase())
                        .or_insert_with(|| mime.to_string());
                }
            }
            for line in std::fs::read_to_string(d.join("mime/generic-icons"))
                .unwrap_or_default()
                .lines()
            {
                if let Some((mime, icon)) = line.split_once(':') {
                    generic
                        .entry(mime.to_string())
                        .or_insert_with(|| icon.to_string());
                }
            }
        }
        Self {
            dirs,
            cache: HashMap::new(),
            ext_mime,
            generic,
        }
    }

    /// Path of one icon by name (or an absolute path), cached.
    pub fn lookup(&mut self, name: &str) -> Option<PathBuf> {
        if name.is_empty() {
            return None;
        }
        if let Some(hit) = self.cache.get(name) {
            return hit.clone();
        }
        let p = Path::new(name);
        let found = if p.is_absolute() {
            p.is_file().then(|| p.to_path_buf())
        } else {
            self.dirs.iter().find_map(|d| {
                ["svg", "png"]
                    .iter()
                    .map(|e| d.join(format!("{name}.{e}")))
                    .find(|f| f.is_file())
            })
        };
        self.cache.insert(name.to_string(), found.clone());
        found
    }

    /// The first of several names the theme has.
    pub fn first(&mut self, names: &[&str]) -> Option<PathBuf> {
        names.iter().find_map(|n| self.lookup(n))
    }

    /// Icon for a file or folder, as the file manager would show it.
    pub fn for_file(&mut self, path: &Path, is_dir: bool) -> Option<PathBuf> {
        if is_dir {
            let home = index::home();
            let special = match path.strip_prefix(&home).ok().and_then(|p| p.to_str()) {
                Some("") => "user-home",
                Some("Desktop") => "user-desktop",
                Some("Documents") => "folder-documents",
                Some("Downloads") => "folder-download",
                Some("Music") => "folder-music",
                Some("Pictures") => "folder-pictures",
                Some("Videos") => "folder-videos",
                _ => "folder",
            };
            return self.first(&[special, "folder"]);
        }
        let ext = index::ext_lc(path);
        let mut names: Vec<String> = Vec::new();
        if let Some(m) = self.ext_mime.get(&ext).cloned() {
            names.push(m.replace('/', "-"));
            if let Some(g) = self.generic.get(&m) {
                names.push(g.clone());
            }
            if let Some((major, _)) = m.split_once('/') {
                names.push(format!("{major}-x-generic"));
            }
        }
        names.extend(["text-x-generic", "application-x-generic", "unknown"].map(String::from));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.first(&refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_ranking() {
        assert_eq!(rank(Path::new("/t/kora/places/scalable")), 0);
        assert!(
            rank(Path::new("/t/hicolor/64x64/apps")) < rank(Path::new("/t/hicolor/16x16/apps"))
        );
        assert!(rank(Path::new("/t/breeze/apps/48")) < rank(Path::new("/t/breeze/apps/16")));
        assert_eq!(rank(Path::new("/t/kora/apps/symbolic")), 1000);
    }
}
