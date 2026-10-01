//! Play mode: what a copy runs as. A small window with the time left and a Stop button; it closes
//! itself when the time is up. It touches no files and makes no requests; the main window cleans
//! up after it.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, CentralPanel, Frame, Layout, Margin, RichText, Ui, ViewportCommand,
};

use super::{fonts, theme, widgets};
use crate::session::{MAX_MINUTES, PLAY_ARG};

/// Longest game name shown.
const MAX_TITLE: usize = 200;

pub struct PlayArgs {
    seconds: u64,
    title: String,
}

impl PlayArgs {
    /// Play mode is `--dqa-play <seconds> <game name>`, exactly. Anything else is the main app.
    pub fn parse(args: &[String]) -> Option<Self> {
        let [flag, seconds, title] = args else {
            return None;
        };
        let seconds: u64 = seconds.parse().ok()?;
        let valid = flag == PLAY_ARG
            && (1..=MAX_MINUTES * 60).contains(&seconds)
            && !title.trim().is_empty()
            && title.chars().count() <= MAX_TITLE;
        valid.then(|| Self {
            seconds,
            title: title.trim().to_owned(),
        })
    }
}

pub fn run(args: PlayArgs) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&args.title)
            .with_inner_size([440.0, 200.0])
            .with_resizable(false)
            .with_maximize_button(false),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    let title = args.title.clone();
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(PlayApp {
                title: args.title,
                length: Duration::from_secs(args.seconds),
                started: Instant::now(),
            }))
        }),
    )
}

struct PlayApp {
    title: String,
    length: Duration,
    started: Instant,
}

impl eframe::App for PlayApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let elapsed = self.started.elapsed();
        if elapsed >= self.length {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        let frame = Frame::new()
            .fill(theme::GROUND)
            .inner_margin(Margin::symmetric(16, 16));
        CentralPanel::default().frame(frame).show(ui, |ui| {
            widgets::card(
                ui,
                &self.title,
                Some("Discord sees this window as the game. Keep it open until the time is up."),
                |ui| self.body(ui, elapsed),
            );
        });
        ui.ctx().request_repaint_after(Duration::from_millis(500));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::GROUND.to_normalized_gamma_f32()
    }
}

impl PlayApp {
    fn body(&self, ui: &mut Ui, elapsed: Duration) {
        let fraction = elapsed.as_secs_f32() / self.length.as_secs_f32().max(1.0);
        widgets::progress_bar(ui, fraction);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let left = self.length.saturating_sub(elapsed);
            ui.label(
                RichText::new(format!("{} left", clock(left)))
                    .monospace()
                    .color(theme::INK),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Stop").clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
    }
}

/// `m:ss`, or `h:mm:ss` from an hour up.
pub fn clock(time: Duration) -> String {
    let total = time.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn only_exact_play_arguments_start_play_mode() {
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "960", "Marathon"])).is_some());
        assert!(PlayArgs::parse(&args(&[])).is_none());
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "960"])).is_none());
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "0", "Game"])).is_none());
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "-5", "Game"])).is_none());
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "99999999", "Game"])).is_none());
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "60", "  "])).is_none());
        assert!(PlayArgs::parse(&args(&["--other", "60", "Game"])).is_none());
        assert!(PlayArgs::parse(&args(&[PLAY_ARG, "60", "Game", "extra"])).is_none());
    }

    #[test]
    fn clock_reads_naturally() {
        assert_eq!(clock(Duration::from_secs(59)), "0:59");
        assert_eq!(clock(Duration::from_secs(16 * 60)), "16:00");
        assert_eq!(clock(Duration::from_secs(3600 + 61)), "1:01:01");
    }
}
