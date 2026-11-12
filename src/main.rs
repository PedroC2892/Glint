//! Lookout: a keyboard-driven search launcher for the Linux desktop.
//!
//! Finds applications, settings panels and files, does maths and unit or
//! currency conversion, looks up definitions and hands anything else to a
//! web search, with a preview of the selected result.

mod calc;
mod draw;
mod icons;
mod index;
mod open;
mod ui;
#[cfg(target_os = "linux")]
mod x11focus;

fn main() -> eframe::Result {
    ui::run()
}
