// SPDX-License-Identifier: GPL-3.0-or-later
//! Getting the keyboard on X11.
//!
//! Window managers with focus-stealing prevention (KWin, for one) ignore a
//! window that asks for focus itself while the person works in another app,
//! so typing kept going to that other app. Requests marked as coming from a
//! pager/taskbar (source 2) are always honoured, so the launcher asks that way,
//! and also asks to stay above other windows like a popover.

use std::sync::Mutex;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::properties::WmHints;
use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask, InputFocus};
use x11rb::rust_connection::RustConnection;

static X11_CONN: Mutex<Option<RustConnection>> = Mutex::new(None);

fn with_conn<F>(mut f: F)
where
    F: FnMut(&RustConnection) -> Result<(), Box<dyn std::error::Error>>,
{
    let mut lock = match X11_CONN.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if lock.is_none()
        && let Ok((conn, _)) = x11rb::connect(None)
    {
        *lock = Some(conn);
    }
    if let Some(conn) = lock.as_ref() {
        if f(conn).is_ok() {
            return;
        }
        // If operation failed (e.g. stale connection), reconnect once.
        if let Ok((new_conn, _)) = x11rb::connect(None) {
            let _ = f(&new_conn);
            *lock = Some(new_conn);
        }
    }
}

/// The X11 window id behind the app window, if it runs on X11.
pub fn window_id(cc: &eframe::CreationContext) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

/// Keep asking for the keyboard from a background thread until the window has
/// it (or about 3 seconds pass). It can't run from the drawing loop: a window
/// that opens behind another one isn't drawn, so the loop would never run.
pub fn activate_in_background(window: u32) {
    std::thread::spawn(move || {
        for _ in 0..30 {
            if let Err(e) = activate(window) {
                eprintln!("could not ask for keyboard focus: {e}");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            if has_focus(window).unwrap_or(false) {
                return;
            }
        }
    });
}

fn has_focus(window: u32) -> Result<bool, Box<dyn std::error::Error>> {
    let (conn, _) = x11rb::connect(None)?;
    Ok(conn.get_input_focus()?.reply()?.focus == window)
}

/// Raise the window above others and give it keyboard focus.
fn activate(window: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
    };
    let (active, state, above) = (
        atom("_NET_ACTIVE_WINDOW")?,
        atom("_NET_WM_STATE")?,
        atom("_NET_WM_STATE_ABOVE")?,
    );

    // Tell the window manager this window takes keyboard input.
    let hints = WmHints {
        input: Some(true),
        ..WmHints::new()
    };
    hints.set(&conn, window)?;

    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    // _NET_WM_STATE: add (1) ABOVE, source 2 (pager).
    let keep_above = ClientMessageEvent::new(32, window, state, [1, above, 0, 2, 0]);
    conn.send_event(false, root, mask, keep_above)?;
    // _NET_ACTIVE_WINDOW: source 2 (pager), current time.
    let activate = ClientMessageEvent::new(32, window, active, [2, CURRENT_TIME, 0, 0, 0]);
    conn.send_event(false, root, mask, activate)?;
    // Direct focus too, for window managers without these hints. Errors are
    // ignored: the window may not be viewable yet, the caller retries.
    conn.set_input_focus(InputFocus::PARENT, window, CURRENT_TIME)?;
    conn.flush()?;
    Ok(())
}

/// Unmap (hide) the window directly via X11.
///
/// eframe's `ViewportCommand::Visible(false)` is only processed during
/// `App::update`, which may never be called while the window is hidden or the
/// event loop is idling. Calling `UnmapWindow` directly through x11rb
/// guarantees the window disappears immediately.
pub fn unmap_window(window: u32) {
    with_conn(|conn| {
        conn.unmap_window(window)?;
        conn.flush()?;
        Ok(())
    });
}

/// Map the window and activate it (raise + focus) for a daemon show.
///
/// Like `unmap_window`, this bypasses the eframe event loop so the window
/// becomes visible immediately even if `App::update` hasn't run yet.
/// After mapping, `activate_in_background` drives the focus retry loop.
pub fn map_and_activate(window: u32) {
    with_conn(|conn| {
        conn.map_window(window)?;
        conn.flush()?;
        Ok(())
    });
    activate_in_background(window);
}
