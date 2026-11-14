// SPDX-License-Identifier: GPL-3.0-or-later
//! Glint: a keyboard-driven search launcher for the Linux desktop.
//!
//! Finds applications, settings panels and files, does maths and unit or
//! currency conversion, looks up definitions and hands anything else to a
//! web search, with a preview of the selected result.

mod calc;
mod config;
mod draw;
mod icons;
mod index;
#[cfg(target_os = "linux")]
mod ipc;
mod open;
mod settings_page;
mod ui;
#[cfg(target_os = "linux")]
mod x11focus;

fn main() -> eframe::Result {
    let args = ui::Args::parse();

    #[cfg(target_os = "linux")]
    {
        // --no-daemon and --screenshot skip daemon logic entirely.
        if !args.no_daemon
            && let Some(sock) = ipc::socket_path()
        {
            // CLIENT PATH (hot path): try to reach a running instance.
            // This runs before ANY GUI/config/index work.
            let msg = if args.open_settings {
                "settings".to_string()
            } else if args.quit {
                "quit".to_string()
            } else if !args.query.is_empty() {
                format!("query {}", args.query)
            } else {
                "show".to_string()
            };

            if !args.daemon {
                if ipc::send_cmd(&sock, &msg).is_ok() {
                    // Instance handled it; exit immediately.
                    return Ok(());
                }
                // No instance reachable: become the instance below.
                if args.quit {
                    // Nothing to quit.
                    return Ok(());
                }
            }

            // --quit with --daemon: send quit and exit.
            if args.daemon && args.quit {
                let _ = ipc::send_cmd(&sock, "quit");
                return Ok(());
            }

            // DAEMON PATH: become the instance.
            match ipc::bind_listener(&sock) {
                Ok(listener) => {
                    return ui::run_daemon(args, listener, sock);
                }
                Err(e) => {
                    if e.contains("already running") {
                        if args.daemon {
                            // --daemon and an instance already runs: exit 0.
                            return Ok(());
                        }
                        // Another instance won the race: retry send_cmd once.
                        if ipc::send_cmd(&sock, &msg).is_ok() {
                            return Ok(());
                        }
                        eprintln!("glint: instance exists but unreachable: {e}");
                    } else {
                        eprintln!("glint: IPC setup failed: {e}");
                    }
                }
            }
        }
        // XDG_RUNTIME_DIR unset or IPC failed: fall back to one-shot.
    }

    // One-shot mode (--no-daemon, --screenshot, or fallback).
    ui::run_oneshot(args)
}
