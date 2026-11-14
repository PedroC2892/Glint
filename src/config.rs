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
}
