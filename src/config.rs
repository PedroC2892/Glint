// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 1 << 20;
const DEFAULT_OPACITY: f32 = 0.92;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub accent: [u8; 3],
    /// Valid range 0.5..=1.0.
    pub opacity: f32,
    pub show_hidden: bool,
    pub outside_home: bool,
    pub extra_roots: Vec<PathBuf>,
    pub skip_dirs: Vec<String>,
    /// Allows network lookups (definitions, exchange rates).
    pub online: bool,
    pub keep_running: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            accent: [52, 120, 246],
            opacity: DEFAULT_OPACITY,
            show_hidden: false,
            outside_home: false,
            extra_roots: ["/media", "/mnt", "/run/media"].map(PathBuf::from).to_vec(),
            skip_dirs: [
                "node_modules",
                "target",
                "__pycache__",
                "venv",
                "site-packages",
                "snap",
                ".npm",
                ".rustup",
                ".cargo",
                ".bun",
                ".nvm",
                ".gradle",
                ".m2",
                ".pnpm-store",
                "flatpak",
                "fnm",
                "pnpm",
                "pipx",
            ]
            .map(String::from)
            .to_vec(),
            online: true,
            keep_running: true,
        }
    }
}

pub fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::index::home().join(".config"))
        .join("glint/config.json")
}

pub fn autostart_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::index::home().join(".config"))
        .join("autostart/glint.desktop")
}

/// Quotes and escapes a path for an Exec field per the Desktop Entry spec.
pub fn desktop_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str(r#"\""#),
            '`' => out.push_str(r"\`"),
            '$' => out.push_str(r"\$"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Returns the contents of the autostart .desktop file for the given setting.
pub fn autostart_entry(keep_running: bool, exe: &Path) -> String {
    if keep_running {
        let exec = desktop_quote(&exe.to_string_lossy());
        format!(
            "[Desktop Entry]\nType=Application\nName=Glint\nExec={exec} --daemon\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-phase=2\n"
        )
    } else {
        "[Desktop Entry]\nType=Application\nName=Glint\nHidden=true\n".to_string()
    }
}

/// Writes the autostart entry to `path` atomically if its content differs.
pub fn write_autostart(path: &Path, keep_running: bool, exe: &Path) -> Result<(), String> {
    let content = autostart_entry(keep_running, exe);
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == content
    {
        return Ok(());
    }
    let dir = path.parent().ok_or("autostart path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("glint.desktop.tmp.{}", std::process::id()));
    std::fs::write(&tmp, &content).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Syncs the current executable into the user autostart entry.
pub fn sync_autostart(keep_running: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    write_autostart(&autostart_path(), keep_running, &exe)
}

/// Repairs the autostart entry if the on/off state drifted, without rewriting
/// the Exec path of an existing enabled entry.
pub fn repair_autostart(path: &Path, keep_running: bool, exe: &Path) -> Result<(), String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return write_autostart(path, keep_running, exe);
        }
        Err(e) => return Err(e.to_string()),
    };
    let has_hidden = content.lines().any(|l| {
        let s = l.trim();
        s == "Hidden=true"
            || s.split_once('=')
                .map(|(k, v)| (k.trim(), v.trim()))
                == Some(("Hidden", "true"))
    });
    if keep_running && has_hidden {
        write_autostart(path, true, exe)
    } else if !keep_running && !has_hidden {
        write_autostart(path, false, exe)
    } else {
        Ok(())
    }
}

/// Loads the settings; on a read or parse error returns defaults plus a message
/// and leaves the broken file untouched.
pub fn load() -> (Settings, Option<String>) {
    load_from(&config_path())
}

fn load_from(path: &Path) -> (Settings, Option<String>) {
    let read = || -> Result<Settings, String> {
        let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if len > MAX_FILE_BYTES {
            return Err(format!("config file too large ({len} bytes)"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    };
    match read() {
        Ok(mut s) => {
            s.sanitize();
            (s, None)
        }
        Err(_) if !path.exists() => (Settings::default(), None),
        Err(e) => (
            Settings::default(),
            Some(format!("Could not load {}: {e}", path.display())),
        ),
    }
}

impl Settings {
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&config_path())
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("config path has no parent")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn sanitize(&mut self) {
        self.opacity = if self.opacity.is_nan() {
            DEFAULT_OPACITY
        } else {
            self.opacity.clamp(0.5, 1.0)
        };
        let mut roots: Vec<PathBuf> = Vec::new();
        for r in self.extra_roots.drain(..) {
            if r.is_absolute() && !roots.contains(&r) {
                roots.push(r);
            }
        }
        self.extra_roots = roots;
        let mut dirs: Vec<String> = Vec::new();
        for d in self.skip_dirs.drain(..) {
            let d = d.trim().to_string();
            if !d.is_empty() && !d.contains('/') && !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        self.skip_dirs = dirs;
    }

    /// True when a change requires re-indexing.
    pub fn index_differs(&self, other: &Settings) -> bool {
        self.show_hidden != other.show_hidden
            || self.outside_home != other.outside_home
            || self.extra_roots != other.extra_roots
            || self.skip_dirs != other.skip_dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d =
            std::env::temp_dir().join(format!("glint-test-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trip() {
        let d = temp_dir("rt");
        let p = d.join("sub/config.json");
        let s = Settings {
            accent: [1, 2, 3],
            opacity: 0.6,
            show_hidden: true,
            ..Settings::default()
        };
        s.save_to(&p).unwrap();
        assert_eq!(load_from(&p), (s, None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let d = temp_dir("partial");
        let p = d.join("config.json");
        std::fs::write(&p, r#"{"opacity": 0.7}"#).unwrap();
        let (s, err) = load_from(&p);
        assert!(err.is_none());
        assert_eq!(
            s,
            Settings {
                opacity: 0.7,
                ..Settings::default()
            }
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn invalid_json_keeps_file() {
        let d = temp_dir("bad");
        let p = d.join("config.json");
        std::fs::write(&p, "{ not json").unwrap();
        let (s, err) = load_from(&p);
        assert_eq!(s, Settings::default());
        assert!(err.is_some());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not json");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_file_is_defaults() {
        let d = temp_dir("missing");
        let p = d.join("config.json");
        assert_eq!(load_from(&p), (Settings::default(), None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sanitize_cleans_values() {
        let mut s = Settings {
            opacity: 3.0,
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.opacity, 1.0);
        s.opacity = f32::NAN;
        s.sanitize();
        assert_eq!(s.opacity, 0.92);
        s.extra_roots = ["media", "/mnt", "/mnt"].map(PathBuf::from).to_vec();
        s.skip_dirs = ["  a/b ", "", " x ", "x"].map(String::from).to_vec();
        s.sanitize();
        assert_eq!(s.extra_roots, vec![PathBuf::from("/mnt")]);
        assert_eq!(s.skip_dirs, vec!["x".to_string()]);
    }

    #[test]
    fn index_differs_only_for_index_fields() {
        let a = Settings::default();
        let b = Settings {
            accent: [0, 0, 0],
            ..a.clone()
        };
        assert!(!a.index_differs(&b));
        let c = Settings {
            show_hidden: true,
            ..a.clone()
        };
        assert!(a.index_differs(&c));
    }

    #[test]
    fn keep_running_defaults_to_true_when_omitted() {
        let d = temp_dir("kr-default");
        let p = d.join("config.json");
        std::fs::write(&p, r#"{"online": false}"#).unwrap();
        let (s, err) = load_from(&p);
        assert!(err.is_none());
        assert!(s.keep_running);
        assert!(!s.online);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn desktop_quote_escaping() {
        let p = Path::new(r#"/usr/local/bin/glint "special space""#);
        let q = desktop_quote(&p.to_string_lossy());
        assert_eq!(q, r#""/usr/local/bin/glint \"special space\"""#);
        let complex = desktop_quote(r#"a\b"c`d$e"#);
        assert_eq!(complex, r#""a\\b\"c\`d\$e""#);
    }

    #[test]
    fn autostart_entry_content() {
        let exe = Path::new("/usr/bin/glint");
        let on = autostart_entry(true, exe);
        assert_eq!(
            on,
            "[Desktop Entry]\nType=Application\nName=Glint\nExec=\"/usr/bin/glint\" --daemon\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-phase=2\n"
        );
        let off = autostart_entry(false, exe);
        assert_eq!(
            off,
            "[Desktop Entry]\nType=Application\nName=Glint\nHidden=true\n"
        );
    }

    #[test]
    fn write_autostart_in_tempdir() {
        let d = temp_dir("autostart-test");
        let p = d.join("autostart/glint.desktop");
        let exe = Path::new("/usr/bin/glint");

        write_autostart(&p, true, exe).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text, autostart_entry(true, exe));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o644);
        }

        // Writing same content again does not rewrite file
        let mtime1 = std::fs::metadata(&p).unwrap().modified().unwrap();
        write_autostart(&p, true, exe).unwrap();
        let mtime2 = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2);

        // Updating to false writes override
        write_autostart(&p, false, exe).unwrap();
        let text2 = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text2, autostart_entry(false, exe));

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn repair_autostart_cases() {
        let d = temp_dir("repair-autostart");
        let p = d.join("glint.desktop");
        let exe = Path::new("/usr/bin/glint");
        let custom_exe = Path::new("/opt/custom/glint");

        // 1. File missing: writes entry for current state.
        repair_autostart(&p, true, exe).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            autostart_entry(true, exe)
        );

        // 2. keep_running == false and file has no Hidden=true line: writes Hidden=true override.
        repair_autostart(&p, false, exe).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            autostart_entry(false, exe)
        );

        // 3. keep_running == true and file contains Hidden=true line: writes enabled entry.
        repair_autostart(&p, true, exe).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            autostart_entry(true, exe)
        );

        // 4. Otherwise: leaves file untouched.
        // Enabled entry with a different Exec is NOT rewritten.
        let custom_content = autostart_entry(true, custom_exe);
        std::fs::write(&p, &custom_content).unwrap();
        repair_autostart(&p, true, exe).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), custom_content);

        // Disabled entry with Hidden=true is untouched when keep_running == false.
        let hidden_custom = "[Desktop Entry]\nType=Application\nName=Glint\nHidden=true\n# custom\n";
        std::fs::write(&p, hidden_custom).unwrap();
        repair_autostart(&p, false, exe).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), hidden_custom);

        // File missing when keep_running == false writes disabled entry.
        let p_missing_false = d.join("missing_false.desktop");
        repair_autostart(&p_missing_false, false, exe).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p_missing_false).unwrap(),
            autostart_entry(false, exe)
        );

        let _ = std::fs::remove_dir_all(&d);
    }
}
