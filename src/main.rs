// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
mod hardening;
mod journal;
mod paths;
mod safe_path;
mod session;
mod steam;
mod ui;

fn main() -> eframe::Result {
    // It can't fail on supported Windows; if it did, the import flag from build.rs still covers
    // the exe's own DLLs.
    let _ = hardening::system32_dlls_only();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ui::play::PlayArgs::parse(&args) {
        Some(play) => ui::play::run(play),
        None => ui::app::run(),
    }
}
