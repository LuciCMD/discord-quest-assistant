// No console window behind the app in release builds
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
    // Can't fail on supported Windows, and the import flag in build.rs still covers the exe's DLLs
    let _ = hardening::system32_dlls_only();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ui::play::PlayArgs::parse(&args) {
        Some(play) => ui::play::run(play),
        None => ui::app::run(),
    }
}
