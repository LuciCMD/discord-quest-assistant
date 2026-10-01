//! The main window: find a game in Discord's list, choose how it runs, start it, and watch it.

use std::fs;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, CentralPanel, Frame, Layout, Margin, Panel, RichText, ScrollArea, Ui,
    ViewportCommand,
};

use super::{fonts, theme, widgets};
use crate::catalog::{Catalog, Game};
use crate::journal::{self, Journal, Undo};
use crate::paths::{Paths, now_unix};
use crate::safe_path::ExePath;
use crate::session::{self, MAX_MINUTES, Me, Running, Target};
use crate::steam::{self, GameFolder, LaunchOption};

/// Most search results listed.
const MAX_RESULTS: usize = 60;
/// How often a stand-in that is still locked is tried again.
const RETRY_EVERY: Duration = Duration::from_secs(2);
const DEFAULT_MINUTES: f64 = 16.0;

#[derive(Clone, Copy)]
enum Tone {
    Good,
    Warn,
    Bad,
}

struct Status {
    tone: Tone,
    text: String,
}

impl Status {
    fn good(text: impl Into<String>) -> Self {
        Self {
            tone: Tone::Good,
            text: text.into(),
        }
    }
    fn warn(text: impl Into<String>) -> Self {
        Self {
            tone: Tone::Warn,
            text: text.into(),
        }
    }
    fn bad(text: impl Into<String>) -> Self {
        Self {
            tone: Tone::Bad,
            text: text.into(),
        }
    }
}

/// What Steam says about the picked game.
struct SteamPick {
    folder: GameFolder,
    options: Vec<LaunchOption>,
    /// Why there are no launch options, if there aren't.
    note: Option<String>,
}

/// How the picked game will run.
#[derive(Default)]
struct Pick {
    exe: usize,
    steam_mode: bool,
    steam: Option<Result<SteamPick, String>>,
    /// An index into the launch options; one past the end means the custom path.
    launch: usize,
    custom: String,
}

pub struct QuestApp {
    paths: Paths,
    me: Result<Me, String>,
    /// Held while the window is open, so a second copy can't manage the same journal.
    _lock: Option<fs::File>,
    blocked: Option<String>,
    catalog: Catalog,
    download: Option<Receiver<Result<Catalog, String>>>,
    query: String,
    results: Vec<usize>,
    selected: Option<usize>,
    pick: Pick,
    minutes: f64,
    journal: Journal,
    running: Vec<Running>,
    /// Journal entries whose copy was still locked; tried again every few seconds.
    leftovers: Vec<String>,
    last_retry: Instant,
    status: Status,
    confirm_quit: bool,
    quitting: bool,
    focused_search: bool,
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Discord Quest Assistant")
            .with_inner_size([760.0, 720.0])
            .with_min_inner_size([520.0, 480.0]),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Discord Quest Assistant",
        options,
        Box::new(|cc| {
            fonts::install(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(QuestApp::new()))
        }),
    )
}

impl QuestApp {
    fn new() -> Self {
        let paths = Paths::resolve();
        let (paths, blocked) = match paths {
            Ok(paths) => (paths, None),
            Err(e) => {
                let fallback = std::env::temp_dir().join("Discord Quest Assistant");
                let paths = Paths {
                    runs: fallback.join("runs"),
                    root: fallback,
                };
                (
                    paths,
                    Some(format!("The app's data folder couldn't be made: {e}.")),
                )
            }
        };
        let lock = take_lock(&paths);
        let blocked = blocked.or_else(|| {
            lock.is_none()
                .then(|| "Discord Quest Assistant is already open in another window.".to_owned())
        });
        let mut app = Self {
            me: whoami(),
            _lock: lock,
            catalog: Catalog::default(),
            download: None,
            query: String::new(),
            results: Vec::new(),
            selected: None,
            pick: Pick::default(),
            minutes: DEFAULT_MINUTES,
            journal: Journal::default(),
            running: Vec::new(),
            leftovers: Vec::new(),
            last_retry: Instant::now(),
            status: Status::good("Ready."),
            confirm_quit: false,
            quitting: false,
            focused_search: false,
            blocked,
            paths,
        };
        if app.blocked.is_none() {
            app.recover();
            app.load_catalog();
        }
        app
    }

    /// Undoes whatever an earlier run left behind, from the journal.
    fn recover(&mut self) {
        let (journal, note) = Journal::load(&self.paths.journal());
        self.journal = journal;
        if let Some(note) = note {
            self.status = Status::bad(note);
        }
        let ids: Vec<String> = self
            .journal
            .entries()
            .iter()
            .map(|e| e.id.clone())
            .collect();
        for id in ids {
            self.finish_entry(&id);
        }
        self.sweep_runs();
    }

    /// Removes session folders under `runs` that no journal entry names, such as ones left by a
    /// journal that was set aside.
    fn sweep_runs(&self) {
        let Ok(dir) = fs::read_dir(&self.paths.runs) else {
            return;
        };
        let known: Vec<&str> = self
            .journal
            .entries()
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if journal::is_session_id(&name) && !known.contains(&name.as_str()) {
                let changes = journal::Changes::Sandbox { dir: entry.path() };
                // A folder still in use stays until the next start.
                let _ = journal::undo(&changes, &self.paths.runs);
            }
        }
    }

    fn load_catalog(&mut self) {
        match Catalog::load(&self.paths.catalog()) {
            Some(catalog) => {
                let stale = catalog.is_stale();
                self.catalog = catalog;
                if stale {
                    self.refresh_catalog();
                }
            }
            None => self.refresh_catalog(),
        }
    }

    fn refresh_catalog(&mut self) {
        if self.download.is_some() {
            return;
        }
        let (send, receive) = mpsc::channel();
        let target = self.paths.catalog();
        std::thread::spawn(move || {
            let result = Catalog::download().map_err(|e| e.to_string());
            let result = result.and_then(|catalog| {
                catalog
                    .save(&target)
                    .map_err(|e| format!("The game list couldn't be saved: {e}"))?;
                Ok(catalog)
            });
            // The window may have closed; then nobody needs the answer.
            let _ = send.send(result);
        });
        self.download = Some(receive);
    }

    fn poll_download(&mut self) {
        let Some(receive) = &self.download else {
            return;
        };
        match receive.try_recv() {
            Ok(Ok(catalog)) => {
                self.status = Status::good(format!(
                    "Discord's game list is up to date: {} games.",
                    thousands(catalog.games.len())
                ));
                self.catalog = catalog;
                self.selected = None;
                self.results = self.catalog.search(&self.query, MAX_RESULTS);
                self.download = None;
            }
            Ok(Err(e)) => {
                self.status = Status::warn(format!("{e}."));
                self.download = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.download = None,
        }
    }

    /// Checks running copies, and cleans up after the ones that have exited.
    fn tick(&mut self) {
        let mut done: Vec<(String, String)> = Vec::new();
        for run in &mut self.running {
            if run.finished() {
                let how = if run.remaining() > Duration::from_secs(5) {
                    format!(
                        "{} was stopped with {} left",
                        run.game,
                        super::play::clock(run.remaining())
                    )
                } else {
                    format!("{} has run its full time", run.game)
                };
                done.push((run.id.clone(), how));
            }
        }
        self.running
            .retain(|r| !done.iter().any(|(id, _)| *id == r.id));
        for (id, how) in done {
            if self.finish_entry(&id) {
                self.status = Status::good(format!("{how}, and its files are cleaned up."));
            }
        }
        if !self.leftovers.is_empty() && self.last_retry.elapsed() >= RETRY_EVERY {
            self.last_retry = Instant::now();
            for id in std::mem::take(&mut self.leftovers) {
                self.finish_entry(&id);
            }
        }
    }

    /// Undoes one journal entry. True if everything was put back.
    fn finish_entry(&mut self, id: &str) -> bool {
        let Some(entry) = self.journal.entries().iter().find(|e| e.id == id).cloned() else {
            return true;
        };
        match journal::undo(&entry.changes, &self.paths.runs) {
            Undo::Done => {
                if let Err(e) = self.journal.remove(id) {
                    self.status =
                        Status::warn(format!("The session journal couldn't be updated: {e}."));
                }
                true
            }
            Undo::Busy => {
                if !self.leftovers.iter().any(|l| l == id) {
                    self.leftovers.push(id.to_owned());
                }
                false
            }
            Undo::Kept(why) => {
                self.status = Status::warn(format!("{}: {why}.", entry.game));
                // Trying again won't change anything, so the entry goes.
                let _ = self.journal.remove(id);
                false
            }
        }
    }

    fn stop_all(&mut self) {
        for run in &mut self.running {
            run.stop();
        }
        let ids: Vec<String> = self.running.drain(..).map(|r| r.id).collect();
        for id in ids {
            // Windows can hold an exited exe briefly; give it a moment before giving up.
            for _ in 0..20 {
                self.leftovers.retain(|l| *l != id);
                // Done, or kept as it is: either way there's nothing to wait for.
                if self.finish_entry(&id) || !self.leftovers.contains(&id) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    fn selected_game(&self) -> Option<&Game> {
        self.selected.and_then(|i| self.catalog.games.get(i))
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        let steam_only = self
            .catalog
            .games
            .get(index)
            .is_some_and(|g| g.exes.is_empty());
        self.pick = Pick {
            steam_mode: steam_only,
            ..Pick::default()
        };
    }

    /// Reads what Steam knows about the game, once per pick or when asked again.
    fn scan_steam(&mut self) {
        let Some(game) = self.selected_game() else {
            return;
        };
        let ids = game.steam.clone();
        let result = steam::libraries().and_then(|libraries| {
            let folder = steam::find_game(&libraries, &ids)?;
            let (options, note) = match steam::launch_options(&libraries, folder.app_id) {
                Ok(options) => (options, None),
                Err(e) => (Vec::new(), Some(format!("{e}."))),
            };
            Ok(SteamPick {
                folder,
                options,
                note,
            })
        });
        self.pick.steam = Some(result.map_err(|e| format!("{e}.")));
        self.pick.launch = 0;
    }

    /// Where the stand-in would go, or why it can't start yet.
    fn target(&self) -> Result<Target, String> {
        let game = self.selected_game().ok_or("Pick a game first.")?;
        if !self.pick.steam_mode {
            let raw = game
                .exes
                .get(self.pick.exe)
                .ok_or("Discord lists no executable for this game.")?;
            return ExePath::parse(raw)
                .map(Target::Sandbox)
                .map_err(|e| format!("Unusable path: {e}."));
        }
        let pick = match &self.pick.steam {
            Some(Ok(pick)) => pick,
            Some(Err(e)) => return Err(e.clone()),
            None => return Err("Checking Steam…".into()),
        };
        let exe = match pick.options.get(self.pick.launch) {
            Some(option) => option.exe.clone(),
            None => ExePath::parse(&self.pick.custom).map_err(|e| format!("Custom path: {e}."))?,
        };
        if pick.folder.installed && exe.under(&pick.folder.folder).exists() {
            return Err("That file is part of the installed game, so it won't be replaced.".into());
        }
        Ok(Target::Steam {
            folder: pick.folder.clone(),
            exe,
        })
    }

    fn start(&mut self) {
        let (Ok(target), Some(game)) =
            (self.target(), self.selected_game().map(|g| g.name.clone()))
        else {
            return;
        };
        let me = match &self.me {
            Ok(me) => me,
            Err(e) => {
                self.status = Status::bad(e.clone());
                return;
            }
        };
        // The field keeps the value within 1-240, so the cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let minutes = (self.minutes.round().max(1.0) as u64).min(MAX_MINUTES);
        match session::start(&game, minutes, &target, me, &self.paths, &mut self.journal) {
            Ok(run) => {
                self.status = Status::good(format!("{game} is running for {minutes} min."));
                self.running.push(run);
            }
            Err(e) => self.status = Status::bad(format!("{e}.")),
        }
    }
}

/// A count with thousands separators: 19,266.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// This exe's path and fingerprint, used to make copies and to know them again later.
fn whoami() -> Result<Me, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("Couldn't find this app's own exe: {e}."))?;
    let sha256 = journal::file_sha256(&exe)
        .map_err(|e| format!("Couldn't read this app's own exe: {e}."))?;
    Ok(Me { exe, sha256 })
}

fn take_lock(paths: &Paths) -> Option<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock())
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

impl eframe::App for QuestApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_close(&ctx);
        if self.blocked.is_none() {
            self.poll_download();
            self.tick();
        }
        self.status_strip(ui);
        let frame = Frame::new().fill(theme::GROUND).inner_margin(Margin {
            left: 28,
            right: 24,
            top: 24,
            bottom: 24,
        });
        CentralPanel::default().frame(frame).show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let size = egui::vec2(theme::CONTENT_MAX_WIDTH.min(ui.available_width()), 0.0);
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| self.page(ui));
            });
        });
        self.quit_dialog(&ctx);
        if !self.running.is_empty() || !self.leftovers.is_empty() || self.download.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_all();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::GROUND.to_normalized_gamma_f32()
    }
}

impl QuestApp {
    fn handle_close(&mut self, ctx: &egui::Context) {
        let asked = ctx.input(|i| i.viewport().close_requested());
        if asked && !self.quitting && !self.running.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_quit = true;
        }
    }

    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_quit {
            return;
        }
        let frame = Frame::popup(&ctx.global_style())
            .inner_margin(Margin::symmetric(18, 16))
            .corner_radius(theme::RADIUS_CARD);
        let modal = egui::Modal::new(egui::Id::new("confirm-quit"))
            .frame(frame)
            .show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(
                RichText::new("Stop and Quit?")
                    .text_style(theme::Text::CardHeading.style())
                    .color(theme::INK),
            );
            ui.add_space(4.0);
            ui.label("The running games stop and their files are cleaned up. Quest progress so far stays.");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, "Keep Running").clicked() {
                    self.confirm_quit = false;
                }
                if ui.button("Stop and Quit").clicked() {
                    self.confirm_quit = false;
                    self.quitting = true;
                    self.stop_all();
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
        if modal.should_close() {
            self.confirm_quit = false;
        }
    }

    fn status_strip(&self, ui: &mut Ui) {
        let frame = Frame::new()
            .fill(theme::GROUND)
            .inner_margin(Margin::symmetric(20, 10));
        Panel::bottom("status").frame(frame).show(ui, |ui| {
            let edge = ui.max_rect().expand2(egui::vec2(20.0, 10.0));
            ui.painter().hline(
                edge.x_range(),
                edge.top(),
                egui::Stroke::new(1.0, theme::LINE_SOFT),
            );
            ui.horizontal(|ui| {
                let color = match self.status.tone {
                    Tone::Good => theme::GOOD,
                    Tone::Warn => theme::WARN,
                    Tone::Bad => theme::BAD,
                };
                widgets::status_dot(ui, color);
                ui.add(egui::Label::new(RichText::new(&self.status.text).color(theme::INK)).wrap());
            });
        });
    }

    fn page(&mut self, ui: &mut Ui) {
        if let Some(why) = &self.blocked {
            widgets::card(ui, "Can't Start", Some(why), |_| {});
            return;
        }
        self.running_card(ui);
        self.search_card(ui);
        if self.selected.is_some() {
            self.settings_card(ui);
        }
    }

    fn search_card(&mut self, ui: &mut Ui) {
        let list_note = self.list_note();
        widgets::card(ui, "Find a Game", Some(&list_note), |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 130.0).max(160.0);
                let field = widgets::filled_field(ui, &mut self.query, "Game name", width);
                if !self.focused_search {
                    field.request_focus();
                    self.focused_search = true;
                }
                if field.changed() {
                    self.results = self.catalog.search(&self.query, MAX_RESULTS);
                }
                let label = if self.download.is_some() { "Updating…" } else { "Refresh List" };
                let refresh = ui.add_enabled(self.download.is_none(), egui::Button::new(label));
                if refresh.clicked() {
                    self.refresh_catalog();
                }
                widgets::info(
                    ui,
                    "Downloads Discord's list of games it can detect again. It updates by itself once a day.",
                );
            });
            ui.add_space(4.0);
            self.results_list(ui);
        });
    }

    fn list_note(&self) -> String {
        let count = self.catalog.games.len();
        if count == 0 {
            return if self.download.is_some() {
                "Downloading Discord's list of games…".into()
            } else {
                "Discord's game list isn't loaded. Refresh it to search.".into()
            };
        }
        let hours = now_unix().saturating_sub(self.catalog.fetched_unix) / 3600;
        let age = match hours {
            0 => "less than an hour ago".to_owned(),
            1 => "an hour ago".to_owned(),
            h if h < 48 => format!("{h} hours ago"),
            h => format!("{} days ago", h / 24),
        };
        format!(
            "Searches the {} games Discord detects. List updated {age}.",
            thousands(count)
        )
    }

    fn results_list(&mut self, ui: &mut Ui) {
        if self.query.trim().is_empty() {
            widgets::hint(ui, "Type a game's name to search Discord's list.");
            return;
        }
        if self.results.is_empty() {
            widgets::hint(ui, "No game in Discord's list matches that.");
            return;
        }
        let mut clicked = None;
        #[allow(clippy::cast_precision_loss)]
        let height = self.results.len().min(7) as f32 * 32.0;
        ScrollArea::vertical()
            .id_salt("results")
            .min_scrolled_height(height)
            .max_height(height)
            .show(ui, |ui| {
                for &index in &self.results {
                    let Some(game) = self.catalog.games.get(index) else {
                        continue;
                    };
                    let detail = game.exes.first().map_or("Steam", String::as_str);
                    let selected = self.selected == Some(index);
                    if widgets::choice_row(ui, selected, &game.name, detail).clicked() {
                        clicked = Some(index);
                    }
                }
            });
        if let Some(index) = clicked {
            self.select(index);
        }
    }

    fn settings_card(&mut self, ui: &mut Ui) {
        let Some(game) = self.selected_game().cloned() else {
            return;
        };
        widgets::card(ui, &game.name, None, |ui| {
            if !game.exes.is_empty() {
                self.exe_choice(ui, &game);
            }
            if !game.steam.is_empty() {
                self.steam_choice(ui, &game);
            }
            ui.add_space(6.0);
            widgets::label_info(
                ui,
                "Minutes",
                "How long the game runs. Most quests ask for 15 minutes; the extra minute covers the time Discord takes to notice it.",
            );
            widgets::number_field(ui, &mut self.minutes, 1.0..=240.0, 0.2, 0, "min", 90.0);
            ui.add_space(10.0);
            let target = self.target();
            ui.horizontal(|ui| {
                let can_start = target.is_ok() && self.me.is_ok();
                let start = ui
                    .add_enabled_ui(can_start, |ui| widgets::primary_button(ui, "Start"))
                    .inner;
                if start.clicked() {
                    self.start();
                }
                // A Steam problem is already shown above, in the warning colour.
                let steam_problem = self.pick.steam_mode && matches!(self.pick.steam, Some(Err(_)));
                if let (Err(why), false) = (&target, steam_problem) {
                    widgets::hint(ui, why);
                } else if let Err(why) = &self.me {
                    widgets::hint(ui, why);
                }
            });
        });
    }

    fn exe_choice(&mut self, ui: &mut Ui, game: &Game) {
        widgets::label_info(
            ui,
            "Executable",
            "Discord knows the game by this file and the folders above it. The first usually works; try another if the quest doesn't move.",
        );
        let enabled = !self.pick.steam_mode;
        ui.add_enabled_ui(enabled, |ui| {
            for (i, exe) in game.exes.iter().enumerate().take(12) {
                if widgets::choice_row(ui, self.pick.exe == i, exe, "").clicked() {
                    self.pick.exe = i;
                }
            }
        });
        ui.add_space(6.0);
    }

    fn steam_choice(&mut self, ui: &mut Ui, game: &Game) {
        let steam_only = game.exes.is_empty();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!steam_only, |ui| {
                widgets::toggle(ui, &mut self.pick.steam_mode, "Run From Steam Library");
            });
            widgets::info(
                ui,
                "For games Discord finds through Steam, such as Marathon. Start the game's download in Steam and pause it at 1-2% first, so Steam makes its folder. A real install is never touched.",
            );
        });
        if steam_only {
            widgets::hint(ui, "Discord finds this game only through Steam.");
        }
        if !self.pick.steam_mode {
            return;
        }
        if self.pick.steam.is_none() {
            self.scan_steam();
        }
        ui.add_space(4.0);
        let mut rescan = false;
        match &self.pick.steam {
            Some(Ok(pick)) => {
                widgets::value_well(ui, &pick.folder.folder.display().to_string());
                if pick.folder.installed {
                    ui.label(
                        RichText::new("This game is installed. Play it from Steam instead; its own files are never replaced.")
                            .color(theme::WARN),
                    );
                }
                let options = pick.options.clone();
                let note = pick.note.clone();
                self.launch_choice(ui, &options, note.as_deref());
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(theme::WARN));
            }
            None => {}
        }
        if ui.button("Check Steam Again").clicked() {
            rescan = true;
        }
        if rescan {
            self.scan_steam();
        }
    }

    fn launch_choice(&mut self, ui: &mut Ui, options: &[LaunchOption], note: Option<&str>) {
        ui.add_space(4.0);
        widgets::label_info(
            ui,
            "Launch Option",
            "Steam's ways of starting the game. Some point at a launcher Discord doesn't watch; if the quest doesn't move, try another or enter the game's own exe.",
        );
        if let Some(note) = note {
            widgets::hint(ui, note);
        }
        for (i, option) in options.iter().enumerate().take(12) {
            let exe = option.exe.to_string();
            if widgets::choice_row(ui, self.pick.launch == i, &exe, &option.label).clicked() {
                self.pick.launch = i;
            }
        }
        let custom = options.len();
        if widgets::choice_row(ui, self.pick.launch >= custom, "Custom Path", "").clicked() {
            self.pick.launch = custom;
        }
        if self.pick.launch >= custom {
            ui.horizontal(|ui| {
                widgets::filled_field(ui, &mut self.pick.custom, "bin/win64/game.exe", 320.0);
                widgets::info(
                    ui,
                    "The exe's path inside the game's folder, as on SteamDB or r/DiscordQuests.",
                );
            });
        }
        ui.add_space(6.0);
    }

    fn running_card(&mut self, ui: &mut Ui) {
        if self.running.is_empty() && self.leftovers.is_empty() {
            return;
        }
        let mut stop = None;
        widgets::card(ui, "Running", None, |ui| {
            for run in &self.running {
                ui.horizontal(|ui| {
                    widgets::status_dot(ui, theme::GOOD);
                    ui.label(RichText::new(&run.game).color(theme::INK));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Stop").clicked() {
                            stop = Some(run.id.clone());
                        }
                        let left = super::play::clock(run.remaining());
                        ui.label(
                            RichText::new(format!("{left} left"))
                                .monospace()
                                .color(theme::MUTED),
                        );
                    });
                });
                let fraction = run.elapsed().as_secs_f32() / run.length.as_secs_f32().max(1.0);
                widgets::progress_bar(ui, fraction);
                widgets::hint(ui, &run.path.display().to_string());
                ui.add_space(6.0);
            }
            for id in &self.leftovers {
                let game = self
                    .journal
                    .entries()
                    .iter()
                    .find(|e| e.id == *id)
                    .map_or("A game", |e| e.game.as_str());
                ui.horizontal(|ui| {
                    widgets::status_dot(ui, theme::WARN);
                    ui.label(
                        RichText::new(format!("{game} is still open from before. It's cleaned up once its window closes."))
                            .color(theme::MUTED),
                    );
                });
            }
        });
        if let Some(id) = stop
            && let Some(run) = self.running.iter_mut().find(|r| r.id == id)
        {
            run.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_with_separators() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(19_266), "19,266");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
