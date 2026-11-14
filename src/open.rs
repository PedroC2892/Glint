// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening things (apps, files, folders, web pages) and the two online
//! lookups: exchange rates and word definitions.

use crate::index::AppEntry;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn spawn(argv: &[String]) -> std::io::Result<()> {
    let (cmd, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Split a desktop-entry Exec line into arguments (quotes and backslash
/// escapes as the spec describes) and drop the %-field codes. No shell runs.
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut quote: Option<char> = None;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => cur.extend(chars.next()),
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, '\\') => cur.extend(chars.next()),
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
                has = false;
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        args.push(cur);
    }
    args.into_iter()
        .filter(|a| !(a.len() == 2 && a.starts_with('%') && a != "%%"))
        .map(|a| a.replace("%%", "%"))
        .collect()
}

pub fn launch_app(app: &AppEntry) -> std::io::Result<()> {
    let mut argv = split_exec(&app.exec);
    if app.terminal {
        let term = std::env::var("TERMINAL").unwrap_or_else(|_| "x-terminal-emulator".into());
        argv.splice(0..0, [term, "-e".to_string()]);
    }
    spawn(&argv)
}

pub fn xdg_open(target: &str) -> std::io::Result<()> {
    spawn(&["xdg-open".into(), target.into()])
}

pub fn percent_encode(s: &str, keep: &[u8]) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || keep.contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Show a file selected in the file manager, or open its folder.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    let uri = format!("file://{}", percent_encode(&path.to_string_lossy(), b"/"));
    let ok = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("['{uri}']"),
            "",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        return Ok(());
    }
    xdg_open(&path.parent().unwrap_or(path).to_string_lossy())
}

pub fn web_url(q: &str) -> String {
    format!("https://duckduckgo.com/?q={}", percent_encode(q, b""))
}

// --- online lookups

#[derive(Clone)]
pub enum Fetch<T> {
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Clone)]
pub struct Definition {
    pub word: String,
    pub phonetic: String,
    pub meanings: Vec<(String, Vec<String>)>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get_json(url: &str) -> Result<(u16, serde_json::Value), String> {
    let mut resp = agent().get(url).call().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let v = resp
        .body_mut()
        .read_json()
        .unwrap_or(serde_json::Value::Null);
    Ok((status, v))
}

/// Exchange rates against USD from a free public table. Only the table is
/// requested; nothing the person typed is sent.
pub fn fetch_rates() -> Result<HashMap<String, f64>, String> {
    let (status, v) = get_json("https://open.er-api.com/v6/latest/USD")?;
    let rates = v["rates"]
        .as_object()
        .ok_or(format!("Rate service answered {status}"))?;
    Ok(rates
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
        .collect())
}

/// Definition of one English word (ASCII letters only, checked by the caller).
pub fn fetch_definition(word: &str) -> Result<Definition, String> {
    let (status, v) = get_json(&format!(
        "https://api.dictionaryapi.dev/api/v2/entries/en/{word}"
    ))?;
    if status == 404 {
        return Err(format!("No definition found for “{word}”."));
    }
    let entry = v.get(0).ok_or(format!("Dictionary answered {status}"))?;
    let mut def = Definition {
        word: entry["word"].as_str().unwrap_or(word).to_string(),
        phonetic: entry["phonetic"].as_str().unwrap_or_default().to_string(),
        meanings: Vec::new(),
    };
    for m in entry["meanings"].as_array().into_iter().flatten() {
        let pos = m["partOfSpeech"].as_str().unwrap_or_default().to_string();
        let defs = m["definitions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["definition"].as_str().map(str::to_string))
            .take(4)
            .collect();
        def.meanings.push((pos, defs));
    }
    Ok(def)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines_and_urls() {
        assert_eq!(split_exec("firefox %u"), vec!["firefox"]);
        assert_eq!(
            split_exec(r#"sh -c "echo \"hi\" 100%%" %F"#),
            vec!["sh", "-c", "echo \"hi\" 100%"]
        );
        assert_eq!(split_exec("'my app' --flag"), vec!["my app", "--flag"]);
        assert_eq!(percent_encode("a b&c", b""), "a%20b%26c");
        assert_eq!(web_url("1+1"), "https://duckduckgo.com/?q=1%2B1");
    }
}
