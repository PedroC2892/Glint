//! Single-instance IPC over a Unix domain socket.
//!
//! Protocol: one UTF-8 line per connection, at most 4 KiB:
//!   `show`, `settings`, `quit`, `query <text>`
//! The listener runs in its own thread and never blocks the UI.

use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Maximum bytes read from one connection.
const MAX_MSG: usize = 4096;
/// Read timeout per connection.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Commands the listener can receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Settings,
    Quit,
    Query(String),
}

/// Strip C0/C1 control characters (except space) from text received over IPC.
fn strip_control(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Parse a single IPC message line.
pub fn parse_msg(line: &str) -> Option<Cmd> {
    let line = line.trim();
    if line.is_empty() || line.len() > MAX_MSG {
        return None;
    }
    if line == "show" {
        Some(Cmd::Show)
    } else if line == "settings" {
        Some(Cmd::Settings)
    } else if line == "quit" {
        Some(Cmd::Quit)
    } else if let Some(rest) = line.strip_prefix("query ") {
        let text = strip_control(rest);
        if text.is_empty() {
            None
        } else {
            Some(Cmd::Query(text))
        }
    } else {
        None
    }
}

/// Return the socket directory and socket path, or `None` when
/// `XDG_RUNTIME_DIR` is unset or unsuitable.
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let runtime = PathBuf::from(runtime);
    if !runtime.is_absolute() {
        return None;
    }
    Some(runtime.join("glint").join("glint.sock"))
}

/// Validate that the socket directory exists, is owned by us, is mode 0700,
/// and is not a symlink. Creates it if missing.
fn ensure_socket_dir(sock: &Path) -> Result<(), String> {
    let dir = sock.parent().ok_or("socket path has no parent")?;
    let uid = libc::getuid();

    if dir.exists() {
        // Must not be a symlink.
        let meta = dir
            .symlink_metadata()
            .map_err(|e| format!("stat {}: {e}", dir.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "{} is a symlink; refusing to use it",
                dir.display()
            ));
        }
        if !meta.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        // Check ownership.
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != uid {
            return Err(format!("{} is not owned by uid {uid}", dir.display()));
        }
        // Enforce mode 0700.
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("chmod {}: {e}", dir.display()))?;
        }
    } else {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// Try to connect to a running instance and send a command.
/// Returns `Ok(())` on success, `Err` if no instance is reachable.
pub fn send_cmd(sock: &Path, cmd: &str) -> Result<(), std::io::Error> {
    let mut stream = UnixStream::connect(sock)?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

/// Bind the listener socket (handling stale sockets and races).
pub fn bind_listener(sock: &Path) -> Result<UnixListener, String> {
    ensure_socket_dir(sock)?;

    // Remove stale socket if connect fails.
    if sock.exists() {
        match UnixStream::connect(sock) {
            Ok(_) => return Err("another instance is already running".into()),
            Err(_) => {
                // Stale socket: remove it.
                let _ = std::fs::remove_file(sock);
            }
        }
    }

    match UnixListener::bind(sock) {
        Ok(listener) => {
            // Set socket file mode to 0600.
            let _ = std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o600));
            Ok(listener)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Race: another instance bound between our check and bind.
            // Retry connect once.
            if UnixStream::connect(sock).is_ok() {
                Err("another instance is already running".into())
            } else {
                Err(format!("bind {}: {e}", sock.display()))
            }
        }
        Err(e) => Err(format!("bind {}: {e}", sock.display())),
    }
}

/// Start the listener thread.
/// `on_cmd` is called from the listener thread for each valid command.
pub fn start_listener<F>(listener: UnixListener, on_cmd: F)
where
    F: Fn(Cmd) + Send + 'static,
{
    std::thread::Builder::new()
        .name("ipc-listener".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Some(cmd) = read_cmd(stream) {
                            on_cmd(cmd);
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .expect("failed to spawn IPC listener thread");
}

/// Read one command from a connection.
fn read_cmd(mut stream: UnixStream) -> Option<Cmd> {
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut buf = vec![0u8; MAX_MSG + 1];
    let mut total = 0;
    loop {
        match stream.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if total > MAX_MSG {
                    return None; // Oversized: drop.
                }
                // Stop at the first newline.
                if buf[..total].contains(&b'\n') {
                    break;
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => return None,
        }
    }
    let text = std::str::from_utf8(&buf[..total]).ok()?;
    let line = text.lines().next()?;
    parse_msg(line)
}

// We reference libc::getuid above; bring in the libc types via the raw FFI
// that Rust std already links. No extra crate needed.
mod libc {
    unsafe extern "C" {
        pub safe fn getuid() -> u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_messages() {
        assert_eq!(parse_msg("show"), Some(Cmd::Show));
        assert_eq!(parse_msg("settings"), Some(Cmd::Settings));
        assert_eq!(parse_msg("quit"), Some(Cmd::Quit));
        assert_eq!(
            parse_msg("query hello world"),
            Some(Cmd::Query("hello world".into()))
        );
        assert_eq!(parse_msg("  show  "), Some(Cmd::Show));
    }

    #[test]
    fn parse_rejects_bad_messages() {
        assert_eq!(parse_msg(""), None);
        assert_eq!(parse_msg("unknown"), None);
        assert_eq!(parse_msg("query "), None); // empty text after strip
        // Oversized message.
        let big = "query ".to_string() + &"a".repeat(MAX_MSG + 1);
        assert_eq!(parse_msg(&big), None);
    }

    #[test]
    fn control_chars_stripped() {
        assert_eq!(
            parse_msg("query hello\x00world\x1b[31m"),
            Some(Cmd::Query("helloworld[31m".into()))
        );
        // Tab and newline are control chars.
        assert_eq!(parse_msg("query a\tb"), Some(Cmd::Query("ab".into())));
    }

    #[test]
    fn strip_control_preserves_unicode() {
        assert_eq!(strip_control("café ☕"), "café ☕");
        assert_eq!(strip_control("a\x07b\x08c"), "abc");
    }

    #[test]
    fn socket_dir_validation_in_tempdir() {
        let tmp = std::env::temp_dir().join(format!(
            "glint-ipc-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sock = tmp.join("glint.sock");
        // Directory doesn't exist yet; ensure_socket_dir should create it.
        assert!(ensure_socket_dir(&sock).is_ok());
        let meta = tmp.metadata().unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stale_socket_recovery_in_tempdir() {
        let tmp = std::env::temp_dir().join(format!(
            "glint-ipc-stale-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sock = tmp.join("glint.sock");
        // Create a stale socket file (just a regular file, no listener).
        std::fs::write(&sock, "").unwrap();
        // bind_listener should remove the stale socket and bind successfully.
        let listener = bind_listener(&sock);
        assert!(
            listener.is_ok(),
            "bind_listener failed: {:?}",
            listener.err()
        );
        drop(listener);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn round_trip_send_recv() {
        let tmp = std::env::temp_dir().join(format!(
            "glint-ipc-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sock = tmp.join("glint.sock");

        let listener = bind_listener(&sock).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        start_listener(listener, move |cmd| {
            tx.send(cmd).ok();
        });

        // Send a command and check it's received.
        send_cmd(&sock, "show").unwrap();
        let cmd = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(cmd, Cmd::Show);

        send_cmd(&sock, "query test text").unwrap();
        let cmd = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(cmd, Cmd::Query("test text".into()));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
