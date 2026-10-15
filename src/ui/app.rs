//! The main window: find a game in Discord's list, choose how it runs, start it and watch it

use std::fs;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, CentralPanel, Frame, Layout, Margin, Panel, Rect, RichText, ScrollArea, Stroke,
    Ui, ViewportCommand, pos2,
};

use super::{fonts, icon, theme, widgets};
use crate::catalog::{Catalog, Game};
use crate::journal::{self, Journal, Undo};
use crate::paths::{Paths, now_unix};
use crate::safe_path::ExePath;
use crate::session::{self, MAX_MINUTES, Me, Running, Target};
use crate::steam::{self, GameFolder, LaunchOption};

const MAX_RESULTS: usize = 60;
/// How often a stand-in that is still locked is tried again
const RETRY_EVERY: Duration = Duration::from_secs(2);
const DEFAULT_MINUTES: f64 = 16.0;
/// Room a queue row keeps right of the name for "240 min" and Remove
const QUEUE_ROW_BUTTONS: f32 = 170.0;

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

/// What Steam says about the picked game
struct SteamPick {
    folder: GameFolder,
    options: Vec<LaunchOption>,
    /// Why there are no launch options, if there aren't
    note: Option<String>,
}

/// How the picked game will run
#[derive(Default)]
struct Pick {
    exe: usize,
    steam_mode: bool,
    steam: Option<Result<SteamPick, String>>,
    /// Index into the launch options, with one past the end meaning the custom path
    launch: usize,
    custom: String,
    /// The card as last loaded from or saved to the queue, so only a real change is written back
    saved: Option<CardKey>,
    /// A queued exe the card can't show, since Discord's list no longer has it
    unlisted: Option<String>,
}

/// Every choice on a game's card, to tell a change from a reload
#[derive(Clone, PartialEq)]
struct CardKey {
    steam_mode: bool,
    exe: usize,
    launch: usize,
    custom: String,
    minutes: u64,
}

/// What was clicked in the queue list this frame, acted on once the list is drawn
#[derive(Default)]
struct QueueClicks {
    remove: Option<String>,
    edit: Option<String>,
    start: bool,
    clear: bool,
}

/// A game that ran this session, for the pipeline strip
struct Done {
    game: String,
    played: Duration,
    /// Ran its whole time, rather than being stopped or closed early
    full: bool,
}

/// The strip keeps this many finished games, oldest dropped first
const MAX_DONE: usize = 64;

/// A game waiting its turn, with the settings it had when it was added
struct Queued {
    game: String,
    minutes: u64,
    target: Target,
}

// Four separate flags, not a state machine
#[allow(clippy::struct_excessive_bools)]
pub struct QuestApp {
    paths: Paths,
    me: Result<Me, String>,
    /// Held while the window is open, so a second copy can't manage the same journal
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
    /// Started one at a time, in order, each once nothing is running
    queue: Vec<Queued>,
    /// Off while a queue is being built, on from Start Queue or any start until it runs dry
    queue_moving: bool,
    /// Uploaded on the first frame, since that needs the egui context
    queue_icons: Option<icon::QueueIcons>,
    /// Journal entries whose copy was still locked, tried again every few seconds
    leftovers: Vec<String>,
    last_retry: Instant,
    status: Status,
    /// Finished this session, oldest first
    done: Vec<Done>,
    confirm_quit: bool,
    confirm_clear: bool,
    quitting: bool,
    focused_search: bool,
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Discord Quest Assistant")
            .with_icon(icon::window_icon())
            .with_inner_size([1080.0, 700.0])
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
            queue: Vec::new(),
            queue_moving: false,
            queue_icons: None,
            leftovers: Vec::new(),
            last_retry: Instant::now(),
            status: Status::good("Ready."),
            done: Vec::new(),
            confirm_quit: false,
            confirm_clear: false,
            quitting: false,
            focused_search: false,
            blocked,
            paths,
        };
        if app.blocked.is_none() {
            app.recover();
            app.load_catalog();
        } else {
            app.status = Status::warn("This window can't run games.");
        }
        app
    }

    /// Undoes whatever an earlier run left behind, from the journal
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

    /// Session folders under `runs` that no journal entry names, such as ones left by a journal
    /// that was set aside
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
                // A folder still in use stays until the next start
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
            // The window may have closed, and then nobody needs the answer
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

    /// Cleans up after copies that have exited
    fn tick(&mut self) {
        // Anything running sets the queue going, whichever way it was started
        self.queue_moving |= !self.running.is_empty();
        let mut ended: Vec<(String, String)> = Vec::new();
        for run in &mut self.running {
            if run.finished() {
                let early = run.remaining() > Duration::from_secs(5);
                self.done.push(Done {
                    game: run.game.clone(),
                    played: run.elapsed(),
                    full: !early,
                });
                let how = if early {
                    format!(
                        "{} was stopped with {} left",
                        run.game,
                        super::play::clock(run.remaining())
                    )
                } else {
                    format!("{} has run its full time", run.game)
                };
                ended.push((run.id.clone(), how));
            }
        }
        if self.done.len() > MAX_DONE {
            self.done.drain(..self.done.len() - MAX_DONE);
        }
        self.running
            .retain(|r| !ended.iter().any(|(id, _)| *id == r.id));
        // Everything that happens in one tick goes into one status, so an unattended queue
        // never hides a game that stopped short or was skipped
        let before = self.status.text.clone();
        let mut notes = Vec::new();
        for (id, how) in ended {
            if self.finish_entry(&id) {
                notes.push(format!("{how}, and its files are cleaned up."));
            } else {
                notes.push(format!("{how}."));
            }
        }
        if !self.leftovers.is_empty() && self.last_retry.elapsed() >= RETRY_EVERY {
            self.last_retry = Instant::now();
            for id in std::mem::take(&mut self.leftovers) {
                self.finish_entry(&id);
            }
        }
        // A cleanup warning from just now leads
        let mut tone = Tone::Good;
        if self.status.text != before && !matches!(self.status.tone, Tone::Good) {
            tone = self.status.tone;
            notes.insert(0, self.status.text.clone());
        }
        self.start_next(&mut notes, &mut tone);
        if !notes.is_empty() {
            self.status = Status {
                tone,
                text: notes.join(" "),
            };
        }
    }

    /// Goes down the queue until a game starts, so one that can't start doesn't hold up the rest
    /// Once nothing runs the queue waits again, for Start Queue or the next start
    fn start_next(&mut self, notes: &mut Vec<String>, tone: &mut Tone) {
        while self.queue_moving && self.running.is_empty() && !self.queue.is_empty() {
            let item = self.queue.remove(0);
            match self.launch(&item) {
                Ok(()) => notes.push(format!(
                    "{} from the queue is running for {} min.",
                    item.game, item.minutes
                )),
                Err(e) => {
                    notes.push(format!("{} was skipped: {e}", item.game));
                    *tone = Tone::Bad;
                }
            }
        }
        self.queue_moving = !self.running.is_empty();
    }

    fn start_queue(&mut self) {
        self.queue_moving = true;
        let mut notes = Vec::new();
        let mut tone = Tone::Good;
        self.start_next(&mut notes, &mut tone);
        if !notes.is_empty() {
            self.status = Status {
                tone,
                text: notes.join(" "),
            };
        }
    }

    /// True if everything was put back
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
                // Trying again won't change anything, so the entry goes
                let _ = self.journal.remove(id);
                false
            }
        }
    }

    fn stop_all(&mut self) {
        self.queue.clear();
        for run in &mut self.running {
            run.stop();
        }
        let ids: Vec<String> = self.running.drain(..).map(|r| r.id).collect();
        for id in ids {
            // Windows can hold an exited exe briefly, so give it a moment before giving up
            for _ in 0..20 {
                self.leftovers.retain(|l| *l != id);
                // Done, or kept as it is, so either way there's nothing to wait for
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
        self.load_queued();
    }

    /// The selected game's queued settings onto its card, so editing them edits the queue
    fn load_queued(&mut self) {
        let Some(game) = self.selected_game().cloned() else {
            return;
        };
        let Some(item) = self.queue.iter().find(|q| q.game == game.name) else {
            return;
        };
        #[allow(clippy::cast_precision_loss)]
        let minutes = item.minutes as f64;
        match item.target.clone() {
            Target::Sandbox(exe) => {
                let listed = game
                    .exes
                    .iter()
                    .position(|raw| ExePath::parse(raw).is_ok_and(|e| same_exe(&e, &exe)));
                match listed {
                    Some(index) => {
                        self.pick.steam_mode = false;
                        self.pick.exe = index;
                    }
                    // The card keeps the game's usual pick, and the queue keeps this exe until
                    // something is chosen
                    None => self.pick.unlisted = Some(exe.to_string()),
                }
            }
            Target::Steam { exe, .. } => {
                self.pick.steam_mode = true;
                self.scan_steam();
                let options = match &self.pick.steam {
                    Some(Ok(pick)) => pick.options.clone(),
                    _ => Vec::new(),
                };
                self.pick.launch = options
                    .iter()
                    .position(|o| same_exe(&o.exe, &exe))
                    .unwrap_or(options.len());
                if self.pick.launch == options.len() {
                    self.pick.custom = exe.to_string();
                }
            }
        }
        self.minutes = minutes;
        self.pick.saved = Some(self.card_key());
    }

    fn card_key(&self) -> CardKey {
        CardKey {
            steam_mode: self.pick.steam_mode,
            exe: self.pick.exe,
            launch: self.pick.launch,
            custom: self.pick.custom.clone(),
            minutes: self.chosen_minutes(),
        }
    }

    /// Writes the card's settings to the selected game's place in the queue when one changes
    /// Opening a card never writes, and settings that can't start keep the queue's last working ones
    fn sync_queued(&mut self) {
        let Some(name) = self.selected_game().map(|g| g.name.clone()) else {
            return;
        };
        if !self.queue.iter().any(|q| q.game == name) {
            // Queued again later, it takes what the card shows
            self.pick.saved = None;
            return;
        }
        let key = self.card_key();
        if self.pick.saved.as_ref() == Some(&key) {
            return;
        }
        let Ok(target) = self.target() else {
            return;
        };
        let minutes = self.chosen_minutes();
        if let Some(item) = self.queue.iter_mut().find(|q| q.game == name) {
            item.target = target;
            item.minutes = minutes;
        }
        self.pick.saved = Some(key);
        self.pick.unlisted = None;
    }

    /// The emptied search shows the pick took, and is ready for the next game
    fn pick_result(&mut self, index: usize) {
        self.select(index);
        self.query.clear();
        self.results.clear();
        self.focused_search = false;
    }

    /// Once per pick, or when asked again
    fn scan_steam(&mut self) {
        let Some(game) = self.selected_game() else {
            return;
        };
        let ids = game.steam.clone();
        // Some(None) is the custom path
        let chosen = match &self.pick.steam {
            Some(Ok(pick)) => Some(pick.options.get(self.pick.launch).map(|o| o.exe.clone())),
            _ => None,
        };
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
        let result = result.map_err(|e| format!("{e}."));
        // Checking again keeps the launch option chosen, if Steam still lists it
        self.pick.launch = match (&result, chosen) {
            (Ok(pick), Some(Some(exe))) => pick
                .options
                .iter()
                .position(|o| same_exe(&o.exe, &exe))
                .unwrap_or(0),
            (Ok(pick), Some(None)) => pick.options.len(),
            _ => 0,
        };
        self.pick.steam = Some(result);
    }

    /// Where the stand-in would go, or why it can't start yet
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

    /// Why this game can't be started or queued again, if it can't
    fn already_listed(&self, game: &str) -> Option<&'static str> {
        if self.running.iter().any(|r| r.game == game) {
            Some("Already running.")
        } else if self.queue.iter().any(|q| q.game == game) {
            Some("Already in the queue.")
        } else if self.leftovers.iter().any(|id| {
            self.journal
                .entries()
                .iter()
                .any(|e| e.id == *id && e.game == game)
        }) {
            // A Steam start would take the old copy for the game's own file
            Some("Still open from before.")
        } else {
            None
        }
    }

    /// Runs now when nothing else does, otherwise waits in the queue
    fn start(&mut self) {
        let (Ok(target), Some(game)) =
            (self.target(), self.selected_game().map(|g| g.name.clone()))
        else {
            return;
        };
        if self.already_listed(&game).is_some() {
            return;
        }
        let item = Queued {
            game,
            minutes: self.chosen_minutes(),
            target,
        };
        if !self.running.is_empty() {
            self.status = Status::good(format!(
                "{} is number {} in the queue.",
                item.game,
                self.queue.len() + 1
            ));
            self.queue.push(item);
            return;
        }
        match self.launch(&item) {
            Ok(()) => {
                self.status = Status::good(format!(
                    "{} is running for {} min.",
                    item.game, item.minutes
                ));
            }
            Err(e) => self.status = Status::bad(e),
        }
    }

    fn chosen_minutes(&self) -> u64 {
        // The field keeps the value within 1-240, so the cast is exact
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let minutes = (self.minutes.round().max(1.0) as u64).min(MAX_MINUTES);
        minutes
    }

    /// From a search result's buttons, with the game's usual settings and without picking it
    fn queue_from_search(&mut self, index: usize, top: bool) {
        let Some(game) = self.catalog.games.get(index).cloned() else {
            return;
        };
        if self.already_listed(&game.name).is_some() {
            return;
        }
        // Start Queue would skip every one of them
        if let Err(e) = &self.me {
            self.status = Status::bad(e.clone());
            return;
        }
        let target = match usual_target(&game) {
            Ok(target) => target,
            Err(e) => {
                self.status = Status::warn(format!("{} wasn't queued: {e}", game.name));
                return;
            }
        };
        let place = if top { 0 } else { self.queue.len() };
        self.queue.insert(
            place,
            Queued {
                game: game.name,
                minutes: self.chosen_minutes(),
                target,
            },
        );
        if let Some(item) = self.queue.get(place) {
            self.status = Status::good(format!(
                "{} is number {} in the queue.",
                item.game,
                place + 1
            ));
        }
    }

    fn launch(&mut self, item: &Queued) -> Result<(), String> {
        let me = self.me.as_ref().map_err(Clone::clone)?;
        // A queued pick can be old, and a finished download must never be replaced
        let target = match &item.target {
            Target::Steam { folder, exe } => Target::Steam {
                folder: steam::recheck(folder).map_err(|e| format!("{e}."))?,
                exe: exe.clone(),
            },
            sandbox @ Target::Sandbox(_) => sandbox.clone(),
        };
        let run = session::start(
            &item.game,
            item.minutes,
            &target,
            me,
            &self.paths,
            &mut self.journal,
        )
        .map_err(|e| format!("{e}."))?;
        self.running.push(run);
        Ok(())
    }
}

/// Windows paths ignore case, and Discord's list changes the case of some
fn same_exe(a: &ExePath, b: &ExePath) -> bool {
    a.to_string().eq_ignore_ascii_case(&b.to_string())
}

/// What a game's card starts with: Discord's first exe, or Steam's first launch option for a game
/// Discord finds only through Steam
fn usual_target(game: &Game) -> Result<Target, String> {
    if let Some(raw) = game.exes.first() {
        return ExePath::parse(raw)
            .map(Target::Sandbox)
            .map_err(|e| format!("Unusable path: {e}."));
    }
    let libraries = steam::libraries().map_err(|e| format!("{e}."))?;
    let folder = steam::find_game(&libraries, &game.steam).map_err(|e| format!("{e}."))?;
    let no_option = "Steam lists no launch option for it, so pick it and enter the exe's path.";
    let exe = steam::launch_options(&libraries, folder.app_id)
        .ok()
        .and_then(|options| options.into_iter().next())
        .map(|o| o.exe)
        .ok_or(no_option)?;
    if folder.installed && exe.under(&folder.folder).exists() {
        return Err("That file is part of the installed game, so it won't be replaced.".into());
    }
    Ok(Target::Steam { folder, exe })
}

/// Room the Refresh List button, its ⓘ and the gaps between them take beside the search field
fn refresh_row_reserve(ui: &Ui, label: &str) -> f32 {
    let spacing = &ui.style().spacing;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::INK)
        .size()
        .x;
    let button = (text + 2.0 * spacing.button_padding.x).max(spacing.interact_size.x);
    button + widgets::INFO_ICON + 2.0 * spacing.item_spacing.x
}

/// 19266 as 19,266
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

/// This exe's path and fingerprint, to make copies and know them again later
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
        self.body(ui);
        self.quit_dialog(&ctx);
        self.clear_dialog(&ctx);
        // A queue only moves while something runs, so a waiting one needs no repaints
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

/// From this width the window shows three panels side by side
const THREE_PANELS: f32 = 900.0;
/// From this width setup and Tonight sit side by side, below it everything stacks
const TWO_PANELS: f32 = 640.0;
/// Find a Game's and the game card's widths in the three-panel layout
/// Tonight takes what's left, so the strip fits three steps a row at the default size
fn three_panel_widths(width: f32) -> (f32, f32) {
    (
        (width * 0.30).clamp(290.0, 380.0),
        (width * 0.26).clamp(270.0, 340.0),
    )
}

/// Rows a search shows before it scrolls, when it shares its panel
const STACKED_RESULTS: usize = 7;

impl QuestApp {
    fn handle_close(&mut self, ctx: &egui::Context) {
        let asked = ctx.input(|i| i.viewport().close_requested());
        let busy = !self.running.is_empty() || !self.queue.is_empty();
        if asked && !self.quitting && busy {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_quit = true;
        }
    }

    fn dialog_frame(ctx: &egui::Context) -> Frame {
        Frame::popup(&ctx.global_style())
            .inner_margin(Margin::symmetric(20, 18))
            .corner_radius(theme::RADIUS_DIALOG)
    }

    fn dialog_heading(ui: &mut Ui, text: &str) {
        ui.label(
            RichText::new(text)
                .text_style(theme::Text::PanelHeading.style())
                .color(theme::INK),
        );
        ui.add_space(4.0);
    }

    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_quit {
            return;
        }
        // Everything ended while the dialog was open, so there's nothing left to lose
        if self.running.is_empty() && self.queue.is_empty() {
            self.confirm_quit = false;
            self.quitting = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let (heading, text, keep, quit) = match (self.running.is_empty(), self.queue.is_empty()) {
            (true, _) => (
                "Quit?",
                "The queue is cleared. None of it has started yet.",
                "Keep Queue",
                "Quit",
            ),
            (false, true) => (
                "Stop and Quit?",
                "The running games stop and their files are cleaned up. Quest progress so far stays.",
                "Keep Running",
                "Stop and Quit",
            ),
            (false, false) => (
                "Stop and Quit?",
                "The running games stop, the queue is cleared and their files are cleaned up. Quest progress so far stays.",
                "Keep Running",
                "Stop and Quit",
            ),
        };
        let modal = egui::Modal::new(egui::Id::new("confirm-quit"))
            .frame(Self::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                Self::dialog_heading(ui, heading);
                ui.label(RichText::new(text).color(theme::MUTED));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, keep).clicked() {
                        self.confirm_quit = false;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::danger_button(ui, quit).clicked() {
                            self.confirm_quit = false;
                            self.quitting = true;
                            self.stop_all();
                            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                        }
                    });
                });
            });
        if modal.should_close() {
            self.confirm_quit = false;
        }
    }

    fn clear_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_clear {
            return;
        }
        // The queue moved on while the dialog was open, so one game or none needs no asking
        if self.queue.len() < 2 {
            self.confirm_clear = false;
            return;
        }
        let text = format!(
            "{} waiting games are taken out. None of them has started, so there's nothing to clean up.",
            self.queue.len()
        );
        let modal = egui::Modal::new(egui::Id::new("confirm-clear"))
            .frame(Self::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                Self::dialog_heading(ui, "Clear the Queue?");
                ui.label(RichText::new(text).color(theme::MUTED));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, "Keep Queue").clicked() {
                        self.confirm_clear = false;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::danger_button(ui, "Clear Queue").clicked() {
                            self.confirm_clear = false;
                            self.clear_queue();
                        }
                    });
                });
            });
        if modal.should_close() {
            self.confirm_clear = false;
        }
    }

    fn status_strip(&self, ui: &mut Ui) {
        let frame = Frame::new()
            .fill(theme::GROUND)
            .inner_margin(Margin::symmetric(18, 10));
        Panel::bottom("status").frame(frame).show(ui, |ui| {
            let edge = ui.max_rect().expand2(egui::vec2(18.0, 10.0));
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

    /// Everything above the status line
    fn body(&mut self, ui: &mut Ui) {
        let frame = Frame::new().fill(theme::SURFACE);
        CentralPanel::default()
            .frame(frame)
            .show(ui, |ui| self.page(ui));
    }

    fn page(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        if let Some(why) = self.blocked.clone() {
            widgets::heading_bar(ui, "Can't Start", None);
            widgets::padded(ui, |ui| {
                ui.label(RichText::new(why).color(theme::MUTED));
            });
            return;
        }
        if self.queue_icons.is_none() {
            self.queue_icons = Some(icon::QueueIcons::load(ui.ctx()));
        }
        let width = rect.width();
        let seam = |ui: &Ui, x: f32| {
            ui.painter()
                .vline(x, rect.y_range(), Stroke::new(1.0, theme::LINE_SOFT));
        };
        if width >= THREE_PANELS {
            let (find, game) = three_panel_widths(width);
            let a = Rect::from_min_max(rect.min, pos2(rect.left() + find, rect.bottom()));
            let b = Rect::from_min_max(
                pos2(a.right() + 1.0, rect.top()),
                pos2(a.right() + 1.0 + game, rect.bottom()),
            );
            let c = Rect::from_min_max(pos2(b.right() + 1.0, rect.top()), rect.max);
            seam(ui, a.right() + 0.5);
            seam(ui, b.right() + 0.5);
            Self::panel(ui, a, "find", |ui| self.find_section(ui, false));
            Self::panel(ui, b, "game", |ui| {
                self.game_section(ui, Some("game-scroll"));
            });
            Self::panel(ui, c, "tonight", |ui| {
                self.tonight_section(ui, Some("tonight-scroll"));
            });
        } else if width >= TWO_PANELS {
            let a = Rect::from_min_max(rect.min, pos2(rect.left() + width * 0.54, rect.bottom()));
            let c = Rect::from_min_max(pos2(a.right() + 1.0, rect.top()), rect.max);
            seam(ui, a.right() + 0.5);
            Self::panel(ui, a, "setup", |ui| {
                Self::scrolled(ui, "setup-scroll", |ui| {
                    self.find_section(ui, true);
                    self.game_section(ui, None);
                });
            });
            Self::panel(ui, c, "tonight", |ui| {
                self.tonight_section(ui, Some("tonight-scroll"));
            });
        } else {
            Self::panel(ui, rect, "all", |ui| {
                Self::scrolled(ui, "all-scroll", |ui| {
                    self.find_section(ui, true);
                    self.game_section(ui, None);
                    self.tonight_section(ui, None);
                });
            });
        }
    }

    fn panel(ui: &mut Ui, rect: Rect, id: &str, contents: impl FnOnce(&mut Ui)) {
        let builder = egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min));
        ui.scope_builder(builder, |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing.y = 6.0;
            contents(ui);
        });
    }

    fn scrolled(ui: &mut Ui, id: &str, contents: impl FnOnce(&mut Ui)) {
        ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                contents(ui);
            });
    }

    /// `stacked` when it shares its panel, so its results stop at a few rows instead of filling it
    fn find_section(&mut self, ui: &mut Ui, stacked: bool) {
        widgets::heading_bar(ui, "Find a Game", None);
        let list_note = self.list_note();
        widgets::padded(ui, |ui| {
            let refresh_info = ui.horizontal(|ui| {
                let label = if self.download.is_some() {
                    "Updating…"
                } else {
                    "Refresh List"
                };
                let width = (ui.available_width() - refresh_row_reserve(ui, label)).max(120.0);
                let field = widgets::filled_field(ui, &mut self.query, "Game name", width);
                if !self.focused_search {
                    field.request_focus();
                    self.focused_search = true;
                }
                if field.changed() {
                    self.results = self.catalog.search(&self.query, MAX_RESULTS);
                }
                let refresh = ui.add_enabled(self.download.is_none(), egui::Button::new(label));
                if refresh.clicked() {
                    self.refresh_catalog();
                }
                widgets::info(ui, "Refresh List")
            });
            widgets::info_box(
                ui,
                refresh_info.inner,
                "Downloads Discord's list of games it can detect again. It updates by itself once a day.",
            );
            widgets::hint(ui, &list_note);
            ui.add_space(6.0);
            self.results_list(ui, stacked);
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

    fn results_list(&mut self, ui: &mut Ui, stacked: bool) {
        if self.query.trim().is_empty() {
            widgets::hint(ui, "Type a game's name to search Discord's list.");
            return;
        }
        if self.results.is_empty() {
            widgets::hint(ui, "No game in Discord's list matches that.");
            return;
        }
        let mut clicked = None;
        let mut queued = None;
        let icons = self.queue_icons.clone();
        let row = widgets::ROW_HEIGHT + ui.spacing().item_spacing.y;
        #[allow(clippy::cast_precision_loss)]
        let height = if stacked {
            self.results.len().min(STACKED_RESULTS) as f32 * row
        } else {
            ui.available_height().max(row)
        };
        ScrollArea::vertical()
            .id_salt("results")
            .auto_shrink([false, true])
            .min_scrolled_height(height)
            .max_height(height)
            .show(ui, |ui| {
                for &index in &self.results {
                    let Some(game) = self.catalog.games.get(index) else {
                        continue;
                    };
                    let detail = game.exes.first().map_or("Steam", String::as_str);
                    let selected = self.selected == Some(index);
                    let why_not = self
                        .already_listed(&game.name)
                        .or_else(|| self.me.as_ref().err().map(String::as_str));
                    ui.push_id(index, |ui| {
                        ui.horizontal(|ui| {
                            let spacing = ui.spacing().item_spacing.x;
                            let buttons = 2.0 * (widgets::ICON_BUTTON + spacing);
                            let row_width = (ui.available_width() - buttons).max(0.0);
                            let height =
                                widgets::choice_row_height(ui, &game.name, row_width, false);
                            let size = egui::vec2(row_width, height);
                            let name = ui.allocate_ui(size, |ui| {
                                widgets::choice_row(ui, selected, &game.name, detail, false)
                            });
                            if name.inner.clicked() {
                                clicked = Some(index);
                            }
                            let Some(icons) = &icons else {
                                return;
                            };
                            let bottom = "Add to Bottom of Queue";
                            if widgets::icon_button(ui, &icons.bottom, bottom, why_not).clicked() {
                                queued = Some((index, false));
                            }
                            let top = "Add to Top of Queue";
                            if widgets::icon_button(ui, &icons.top, top, why_not).clicked() {
                                queued = Some((index, true));
                            }
                        });
                    });
                }
            });
        if let Some(index) = clicked {
            self.pick_result(index);
        }
        if let Some((index, top)) = queued {
            self.queue_from_search(index, top);
        }
    }

    /// `scroll` when the panel is its own, so the heading stays put above the scrolling body
    fn game_section(&mut self, ui: &mut Ui, scroll: Option<&str>) {
        widgets::heading_bar(ui, "Game", None);
        match scroll {
            Some(id) => Self::scrolled(ui, id, |ui| self.game_body(ui)),
            None => self.game_body(ui),
        }
    }

    fn game_body(&mut self, ui: &mut Ui) {
        let Some(game) = self.selected_game().cloned() else {
            widgets::padded(ui, |ui| {
                widgets::hint(ui, "Pick a game from the list to see how it runs.");
            });
            return;
        };
        widgets::padded(ui, |ui| {
            // The picked game outranks the panel labels, it's what this panel is about
            ui.add(
                egui::Label::new(
                    RichText::new(&game.name)
                        .text_style(theme::Text::Nameplate.style())
                        .color(theme::INK),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            self.game_settings(ui, &game);
        });
        self.sync_queued();
    }

    fn game_settings(&mut self, ui: &mut Ui, game: &Game) {
        if !game.exes.is_empty() {
            self.exe_choice(ui, game);
        }
        if !game.steam.is_empty() {
            self.steam_choice(ui, game);
        }
        ui.add_space(6.0);
        let minutes_info = widgets::label_info(ui, "Minutes");
        widgets::number_field(ui, &mut self.minutes, 1.0..=240.0, 0.2, 0, "min", 96.0);
        widgets::info_box(
            ui,
            minutes_info,
            "How long the game runs. Most quests ask for 15 minutes; the extra minute covers the time Discord takes to notice it.",
        );
        ui.add_space(12.0);
        let target = self.target();
        // A Steam problem is already shown above, in the warning colour
        let steam_problem = self.pick.steam_mode && matches!(self.pick.steam, Some(Err(_)));
        if self.queue.iter().any(|q| q.game == game.name) {
            let warning = match (&target, &self.pick.unlisted) {
                (_, Some(exe)) => Some(format!(
                    "It's queued with {exe}, which Discord no longer lists. Choose another to replace it."
                )),
                (Err(_), None) if steam_problem => {
                    Some("The queue keeps its last working settings.".to_owned())
                }
                (Err(why), None) => {
                    Some(format!("{why} The queue keeps its last working settings."))
                }
                (Ok(_), None) => None,
            };
            match warning {
                Some(text) => {
                    ui.label(RichText::new(text).color(theme::WARN));
                }
                None => widgets::hint(ui, "Changes are saved to its place in the queue."),
            }
            return;
        }
        let listed = self.already_listed(&game.name);
        ui.horizontal_wrapped(|ui| {
            let can_start = target.is_ok() && self.me.is_ok() && listed.is_none();
            let label = if self.running.is_empty() {
                "Start"
            } else {
                "Add to Queue"
            };
            let start = ui
                .add_enabled_ui(can_start, |ui| widgets::primary_button(ui, label))
                .inner;
            if start.clicked() {
                self.start();
            }
            if let Some(why) = listed {
                widgets::hint(ui, why);
            } else if let (Err(why), false) = (&target, steam_problem) {
                widgets::hint(ui, why);
            } else if let Err(why) = &self.me {
                widgets::hint(ui, why);
            }
        });
    }

    fn exe_choice(&mut self, ui: &mut Ui, game: &Game) {
        let open = widgets::label_info(ui, "Executable");
        widgets::info_box(
            ui,
            open,
            "Discord knows the game by this file and the folders above it. The first usually works; try another if the quest doesn't move.",
        );
        let enabled = !self.pick.steam_mode;
        ui.add_enabled_ui(enabled, |ui| {
            for (i, exe) in game.exes.iter().enumerate().take(12) {
                if widgets::choice_row(ui, self.pick.exe == i, exe, "", true).clicked() {
                    self.pick.exe = i;
                    // A click is a choice even on the one already shown, so it's saved
                    self.pick.saved = None;
                }
            }
        });
        ui.add_space(8.0);
    }

    fn steam_choice(&mut self, ui: &mut Ui, game: &Game) {
        let steam_only = game.exes.is_empty();
        let open = ui
            .horizontal(|ui| {
                ui.add_enabled_ui(!steam_only, |ui| {
                    widgets::toggle(ui, &mut self.pick.steam_mode, "Run from Steam Library");
                });
                widgets::info(ui, "Run from Steam Library")
            })
            .inner;
        widgets::info_box(
            ui,
            open,
            "For games Discord finds through Steam, such as Marathon. Start the game's download in Steam and pause it at 1-2% first, so Steam makes its folder. A real install is never touched.",
        );
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
        let open = widgets::label_info(ui, "Launch Option");
        widgets::info_box(
            ui,
            open,
            "Steam's ways of starting the game. Some point at a launcher Discord doesn't watch; if the quest doesn't move, try another or enter the game's own exe.",
        );
        if let Some(note) = note {
            widgets::hint(ui, note);
        }
        for (i, option) in options.iter().enumerate().take(12) {
            let exe = option.exe.to_string();
            if widgets::choice_row(ui, self.pick.launch == i, &exe, &option.label, true).clicked() {
                self.pick.launch = i;
                self.pick.saved = None;
            }
        }
        let custom = options.len();
        if widgets::choice_row(ui, self.pick.launch >= custom, "Custom Path", "", false).clicked() {
            self.pick.launch = custom;
            self.pick.saved = None;
        }
        if self.pick.launch >= custom {
            let custom_info = ui
                .horizontal(|ui| {
                    let width =
                        (ui.available_width() - widgets::INFO_ICON - 8.0).clamp(80.0, 320.0);
                    widgets::filled_field(ui, &mut self.pick.custom, "bin/win64/game.exe", width);
                    widgets::info(ui, "Custom Path")
                })
                .inner;
            widgets::info_box(
                ui,
                custom_info,
                "The exe's path inside the game's folder, as on SteamDB or r/DiscordQuests.",
            );
        }
        ui.add_space(8.0);
    }

    /// The right panel's readout: when tonight ends, how much is queued, or that all is done
    fn readout(&self) -> Option<(String, egui::Color32)> {
        let queued: u64 = self.queue.iter().map(|q| q.minutes * 60).sum();
        if let Some(left) = self.running.iter().map(Running::remaining).max() {
            let end = left + Duration::from_secs(queued);
            return Some((
                format!("Ends in {}", super::play::clock(end)),
                theme::ACCENT,
            ));
        }
        if queued > 0 {
            return Some((
                format!("{} queued", super::play::clock(Duration::from_secs(queued))),
                theme::MUTED,
            ));
        }
        (!self.done.is_empty()).then(|| ("All done".to_owned(), theme::GOOD))
    }

    /// Done, then running, then queued, in the order they ran or will run
    fn steps(&self) -> Vec<widgets::Step> {
        let done = self.done.iter().map(|d| widgets::Step {
            state: if d.full {
                widgets::StepState::Done
            } else {
                widgets::StepState::Stopped
            },
            name: d.game.clone(),
            detail: if d.full {
                format!("{} played", super::play::clock(d.played))
            } else {
                format!("Stopped {}", super::play::clock(d.played))
            },
        });
        let live = self.running.iter().map(|r| widgets::Step {
            state: widgets::StepState::Live,
            name: r.game.clone(),
            detail: format!("{} left", super::play::clock(r.remaining())),
        });
        let queued = self.queue.iter().map(|q| widgets::Step {
            state: widgets::StepState::Queued,
            name: q.game.clone(),
            detail: format!("{} min", q.minutes),
        });
        done.chain(live).chain(queued).collect()
    }

    /// `scroll` when the panel is its own, so the heading and its readout stay put
    fn tonight_section(&mut self, ui: &mut Ui, scroll: Option<&str>) {
        let readout = self.readout();
        widgets::heading_bar(
            ui,
            "Tonight",
            readout
                .as_ref()
                .map(|(text, color)| (text.as_str(), *color)),
        );
        match scroll {
            Some(id) => Self::scrolled(ui, id, |ui| self.tonight_body(ui)),
            None => self.tonight_body(ui),
        }
    }

    fn tonight_body(&mut self, ui: &mut Ui) {
        let mut stop = None;
        let mut clicks = QueueClicks::default();
        widgets::padded(ui, |ui| {
            let steps = self.steps();
            if steps.is_empty() && self.leftovers.is_empty() {
                widgets::hint(
                    ui,
                    "Nothing queued yet. Use the queue buttons beside a game, or pick one and press Start.",
                );
                return;
            }
            if !steps.is_empty() {
                widgets::pipeline(ui, &steps);
            }
            if !self.running.is_empty() || !self.leftovers.is_empty() {
                widgets::section(ui, "Running");
            }
            for run in &self.running {
                // Ids by session, so a click aimed at a row that just ended can't land on the next
                ui.push_id(&run.id, |ui| {
                    ui.horizontal(|ui| {
                        let room = ui.available_width() - QUEUE_ROW_BUTTONS;
                        widgets::truncated(ui, &run.game, theme::INK, room);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::danger_button(ui, "Stop").clicked() {
                                stop = Some(run.id.clone());
                            }
                        });
                    });
                    // The one large figure, readable from across the room
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(super::play::clock(run.remaining()))
                                .font(egui::FontId::new(30.0, egui::FontFamily::Monospace))
                                .color(theme::ACCENT),
                        );
                        ui.label(RichText::new("left").color(theme::MUTED));
                    });
                });
                let fraction = run.elapsed().as_secs_f32() / run.length.as_secs_f32().max(1.0);
                widgets::progress_bar(ui, fraction);
                widgets::path_label(ui, &run.path.display().to_string(), theme::FAINT);
                ui.add_space(4.0);
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
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{game} is still open from before. It's cleaned up once its window closes."))
                                .color(theme::MUTED),
                        )
                        .wrap(),
                    );
                });
            }
            clicks = self.queue_list(ui);
        });
        self.act_on_queue(clicks);
        if let Some(id) = stop
            && let Some(run) = self.running.iter_mut().find(|r| r.id == id)
        {
            run.stop();
        }
    }

    /// Up Next with Remove on each, then Start Queue while nothing runs and Clear Queue apart
    fn queue_list(&self, ui: &mut Ui) -> QueueClicks {
        let mut clicks = QueueClicks::default();
        if self.queue.is_empty() {
            return clicks;
        }
        widgets::section(ui, "Up Next");
        let picked = self.selected_game().map(|g| g.name.clone());
        for (i, item) in self.queue.iter().enumerate() {
            // Hairlines between rows, so the minutes column reads as one
            if i > 0 {
                let (rule, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().hline(
                    rule.x_range(),
                    rule.center().y,
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
            }
            // A game is queued once at most, so its name is a lasting id
            ui.push_id(&item.game, |ui| {
                ui.horizontal(|ui| {
                    let selected = picked.as_ref() == Some(&item.game);
                    let room = ui.available_width() - QUEUE_ROW_BUTTONS;
                    if widgets::select_label(ui, &item.game, selected, room).clicked() {
                        clicks.edit = Some(item.game.clone());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::danger_button(ui, "Remove").clicked() {
                            clicks.remove = Some(item.game.clone());
                        }
                        ui.label(
                            RichText::new(format!("{} min", item.minutes))
                                .monospace()
                                .color(theme::MUTED),
                        );
                    });
                });
            });
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            // A game left over from before doesn't count, it isn't one the queue waits on
            if self.running.is_empty() {
                clicks.start = widgets::primary_button(ui, "Start Queue").clicked();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicks.clear = widgets::danger_button(ui, "Clear Queue").clicked();
            });
        });
        clicks
    }

    /// The catalog entry a queued game came from, which for two entries sharing a name is the
    /// one listing its exe
    fn catalog_index_of(&self, game: &str) -> Option<usize> {
        let queued = self.queue.iter().find(|q| q.game == game)?;
        let named = || {
            self.catalog
                .games
                .iter()
                .enumerate()
                .filter(move |(_, g)| g.name == game)
        };
        let lists_it = |g: &Game| match &queued.target {
            Target::Sandbox(exe) => g
                .exes
                .iter()
                .any(|raw| ExePath::parse(raw).is_ok_and(|e| same_exe(&e, exe))),
            Target::Steam { folder, .. } => g.steam.contains(&folder.app_id),
        };
        named()
            .find(|(_, g)| lists_it(g))
            .or_else(|| named().next())
            .map(|(i, _)| i)
    }

    fn act_on_queue(&mut self, clicks: QueueClicks) {
        if let Some(game) = clicks.remove {
            self.queue.retain(|q| q.game != game);
        }
        if clicks.start {
            self.start_queue();
        }
        if clicks.clear {
            if self.queue.len() > 1 {
                self.confirm_clear = true;
            } else {
                self.clear_queue();
            }
        }
        if let Some(game) = clicks.edit {
            match self.catalog_index_of(&game) {
                Some(index) => self.select(index),
                None => {
                    self.status = Status::warn(format!(
                        "{game} is no longer in Discord's list, so its settings can't be changed."
                    ));
                }
            }
        }
    }

    fn clear_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.queue.clear();
        self.status = Status::good("The queue is cleared.");
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

#[cfg(test)]
mod info_icons {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
    use std::path::PathBuf;

    const KEYS: [&str; 6] = [
        "Refresh List",
        "Executable",
        "Run from Steam Library",
        "Launch Option",
        "Custom Path",
        "Minutes",
    ];

    pub(super) fn test_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        fonts::install(&ctx);
        theme::apply(&ctx);
        ctx
    }

    /// Never loads, saves or locks anything, and Start stays disabled since `me` is an error
    pub(super) fn test_app(exes: &[&str]) -> QuestApp {
        let root = std::env::temp_dir().join("dqa-info-icons-never-written");
        let game = Game {
            name: "Marathon".into(),
            aliases: Vec::new(),
            exes: exes.iter().map(|s| (*s).to_owned()).collect(),
            steam: vec![3_065_800],
        };
        QuestApp {
            paths: Paths {
                runs: root.join("runs"),
                root,
            },
            me: Err("test".into()),
            _lock: None,
            blocked: None,
            catalog: Catalog {
                fetched_unix: now_unix(),
                games: vec![game],
            },
            download: None,
            query: "mara".into(),
            results: vec![0],
            selected: Some(0),
            pick: Pick {
                steam_mode: exes.is_empty(),
                ..Pick::default()
            },
            minutes: DEFAULT_MINUTES,
            journal: Journal::default(),
            running: Vec::new(),
            queue: Vec::new(),
            queue_moving: false,
            queue_icons: None,
            leftovers: Vec::new(),
            last_retry: Instant::now(),
            status: Status::good("Ready."),
            done: Vec::new(),
            confirm_quit: false,
            confirm_clear: false,
            quitting: false,
            focused_search: true,
        }
    }

    /// Steam mode with a found folder, so nothing scans the real Steam
    fn with_steam(mut app: QuestApp, launch: usize) -> QuestApp {
        app.pick.steam_mode = true;
        app.pick.steam = Some(Ok(SteamPick {
            folder: GameFolder {
                app_id: 3_065_800,
                folder: PathBuf::from(r"C:\No Such Library\steamapps\common\Marathon"),
                installed: false,
            },
            options: vec![LaunchOption {
                exe: ExePath::parse("marathon.exe").unwrap(),
                label: "Play".into(),
            }],
            note: None,
        }));
        app.pick.launch = launch;
        app
    }

    /// The window's own layout, minus the status strip
    fn frame(ctx: &egui::Context, app: &mut QuestApp, events: Vec<Event>, width: f32) {
        let input = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 3000.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            app.body(ui);
        })
        .textures_delta
        .clear();
    }

    fn icon(ctx: &egui::Context, key: &str) -> Option<Rect> {
        ctx.read_response(egui::Id::new(("info", key)))
            .map(|r| r.rect)
    }

    fn is_open(ctx: &egui::Context, key: &str) -> bool {
        ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("info", key))))
            .unwrap_or(false)
    }

    fn click(ctx: &egui::Context, app: &mut QuestApp, at: Pos2, width: f32) {
        for pressed in [true, false] {
            let event = Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            };
            frame(ctx, app, vec![Event::PointerMoved(at), event], width);
        }
        frame(ctx, app, vec![], width);
    }

    #[test]
    fn every_icon_is_drawn_and_toggles_only_its_own_box() {
        let ctx = test_ctx();
        let mut app = with_steam(test_app(&["bin/marathon.exe"]), 1);
        frame(&ctx, &mut app, vec![], 760.0);
        frame(&ctx, &mut app, vec![], 760.0);
        for key in KEYS {
            assert!(icon(&ctx, key).is_some(), "no icon for {key}");
        }
        let rects: Vec<Rect> = KEYS.iter().filter_map(|k| icon(&ctx, k)).collect();
        for (i, a) in rects.iter().enumerate() {
            for (j, b) in rects.iter().enumerate().skip(i + 1) {
                assert!(!a.intersects(*b), "{} and {} overlap", KEYS[i], KEYS[j]);
            }
        }
        for key in KEYS {
            let at = icon(&ctx, key).unwrap().center();
            click(&ctx, &mut app, at, 760.0);
            for other in KEYS {
                assert_eq!(
                    is_open(&ctx, other),
                    other == key,
                    "after clicking {key}, {other} is wrong"
                );
            }
            let at = icon(&ctx, key).unwrap().center();
            click(&ctx, &mut app, at, 760.0);
            assert!(!is_open(&ctx, key), "second click didn't close {key}");
        }
        assert_eq!(app.pick.launch, 1, "an icon click changed the launch pick");
        assert!(app.pick.steam_mode, "an icon click flipped the toggle");
    }

    #[test]
    fn all_boxes_open_at_once_and_stay_open() {
        let ctx = test_ctx();
        let mut app = with_steam(test_app(&["bin/marathon.exe"]), 1);
        frame(&ctx, &mut app, vec![], 760.0);
        for key in KEYS {
            // Boxes above push later icons down, so look each one up afresh
            let at = icon(&ctx, key).unwrap().center();
            click(&ctx, &mut app, at, 760.0);
        }
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![], 760.0);
        }
        for key in KEYS {
            assert!(is_open(&ctx, key), "{key} closed by itself");
        }
    }

    #[test]
    fn icons_work_beside_disabled_controls() {
        let ctx = test_ctx();
        // Steam only, so the toggle is disabled, and steam mode, so the exe list would be too
        let mut app = with_steam(test_app(&[]), 1);
        frame(&ctx, &mut app, vec![], 760.0);
        let at = icon(&ctx, "Run from Steam Library").unwrap().center();
        click(&ctx, &mut app, at, 760.0);
        assert!(
            is_open(&ctx, "Run from Steam Library"),
            "toggle's ⓘ dead while it's disabled"
        );

        let ctx = test_ctx();
        let mut app = with_steam(test_app(&["bin/marathon.exe"]), 0);
        frame(&ctx, &mut app, vec![], 760.0);
        let at = icon(&ctx, "Executable").unwrap().center();
        click(&ctx, &mut app, at, 760.0);
        assert!(
            is_open(&ctx, "Executable"),
            "Executable's ⓘ dead while the list is disabled"
        );
    }

    /// Right edge of the window, which every panel's content stays inside
    fn card_inner_right(_ctx: &egui::Context, width: f32) -> f32 {
        width
    }

    #[test]
    fn icons_fit_inside_their_card_from_min_to_wide_windows() {
        let mut overflows = Vec::new();
        for width in [520.0, 560.0, 600.0, 680.0, 760.0, 1200.0] {
            let card_right = card_inner_right(&test_ctx(), width);
            let ctx = test_ctx();
            let mut app = with_steam(test_app(&["bin/marathon.exe"]), 1);
            frame(&ctx, &mut app, vec![], width);
            frame(&ctx, &mut app, vec![], width);
            for key in KEYS {
                let rect = icon(&ctx, key).unwrap();
                if rect.right() > card_right + 0.5 {
                    overflows.push(format!(
                        "{width} px window: {key} ⓘ ends at {} but the card's content ends at {card_right}",
                        rect.right()
                    ));
                }
            }
        }
        assert!(overflows.is_empty(), "{overflows:#?}");
    }

    #[test]
    fn custom_path_box_reopens_with_its_field() {
        let ctx = test_ctx();
        let mut app = with_steam(test_app(&["bin/marathon.exe"]), 1);
        frame(&ctx, &mut app, vec![], 760.0);
        let at = icon(&ctx, "Custom Path").unwrap().center();
        click(&ctx, &mut app, at, 760.0);
        assert!(is_open(&ctx, "Custom Path"));
        app.pick.launch = 0;
        frame(&ctx, &mut app, vec![], 760.0);
        app.pick.launch = 1;
        frame(&ctx, &mut app, vec![], 760.0);
        assert!(
            is_open(&ctx, "Custom Path"),
            "hiding the field forgot the open box"
        );
    }
}

#[cfg(test)]
mod queue {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// Everything in a temp folder, with a copy of a quiet System32 tool as the stand-in so no
    /// window opens
    pub(super) fn app_in(tmp: &Path) -> QuestApp {
        let mut app = super::info_icons::test_app(&["bin/marathon.exe"]);
        app.paths = Paths {
            root: tmp.to_path_buf(),
            runs: tmp.join("runs"),
        };
        fs::create_dir_all(&app.paths.runs).unwrap();
        app.journal = Journal::load(&app.paths.journal()).0;
        let system = std::env::var("SystemRoot").unwrap();
        let exe = tmp.join("stand-in.exe");
        fs::copy(Path::new(&system).join(r"System32\hostname.exe"), &exe).unwrap();
        app.me = Ok(Me {
            sha256: journal::file_sha256(&exe).unwrap(),
            exe,
        });
        app
    }

    /// A game that keeps running until stopped
    pub(super) fn busy(game: &str) -> Running {
        let child = Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        Running::for_test(game, child)
    }

    pub(super) fn sandbox(game: &str) -> Queued {
        Queued {
            game: game.into(),
            minutes: 1,
            target: Target::Sandbox(ExePath::parse("bin/game.exe").unwrap()),
        }
    }

    /// Sandbox games only, so nothing reads the real Steam
    pub(super) fn games(names: &[&str]) -> Vec<Game> {
        names
            .iter()
            .map(|name| Game {
                name: (*name).into(),
                aliases: Vec::new(),
                exes: vec![format!("bin/{}.exe", name.to_lowercase())],
                steam: Vec::new(),
            })
            .collect()
    }

    fn names(app: &QuestApp) -> Vec<&str> {
        app.queue.iter().map(|q| q.game.as_str()).collect()
    }

    #[test]
    fn search_buttons_build_a_queue_that_waits_for_start_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = games(&["Alpha", "Beta", "Gamma"]);
        app.queue_from_search(0, false);
        app.queue_from_search(1, false);
        app.queue_from_search(2, true);
        assert_eq!(names(&app), ["Gamma", "Alpha", "Beta"]);
        assert!(
            app.status.text.contains("Gamma is number 1"),
            "{}",
            app.status.text
        );
        app.queue_from_search(0, true);
        assert_eq!(
            names(&app),
            ["Gamma", "Alpha", "Beta"],
            "Alpha was queued twice"
        );

        for _ in 0..3 {
            app.tick();
        }
        assert!(app.running.is_empty(), "the queue started by itself");
        assert!(app.journal.entries().is_empty());

        app.start_queue();
        assert_eq!(app.running[0].game, "Gamma");
        assert_eq!(names(&app), ["Alpha", "Beta"]);
        for next in ["Alpha", "Beta"] {
            app.running[0].stop();
            app.tick();
            assert_eq!(app.running[0].game, next);
        }
        app.running[0].stop();
        app.tick();
        assert!(app.running.is_empty() && app.queue.is_empty());
        assert!(!app.queue_moving);

        // Dry, so a new queue waits again
        app.queue_from_search(0, false);
        app.tick();
        app.tick();
        assert!(app.running.is_empty(), "a new queue started by itself");
        app.stop_all();
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn a_game_still_open_from_before_cant_be_queued_again() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = games(&["Alpha"]);
        app.journal
            .add(journal::Entry {
                id: "s1".into(),
                game: "Alpha".into(),
                changes: journal::Changes::Sandbox {
                    dir: app.paths.runs.join("s1"),
                },
            })
            .unwrap();
        app.leftovers.push("s1".into());
        assert_eq!(app.already_listed("Alpha"), Some("Still open from before."));
        app.queue_from_search(0, false);
        assert!(app.queue.is_empty());
    }

    #[test]
    fn starting_a_game_by_hand_sets_a_waiting_queue_going() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = games(&["Alpha", "Beta"]);
        app.queue_from_search(1, false);
        app.select(0);
        app.start();
        assert_eq!(
            app.running[0].game, "Alpha",
            "Start didn't run the picked game now"
        );
        assert_eq!(names(&app), ["Beta"]);
        app.running[0].stop();
        app.tick();
        assert_eq!(app.running[0].game, "Beta");
        app.stop_all();
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn picking_a_result_clears_the_search() {
        let mut app = super::info_icons::test_app(&["bin/marathon.exe"]);
        app.selected = None;
        app.pick_result(0);
        assert_eq!(app.selected, Some(0));
        assert!(
            app.query.is_empty(),
            "the search still reads {:?}",
            app.query
        );
        assert!(app.results.is_empty(), "the old results are still listed");
        assert!(
            !app.focused_search,
            "the search field should take focus again"
        );
    }

    #[test]
    fn start_queues_while_a_game_runs_then_runs_it_next() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Other Game"));
        app.start();
        assert_eq!(
            app.running.len(),
            1,
            "Marathon started beside the running game"
        );
        assert_eq!(app.queue.len(), 1);
        assert!(
            app.status.text.contains("number 1 in the queue"),
            "{}",
            app.status.text
        );

        app.start();
        assert_eq!(app.queue.len(), 1, "the same game was queued twice");
        assert_eq!(
            app.already_listed("Marathon"),
            Some("Already in the queue.")
        );

        app.running[0].stop();
        app.tick();
        assert!(app.queue.is_empty());
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.running[0].game, "Marathon");
        assert_eq!(app.journal.entries().len(), 1);
        assert!(
            app.status.text.contains("from the queue"),
            "{}",
            app.status.text
        );

        app.stop_all();
        assert!(
            app.journal.entries().is_empty(),
            "the queued run wasn't cleaned up"
        );
    }

    #[test]
    fn a_game_that_cant_start_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        let gone = GameFolder {
            app_id: 1,
            folder: tmp.path().join(r"common\Gone"),
            installed: false,
        };
        app.queue.push(Queued {
            game: "Gone Game".into(),
            minutes: 1,
            target: Target::Steam {
                folder: gone,
                exe: ExePath::parse("gone.exe").unwrap(),
            },
        });
        app.queue.push(sandbox("Next Game"));
        app.start_queue();
        assert!(app.queue.is_empty());
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.running[0].game, "Next Game");
        assert!(
            app.status.text.starts_with("Gone Game was skipped"),
            "the skip was hidden: {}",
            app.status.text
        );
        app.stop_all();
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn nothing_starts_from_the_queue_while_a_game_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Other Game"));
        app.queue.push(sandbox("Next Game"));
        for _ in 0..3 {
            app.tick();
        }
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.queue.len(), 1);
        assert!(
            app.journal.entries().is_empty(),
            "something was written early"
        );
        app.stop_all();
        assert!(app.queue.is_empty(), "quitting kept the queue");
        assert!(app.running.is_empty());
    }
}

/// Edge cases around the queue, all in temp folders with no real Steam or data folder
#[cfg(test)]
mod queue_edges {
    use super::info_icons::test_ctx;
    use super::queue::{app_in, busy, sandbox};
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
    use std::path::Path;

    fn missing_steam(tmp: &Path, game: &str) -> Queued {
        Queued {
            game: game.into(),
            minutes: 1,
            target: Target::Steam {
                folder: GameFolder {
                    app_id: 1,
                    folder: tmp.join("common").join(game),
                    installed: false,
                },
                exe: ExePath::parse("gone.exe").unwrap(),
            },
        }
    }

    fn frame(ctx: &egui::Context, app: &mut QuestApp, events: Vec<Event>) {
        let input = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(760.0, 3000.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            app.body(ui);
        })
        .textures_delta
        .clear();
    }

    fn press_release(ctx: &egui::Context, app: &mut QuestApp, at: Pos2) {
        for pressed in [true, false] {
            let event = Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            };
            frame(ctx, app, vec![Event::PointerMoved(at), event]);
        }
    }

    /// Clickable widgets of the last frame, top to bottom
    fn clickables(ctx: &egui::Context) -> Vec<Rect> {
        let mut rects: Vec<Rect> = ctx.viewport(|vp| {
            vp.this_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.enabled)
                .map(|w| w.rect)
                .collect()
        });
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
        rects
    }

    /// Leaves the search card as one row, the field with Refresh List and its ⓘ
    fn clear_search(app: &mut QuestApp) {
        app.query.clear();
        app.results.clear();
    }

    /// Buttons below the search row, so the Running card's Stop and Remove in order
    /// Buttons are 28 px or more, the clickable game names 18 px
    fn queue_card_clickables(ctx: &egui::Context) -> Vec<Rect> {
        let rects = clickables(ctx);
        let Some(search) = rects.first().map(Rect::bottom) else {
            return rects;
        };
        rects
            .into_iter()
            .filter(|r| r.top() > search + 1.0 && r.height() >= 24.0)
            .collect()
    }

    fn names(app: &QuestApp) -> Vec<&str> {
        app.queue.iter().map(|q| q.game.as_str()).collect()
    }

    #[test]
    fn every_skipped_game_is_reported_not_just_the_last() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.queue.push(missing_steam(tmp.path(), "First Gone"));
        app.queue.push(missing_steam(tmp.path(), "Second Gone"));
        app.queue.push(sandbox("Next Game"));
        app.start_queue();
        assert_eq!(app.running.len(), 1);
        let text = app.status.text.clone();
        app.stop_all();
        assert!(
            text.contains("First Gone"),
            "First Gone left the queue without a word, status reads: {text}"
        );
    }

    #[test]
    fn a_game_that_ended_early_is_still_reported_when_the_next_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        // Its window closed or it crashed, well before its 60 s were up
        app.running.push(busy("Early Game"));
        app.running[0].stop();
        app.queue.push(sandbox("Next Game"));
        app.tick();
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.running[0].game, "Next Game");
        let text = app.status.text.clone();
        app.stop_all();
        assert!(
            text.contains("Early Game"),
            "nothing says Early Game stopped short of its time, status reads: {text}"
        );
    }

    #[test]
    fn a_queued_steam_game_installed_since_is_never_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        let apps = tmp.path().join("lib").join("steamapps");
        let folder = apps.join("common").join("Late Game");
        fs::create_dir_all(&folder).unwrap();
        // Partial when queued, so the scan said not installed
        let item = Queued {
            game: "Late Game".into(),
            minutes: 1,
            target: Target::Steam {
                folder: GameFolder {
                    app_id: 7,
                    folder: folder.clone(),
                    installed: false,
                },
                exe: ExePath::parse("game.exe").unwrap(),
            },
        };
        app.running.push(busy("Other Game"));
        app.queue.push(item);
        // Steam resumed the download and finished it while the queue waited
        fs::write(folder.join("game.exe"), b"the real game").unwrap();
        fs::write(
            apps.join("appmanifest_7.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"installdir\"\t\t\"Late Game\"\n\t\"StateFlags\"\t\t\"4\"\n}\n",
        )
        .unwrap();
        app.running[0].stop();
        app.tick();
        let real_moved = folder.join("game.exe.dqa-backup").exists();
        let started = app.running.iter().any(|r| r.game == "Late Game");
        app.stop_all();
        assert_eq!(
            fs::read(folder.join("game.exe")).unwrap(),
            b"the real game",
            "cleanup didn't put the real exe back"
        );
        assert!(
            !real_moved && !started,
            "the installed game's exe was moved aside and the stand-in put in its place"
        );
    }

    #[test]
    fn remove_takes_the_row_the_user_saw_even_if_the_queue_moved_on() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        clear_search(&mut app);
        app.selected = None;
        app.running.push(busy("Game A"));
        app.queue.push(sandbox("Game B"));
        app.queue.push(sandbox("Game C"));
        let ctx = test_ctx();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rects = queue_card_clickables(&ctx);
        // Stop, then Remove for B, then Remove for C
        assert!(rects.len() >= 3, "{rects:?}");
        let remove_b = rects[1].center();
        assert!(
            (rects[1].center().x - rects[2].center().x).abs() < 1.0
                && rects[2].top() > rects[1].top(),
            "rows aren't where expected: {rects:?}"
        );

        // A ends just before the click lands, so the frame that takes the click starts B first
        app.running[0].stop();
        app.tick();
        assert_eq!(names(&app), ["Game C"]);
        press_release(&ctx, &mut app, remove_b);
        let left = names(&app).join(", ");
        app.stop_all();
        assert_eq!(
            left, "Game C",
            "clicking B's Remove took C out of the queue instead"
        );
    }

    /// The 28 px icon buttons that take clicks, row by row, left to right
    fn icon_buttons(ctx: &egui::Context) -> Vec<Rect> {
        let size = widgets::ICON_BUTTON;
        let mut rects: Vec<Rect> = clickables(ctx)
            .into_iter()
            .filter(|r| (r.width() - size).abs() < 0.5 && (r.height() - size).abs() < 0.5)
            .collect();
        rects.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        rects
    }

    #[test]
    fn the_row_buttons_queue_at_top_and_bottom_without_picking() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = super::queue::games(&["Alpha", "Beta"]);
        app.query = "a".into();
        app.results = vec![0, 1];
        app.selected = None;
        let ctx = test_ctx();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let buttons = icon_buttons(&ctx);
        assert_eq!(buttons.len(), 4, "{buttons:?}");

        // Each row is Bottom then Top, so Alpha to the bottom, then Beta to the top
        press_release(&ctx, &mut app, buttons[0].center());
        press_release(&ctx, &mut app, buttons[3].center());
        frame(&ctx, &mut app, vec![]);
        assert_eq!(names(&app), ["Beta", "Alpha"]);
        assert_eq!(app.selected, None, "a queue button picked the game");
        assert_eq!(app.query, "a", "a queue button cleared the search");
        assert!(app.running.is_empty(), "the queue started by itself");
        // The widget list read back here lags a frame
        frame(&ctx, &mut app, vec![]);
        let left = icon_buttons(&ctx);
        assert!(
            left.is_empty(),
            "buttons on queued games still take clicks: {left:?} of {:?}",
            clickables(&ctx)
        );
    }

    /// Alpha with two exes, queued with the second one and 30 minutes
    fn alpha_queued_on_its_second_exe(tmp: &Path) -> QuestApp {
        let mut app = app_in(tmp);
        let mut alpha = super::queue::games(&["Alpha"]).remove(0);
        alpha.exes.push("win64/alpha.exe".into());
        app.catalog.games = vec![alpha];
        clear_search(&mut app);
        app.selected = None;
        app.minutes = 16.0;
        app.queue.push(Queued {
            game: "Alpha".into(),
            minutes: 30,
            target: Target::Sandbox(ExePath::parse("win64/alpha.exe").unwrap()),
        });
        app
    }

    fn queued_exe(app: &QuestApp) -> String {
        match &app.queue[0].target {
            Target::Sandbox(exe) => exe.to_string(),
            Target::Steam { .. } => "steam".into(),
        }
    }

    #[test]
    fn clicking_a_queued_game_opens_its_own_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = alpha_queued_on_its_second_exe(tmp.path());
        let ctx = test_ctx();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let search = clickables(&ctx)[0].bottom();
        let name = clickables(&ctx)
            .into_iter()
            .find(|r| r.top() > search && r.height() < 24.0)
            .expect("the queued name isn't clickable");
        press_release(&ctx, &mut app, name.center());
        assert_eq!(
            app.selected,
            Some(0),
            "clicking the name didn't pick the game"
        );
        assert_eq!(
            app.pick.exe, 1,
            "the card shows the usual exe, not the queued one"
        );
        assert!(
            (app.minutes - 30.0).abs() < 0.01,
            "minutes read {}",
            app.minutes
        );
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.queue[0].minutes, 30,
            "opening the card changed the queue"
        );
        assert_eq!(queued_exe(&app), "win64/alpha.exe");
    }

    #[test]
    fn changes_on_a_queued_games_card_reach_the_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = alpha_queued_on_its_second_exe(tmp.path());
        app.pick_result(0);
        assert_eq!(
            app.pick.exe, 1,
            "picking from search ignored the queued settings"
        );
        app.pick.exe = 0;
        app.minutes = 20.0;
        let ctx = test_ctx();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(queued_exe(&app), "bin/alpha.exe");
        assert_eq!(app.queue[0].minutes, 20);
    }

    #[test]
    fn settings_that_cant_start_leave_the_queue_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = alpha_queued_on_its_second_exe(tmp.path());
        app.select(0);
        // As if Steam mode were on and Steam couldn't be read
        app.pick.steam_mode = true;
        app.pick.steam = Some(Err("Steam doesn't seem to be installed.".into()));
        app.minutes = 45.0;
        let ctx = test_ctx();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(queued_exe(&app), "win64/alpha.exe");
        assert_eq!(app.queue[0].minutes, 30, "half-valid settings were saved");
    }

    #[test]
    fn remove_takes_out_only_that_row() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        clear_search(&mut app);
        app.selected = None;
        app.running.push(busy("Game A"));
        for g in ["Game B", "Game C", "Game D"] {
            app.queue.push(sandbox(g));
        }
        let ctx = test_ctx();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rects = queue_card_clickables(&ctx);
        press_release(&ctx, &mut app, rects[2].center());
        frame(&ctx, &mut app, vec![]);
        assert_eq!(names(&app), ["Game B", "Game D"]);
        assert_eq!(app.running.len(), 1, "Remove stopped the running game");
        assert!(
            !app.running[0].finished(),
            "Remove stopped the running game"
        );
        app.stop_all();
    }

    #[test]
    fn a_catalog_refresh_keeps_the_queue_and_it_still_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Game A"));
        app.start();
        assert_eq!(names(&app), ["Marathon"]);
        let (send, receive) = mpsc::channel();
        send.send(Ok(Catalog {
            fetched_unix: now_unix(),
            games: Vec::new(),
        }))
        .unwrap();
        app.download = Some(receive);
        app.poll_download();
        assert_eq!(app.selected, None);
        assert_eq!(names(&app), ["Marathon"], "the refresh dropped the queue");
        app.running[0].stop();
        app.tick();
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.running[0].game, "Marathon");
        app.stop_all();
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn queueing_writes_nothing_until_the_game_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Game A"));
        app.start();
        assert_eq!(app.queue.len(), 1);
        assert!(app.journal.entries().is_empty());
        assert!(
            fs::read_dir(&app.paths.runs).unwrap().next().is_none(),
            "a session folder was made for a queued game"
        );
        assert!(!app.paths.journal().exists(), "the journal was written");
        app.stop_all();
    }

    #[test]
    fn start_with_me_an_error_says_why_and_queues_nothing() {
        let mut app = super::info_icons::test_app(&["bin/marathon.exe"]);
        app.start();
        assert!(app.running.is_empty());
        assert!(app.queue.is_empty());
        assert!(matches!(app.status.tone, Tone::Bad), "{}", app.status.text);
        assert_eq!(app.status.text, "test");
    }

    #[test]
    fn a_queue_of_only_failures_empties_and_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.queue.push(missing_steam(tmp.path(), "Gone One"));
        app.queue.push(missing_steam(tmp.path(), "Gone Two"));
        app.start_queue();
        assert!(app.queue.is_empty());
        assert!(app.running.is_empty());
        assert!(!app.queue_moving, "a dry queue should wait again");
        assert!(matches!(app.status.tone, Tone::Bad));
        assert!(
            app.journal.entries().is_empty(),
            "a failed start left a journal entry"
        );
        // Nothing left to do, so the window can close without asking
        let ctx = test_ctx();
        app.handle_close(&ctx);
        assert!(!app.confirm_quit);
    }

    #[test]
    fn closing_with_only_a_queue_asks_first() {
        let mut app = super::info_icons::test_app(&["bin/marathon.exe"]);
        app.queue.push(sandbox("Game B"));
        let ctx = test_ctx();
        let mut input = RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        ctx.run_ui(input, |_| app.handle_close(&ctx))
            .textures_delta
            .clear();
        assert!(app.confirm_quit, "closing with a queue didn't ask");
    }
}

/// The search rows' queue buttons and the queue's run state, in temp folders only
#[cfg(test)]
mod queue_buttons_edges {
    use super::info_icons::test_ctx;
    use super::queue::{app_in, busy, games, sandbox};
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

    fn names(app: &QuestApp) -> Vec<&str> {
        app.queue.iter().map(|q| q.game.as_str()).collect()
    }

    fn press(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        }
    }

    /// Laid out the way `ui` lays out the real window, at the given width
    /// Each frame is a quarter second on, so scroll bar animations settle
    fn real_frame(ctx: &egui::Context, app: &mut QuestApp, width: f32, events: Vec<Event>) {
        let time = ctx.input(|i| i.time) + 0.25;
        let input = RawInput {
            events,
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 720.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            app.body(ui);
        })
        .textures_delta
        .clear();
    }

    fn clickables(ctx: &egui::Context) -> Vec<Rect> {
        ctx.viewport(|vp| {
            vp.this_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.enabled)
                .map(|w| w.rect)
                .collect()
        })
    }

    fn icon_buttons(ctx: &egui::Context) -> Vec<Rect> {
        let size = widgets::ICON_BUTTON;
        let mut rects: Vec<Rect> = clickables(ctx)
            .into_iter()
            .filter(|r| (r.width() - size).abs() < 0.5 && (r.height() - size).abs() < 0.5)
            .collect();
        rects.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        rects
    }

    /// Every label and value the frame exposes, read back through AccessKit
    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .map(|u| {
                u.nodes
                    .iter()
                    .flat_map(|(_, n)| {
                        [n.label(), n.value()]
                            .into_iter()
                            .flatten()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn dialog_texts(ctx: &egui::Context, app: &mut QuestApp) -> Vec<String> {
        let mut output = ctx.run_ui(RawInput::default(), |ui| app.quit_dialog(ui.ctx()));
        output.textures_delta.clear();
        texts(&output)
    }

    #[test]
    fn row_buttons_are_off_when_no_game_can_start() {
        // `me` is an error here, the same state that disables the card's Start
        let mut app = super::info_icons::test_app(&["bin/marathon.exe"]);
        app.catalog.games = games(&["Alpha", "Beta"]);
        app.query = "a".into();
        app.results = vec![0, 1];
        app.selected = None;
        let ctx = test_ctx();
        for _ in 0..3 {
            real_frame(&ctx, &mut app, 760.0, vec![]);
        }
        let enabled = icon_buttons(&ctx);
        assert!(
            enabled.is_empty(),
            "{} queue buttons take clicks though nothing can start, so Start Queue would skip and drop every game",
            enabled.len()
        );
    }

    #[test]
    fn a_dialog_left_open_after_the_last_game_ends_says_nothing_about_a_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Game A"));
        app.confirm_quit = true;
        let ctx = test_ctx();
        ctx.enable_accesskit();
        let before = dialog_texts(&ctx, &mut app);
        assert!(
            before.iter().any(|t| t == "Stop and Quit?"),
            "AccessKit readback found no dialog: {before:?}"
        );
        // The game ends while the dialog waits for an answer
        app.running[0].stop();
        app.tick();
        assert!(app.running.is_empty() && app.queue.is_empty());
        let after = dialog_texts(&ctx, &mut app);
        app.stop_all();
        assert!(
            !after.iter().any(|t| t.to_lowercase().contains("queue")),
            "with nothing running or queued the dialog reads {after:?}"
        );
    }

    #[test]
    fn a_catalog_refresh_mid_click_never_queues_a_different_game() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = games(&["Alpha"]);
        app.query = "alpha".into();
        app.results = app.catalog.search(&app.query, MAX_RESULTS);
        app.selected = None;
        let ctx = test_ctx();
        for _ in 0..2 {
            real_frame(&ctx, &mut app, 760.0, vec![]);
        }
        let buttons = icon_buttons(&ctx);
        assert_eq!(buttons.len(), 2, "{buttons:?}");
        let at = buttons[1].center();
        real_frame(
            &ctx,
            &mut app,
            760.0,
            vec![Event::PointerMoved(at), press(at, true)],
        );

        // The daily refresh lands, with a new game that sorts after Alpha but takes index 0
        let (send, receive) = mpsc::channel();
        send.send(Ok(Catalog {
            fetched_unix: now_unix(),
            games: games(&["Alphabet", "Alpha"]),
        }))
        .unwrap();
        app.download = Some(receive);
        app.poll_download();
        assert_eq!(app.results, [1, 0], "Alpha should stay on the first row");
        real_frame(&ctx, &mut app, 760.0, vec![press(at, false)]);
        let queued = names(&app).join(", ");
        app.stop_all();
        assert!(
            queued.is_empty() || queued == "Alpha",
            "pressed on Alpha's row, queued {queued}"
        );
    }

    #[test]
    fn at_the_narrowest_window_the_row_buttons_fit_and_overlap_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        let long = "An Extremely Long Game Name That Goes On and On Past Any Sensible Width";
        let mut list = games(&["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta"]);
        list.push(Game {
            name: long.into(),
            aliases: Vec::new(),
            exes: vec!["steamapps/common/some very long folder name/binaries/win64/game-win64-shipping.exe".into()],
            steam: Vec::new(),
        });
        app.catalog.games = list;
        app.query = "a".into();
        app.results = (0..app.catalog.games.len()).collect();
        app.selected = None;
        let ctx = test_ctx();
        let width = 520.0;
        for _ in 0..3 {
            real_frame(&ctx, &mut app, width, vec![]);
        }
        let buttons = icon_buttons(&ctx);
        assert!(buttons.len() >= 2, "no row buttons at 520 px: {buttons:?}");
        let others: Vec<Rect> = clickables(&ctx)
            .into_iter()
            .filter(|r| !buttons.contains(r))
            .collect();
        // The panel's padding starts where the search row ends
        let card_right = {
            let rects = clickables(&ctx);
            let top = rects.iter().map(Rect::top).fold(f32::INFINITY, f32::min);
            rects
                .iter()
                .filter(|r| r.top() < top + 20.0)
                .map(Rect::right)
                .fold(0.0, f32::max)
        };
        for b in &buttons {
            assert!(
                b.right() <= card_right + 0.5,
                "button {b:?} runs past the card's edge at {card_right}"
            );
            assert!(b.left() > 28.0, "button {b:?} squeezed off the left");
            for o in &others {
                assert!(
                    !b.shrink(0.5).intersects(*o),
                    "button {b:?} overlaps another control {o:?}"
                );
            }
        }
        app.stop_all();
    }

    /// An Add to Bottom button close to the window's right edge, as the rows' buttons sit, on a row
    /// the page has scrolled up near the window's top
    #[test]
    fn a_tooltip_near_the_top_right_flips_below_instead_of_covering_the_button() {
        let ctx = test_ctx();
        ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        let image = egui::ColorImage::filled([4, 4], egui::Color32::WHITE);
        let icon = ctx.load_texture("test-icon", image, egui::TextureOptions::LINEAR);
        let run = |events: Vec<Event>, time: f64| {
            let input = RawInput {
                events,
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(520.0, 480.0))),
                ..Default::default()
            };
            let mut out = None;
            ctx.run_ui(input, |ui| {
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    ui.add_space(42.0 - ui.spacing().item_spacing.x);
                    out = Some(widgets::icon_button(
                        ui,
                        &icon,
                        "Add to Bottom of Queue",
                        None,
                    ));
                });
            })
            .textures_delta
            .clear();
            out.unwrap()
        };
        let at = run(vec![], 0.0).rect.center();
        let mut response = run(vec![Event::PointerMoved(at)], 0.2);
        for step in 2..6 {
            response = run(vec![], f64::from(step) * 0.2);
        }
        let tip = ctx
            .memory(|m| m.area_rect(egui::Tooltip::tooltip_id(response.id, 0)))
            .expect("no tooltip while hovering");
        assert!(
            tip.top() >= -0.5,
            "tooltip {tip:?} is cut off above the window"
        );
        assert!(
            !tip.intersects(response.rect.shrink(0.5)),
            "tooltip {tip:?} covers the button {:?}",
            response.rect
        );
    }

    #[test]
    fn duplicate_names_in_the_catalog_queue_once() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        let mut list = games(&["Alpha"]);
        list.push(Game {
            name: "Alpha".into(),
            aliases: Vec::new(),
            exes: vec!["other/alpha.exe".into()],
            steam: Vec::new(),
        });
        app.catalog.games = list;
        app.queue_from_search(0, false);
        app.queue_from_search(1, true);
        assert_eq!(names(&app), ["Alpha"]);
    }

    #[test]
    fn a_queued_pick_cant_be_started_from_its_card_too() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.select(0);
        app.queue_from_search(0, false);
        app.start();
        assert!(app.running.is_empty(), "the queued game also started");
        assert_eq!(names(&app), ["Marathon"]);
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn minutes_are_taken_when_queued_and_kept_in_range() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = games(&["Alpha", "Beta", "Gamma"]);
        app.minutes = 45.4;
        app.queue_from_search(0, false);
        app.minutes = 0.2;
        app.queue_from_search(1, false);
        app.minutes = 999.0;
        app.queue_from_search(2, false);
        app.minutes = 10.0;
        let minutes: Vec<u64> = app.queue.iter().map(|q| q.minutes).collect();
        assert_eq!(minutes, [45, 1, MAX_MINUTES]);
    }

    #[test]
    fn an_unusable_exe_is_reported_and_not_queued() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = vec![Game {
            name: "Broken".into(),
            aliases: Vec::new(),
            exes: vec!["../../escape.exe".into()],
            steam: Vec::new(),
        }];
        app.queue_from_search(0, true);
        assert!(app.queue.is_empty());
        assert!(matches!(app.status.tone, Tone::Warn), "{}", app.status.text);
        assert!(app.status.text.contains("Broken"), "{}", app.status.text);
        // An index from a list that has since shrunk does nothing
        app.queue_from_search(5, true);
        assert!(app.queue.is_empty());
    }

    #[test]
    fn a_failed_start_from_the_card_leaves_a_waiting_queue_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.queue.push(sandbox("Game B"));
        // A file where the runs folder should be, so the next start can't make its folder
        let blocker = tmp.path().join("runs-blocked");
        fs::write(&blocker, b"").unwrap();
        app.paths.runs = blocker;
        app.select(0);
        app.start();
        assert!(app.running.is_empty());
        assert!(matches!(app.status.tone, Tone::Bad), "{}", app.status.text);
        for _ in 0..3 {
            app.tick();
        }
        assert_eq!(
            names(&app),
            ["Game B"],
            "the waiting queue moved or emptied"
        );
        assert!(!app.queue_moving);
        assert!(app.running.is_empty());
    }

    #[test]
    fn start_queue_works_beside_a_leftover_and_keeps_going() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.query.clear();
        app.results.clear();
        app.selected = None;
        app.leftovers.push("s1".into());
        app.queue.push(sandbox("Game B"));
        app.queue.push(sandbox("Game C"));
        let ctx = test_ctx();
        for _ in 0..2 {
            real_frame(&ctx, &mut app, 760.0, vec![]);
        }
        // Start Queue is the leftmost button on the bottom row, Clear Queue sits apart on its right
        let rects = clickables(&ctx);
        // By centre line, since the smaller Clear Queue sits a few pixels lower on the same row
        let bottom = rects.iter().map(|r| r.center().y).fold(f32::MIN, f32::max);
        let start = rects
            .iter()
            .filter(|r| (r.center().y - bottom).abs() < 4.0)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .copied()
            .unwrap();
        real_frame(
            &ctx,
            &mut app,
            760.0,
            vec![
                Event::PointerMoved(start.center()),
                press(start.center(), true),
            ],
        );
        real_frame(&ctx, &mut app, 760.0, vec![press(start.center(), false)]);
        assert_eq!(
            app.running.len(),
            1,
            "Start Queue didn't start: {}",
            app.status.text
        );
        assert_eq!(app.running[0].game, "Game B");
        app.running[0].stop();
        app.tick();
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.running[0].game, "Game C");
        app.stop_all();
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn stopping_the_last_running_game_moves_the_queue_on() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Game A"));
        app.queue_from_search(0, false);
        app.tick();
        assert!(app.queue_moving);
        app.running[0].stop();
        app.tick();
        assert_eq!(app.running.len(), 1);
        assert_eq!(app.running[0].game, "Marathon");
        app.stop_all();
        assert!(app.journal.entries().is_empty());
    }
}

/// Changing a queued game's settings from its card, in temp folders with no real Steam
#[cfg(test)]
mod queue_editing_edges {
    use super::info_icons::test_ctx;
    use super::queue::{app_in, busy, games};
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
    use std::path::{Path, PathBuf};

    fn game(name: &str, exes: &[&str], steam: &[u32]) -> Game {
        Game {
            name: name.into(),
            aliases: Vec::new(),
            exes: exes.iter().map(|s| (*s).to_owned()).collect(),
            steam: steam.to_vec(),
        }
    }

    fn sandboxed(name: &str, exe: &str, minutes: u64) -> Queued {
        Queued {
            game: name.into(),
            minutes,
            target: Target::Sandbox(ExePath::parse(exe).unwrap()),
        }
    }

    /// Search emptied and nothing picked, so the card below the search is the queue
    fn app_with(tmp: &Path, list: Vec<Game>) -> QuestApp {
        let mut app = app_in(tmp);
        app.catalog.games = list;
        app.query.clear();
        app.results.clear();
        app.selected = None;
        app.minutes = 16.0;
        app
    }

    /// The real window's layout, a quarter second per frame
    fn frame_at(
        ctx: &egui::Context,
        app: &mut QuestApp,
        width: f32,
        events: Vec<Event>,
    ) -> egui::FullOutput {
        let time = ctx.input(|i| i.time) + 0.25;
        let input = RawInput {
            events,
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 3000.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.body(ui);
        });
        out.textures_delta.clear();
        out
    }

    fn frame(ctx: &egui::Context, app: &mut QuestApp) {
        frame_at(ctx, app, 760.0, vec![]);
    }

    fn click(ctx: &egui::Context, app: &mut QuestApp, at: Pos2) {
        for pressed in [true, false] {
            let event = Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            };
            frame_at(ctx, app, 760.0, vec![Event::PointerMoved(at), event]);
        }
    }

    fn clickables(ctx: &egui::Context) -> Vec<Rect> {
        let mut rects: Vec<Rect> = ctx.viewport(|vp| {
            vp.this_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.enabled)
                .map(|w| w.rect)
                .collect()
        });
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
        rects
    }

    /// Clickable queued names, only meaningful while no settings card is open
    fn queued_name_rects(ctx: &egui::Context) -> Vec<Rect> {
        let rects = clickables(ctx);
        let search = rects[0].bottom();
        rects
            .into_iter()
            .filter(|r| r.top() > search + 1.0 && r.height() < 24.0)
            .collect()
    }

    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .map(|u| {
                u.nodes
                    .iter()
                    .flat_map(|(_, n)| {
                        [n.label(), n.value()]
                            .into_iter()
                            .flatten()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn edit(app: &mut QuestApp, name: &str) {
        app.act_on_queue(QueueClicks {
            edit: Some(name.into()),
            ..QueueClicks::default()
        });
    }

    fn names(app: &QuestApp) -> Vec<&str> {
        app.queue.iter().map(|q| q.game.as_str()).collect()
    }

    fn exe_of(app: &QuestApp, name: &str) -> String {
        match &app.queue.iter().find(|q| q.game == name).unwrap().target {
            Target::Sandbox(exe) => exe.to_string(),
            Target::Steam { exe, .. } => format!("steam:{exe}"),
        }
    }

    fn minutes_of(app: &QuestApp, name: &str) -> u64 {
        app.queue.iter().find(|q| q.game == name).unwrap().minutes
    }

    #[test]
    fn opening_a_queued_game_whose_exe_left_discords_list_keeps_the_queued_exe() {
        let tmp = tempfile::tempdir().unwrap();
        // Queued on win64/alpha.exe, then a list refresh dropped that exe
        let mut app = app_with(tmp.path(), vec![game("Alpha", &["bin/alpha.exe"], &[])]);
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        let name = queued_name_rects(&ctx)[0];
        click(&ctx, &mut app, name.center());
        frame(&ctx, &mut app);
        assert_eq!(
            app.selected,
            Some(0),
            "clicking the name didn't open the card"
        );
        assert_eq!(
            exe_of(&app, "Alpha"),
            "win64/alpha.exe",
            "only opening the card swapped the queued exe for the list's first one"
        );
        assert_eq!(minutes_of(&app, "Alpha"), 30);
    }

    #[test]
    fn a_list_refresh_that_changes_an_exes_case_doesnt_rewrite_the_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![game("Alpha", &["bin/alpha.exe", "Win64/Alpha.exe"], &[])],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        assert_eq!(
            app.pick.exe, 1,
            "the card didn't find the queued exe, Windows paths ignore case"
        );
        assert_ne!(
            exe_of(&app, "Alpha"),
            "bin/alpha.exe",
            "opening the card moved the queued game onto a different exe"
        );
    }

    #[test]
    fn opening_the_second_of_two_same_named_games_from_the_queue_keeps_its_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![
                game("Alpha", &["bin/alpha.exe"], &[]),
                game("Alpha", &["other/alpha.exe"], &[]),
            ],
        );
        // Queued from the second row's button
        app.queue_from_search(1, false);
        assert_eq!(exe_of(&app, "Alpha"), "other/alpha.exe");
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        let name = queued_name_rects(&ctx)[0];
        click(&ctx, &mut app, name.center());
        frame(&ctx, &mut app);
        assert_eq!(
            exe_of(&app, "Alpha"),
            "other/alpha.exe",
            "opening the queued game's card rewrote it with the first same-named entry's exe"
        );
    }

    #[test]
    fn a_game_the_queue_starts_while_its_card_is_open_takes_no_later_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[])],
        );
        app.running.push(busy("Other"));
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        app.running[0].stop();
        app.tick();
        let started: Vec<(String, Duration, PathBuf)> = app
            .running
            .iter()
            .map(|r| (r.game.clone(), r.length, r.path.clone()))
            .collect();
        // Edits on the still open card after the start
        app.minutes = 50.0;
        app.pick.exe = 0;
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        let listed = app.already_listed("Alpha");
        let queue_len = app.queue.len();
        app.start();
        let after: Vec<(String, Duration)> = app
            .running
            .iter()
            .map(|r| (r.game.clone(), r.length))
            .collect();
        app.stop_all();
        assert_eq!(started.len(), 1, "Alpha didn't start from the queue");
        assert_eq!(started[0].0, "Alpha");
        assert_eq!(started[0].1, Duration::from_secs(30 * 60));
        assert!(
            started[0].2.ends_with(Path::new("win64").join("alpha.exe")),
            "{:?}",
            started[0].2
        );
        assert_eq!(
            queue_len, 0,
            "an edit put the started game back in the queue"
        );
        assert_eq!(listed, Some("Already running."));
        assert_eq!(
            after,
            [("Alpha".to_owned(), Duration::from_secs(30 * 60))],
            "a second copy started, or the running one changed"
        );
        assert!(app.journal.entries().is_empty());
    }

    #[test]
    fn start_queue_with_the_open_card_first_runs_the_cards_latest_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![
                game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[]),
                game("Beta", &["bin/beta.exe"], &[]),
            ],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        app.queue.push(sandboxed("Beta", "bin/beta.exe", 10));
        edit(&mut app, "Alpha");
        app.pick.exe = 0;
        app.minutes = 45.0;
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        app.act_on_queue(QueueClicks {
            start: true,
            ..QueueClicks::default()
        });
        frame(&ctx, &mut app);
        let run: Vec<(String, Duration, PathBuf)> = app
            .running
            .iter()
            .map(|r| (r.game.clone(), r.length, r.path.clone()))
            .collect();
        let left: Vec<String> = names(&app).iter().map(|s| (*s).to_owned()).collect();
        let beta = (exe_of(&app, "Beta"), minutes_of(&app, "Beta"));
        app.stop_all();
        assert_eq!(run.len(), 1);
        assert_eq!(run[0].0, "Alpha");
        assert_eq!(
            run[0].1,
            Duration::from_secs(45 * 60),
            "ran with the old minutes"
        );
        assert!(
            run[0].2.ends_with(Path::new("bin").join("alpha.exe")),
            "ran with the old exe: {:?}",
            run[0].2
        );
        assert_eq!(left, ["Beta"]);
        assert_eq!(
            beta,
            ("bin/beta.exe".to_owned(), 10),
            "Alpha's card leaked into Beta"
        );
    }

    #[test]
    fn removing_the_open_queued_game_then_queueing_it_again_uses_the_cards_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[])],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        app.act_on_queue(QueueClicks {
            remove: Some("Alpha".into()),
            ..QueueClicks::default()
        });
        frame(&ctx, &mut app);
        assert!(app.queue.is_empty(), "Remove left the open game queued");
        assert_eq!(app.selected, Some(0), "Remove closed the card");
        assert_eq!(app.already_listed("Alpha"), None);
        // A row button on the same game, then the frame's card draw
        app.queue_from_search(0, false);
        frame(&ctx, &mut app);
        assert_eq!(names(&app), ["Alpha"]);
        assert_eq!(exe_of(&app, "Alpha"), "win64/alpha.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 30);
    }

    #[test]
    fn the_row_button_on_the_open_cards_game_queues_what_the_card_shows() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![
                game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[]),
                game("Beta", &["bin/beta.exe"], &[]),
            ],
        );
        app.query = "a".into();
        app.results = vec![0, 1];
        app.select(0);
        app.pick.exe = 1;
        app.minutes = 25.0;
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        let size = widgets::ICON_BUTTON;
        let mut buttons: Vec<Rect> = clickables(&ctx)
            .into_iter()
            .filter(|r| (r.width() - size).abs() < 0.5 && (r.height() - size).abs() < 0.5)
            .collect();
        buttons.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        assert_eq!(buttons.len(), 4, "{buttons:?}");
        // Alpha's Add to Bottom
        click(&ctx, &mut app, buttons[0].center());
        frame(&ctx, &mut app);
        assert_eq!(names(&app), ["Alpha"]);
        assert_eq!(exe_of(&app, "Alpha"), "win64/alpha.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 25);
        assert!(app.running.is_empty());
    }

    #[test]
    fn a_catalog_refresh_while_editing_keeps_the_last_saved_settings_and_reopens_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let two = || game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[]);
        let mut app = app_with(tmp.path(), vec![two()]);
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        app.pick.exe = 0;
        app.minutes = 20.0;
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        let (send, receive) = mpsc::channel();
        send.send(Ok(Catalog {
            fetched_unix: now_unix(),
            games: vec![game("Zed", &["bin/zed.exe"], &[]), two()],
        }))
        .unwrap();
        app.download = Some(receive);
        app.poll_download();
        assert_eq!(app.selected, None);
        app.minutes = 99.0;
        frame(&ctx, &mut app);
        assert_eq!(exe_of(&app, "Alpha"), "bin/alpha.exe");
        assert_eq!(
            minutes_of(&app, "Alpha"),
            20,
            "an edit after the card closed reached the queue"
        );
        edit(&mut app, "Alpha");
        assert_eq!(
            app.selected,
            Some(1),
            "reopened the wrong row after the list moved"
        );
        assert_eq!(app.pick.exe, 0);
        assert!((app.minutes - 20.0).abs() < 0.01);
        frame(&ctx, &mut app);
        assert_eq!(exe_of(&app, "Alpha"), "bin/alpha.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 20);
    }

    #[test]
    fn a_queued_game_gone_from_the_list_says_so_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(tmp.path(), games(&["Beta"]));
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        assert_eq!(app.selected, None);
        assert!(matches!(app.status.tone, Tone::Warn), "{}", app.status.text);
        assert!(app.status.text.contains("Alpha"), "{}", app.status.text);
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        assert_eq!(exe_of(&app, "Alpha"), "win64/alpha.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 30);
    }

    #[test]
    fn switching_between_queued_games_keeps_each_ones_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![
                game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[]),
                game("Beta", &["bin/beta.exe"], &[]),
            ],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        app.queue.push(sandboxed("Beta", "bin/beta.exe", 10));
        edit(&mut app, "Alpha");
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        // Half-made settings that can't start, then straight on to Beta
        app.pick.steam_mode = true;
        app.pick.steam = Some(Err("Steam doesn't seem to be installed.".into()));
        app.minutes = 99.0;
        frame(&ctx, &mut app);
        edit(&mut app, "Beta");
        frame(&ctx, &mut app);
        edit(&mut app, "Beta");
        frame(&ctx, &mut app);
        assert!(
            (app.minutes - 10.0).abs() < 0.01,
            "Beta's card shows {}",
            app.minutes
        );
        assert!(!app.pick.steam_mode);
        edit(&mut app, "Alpha");
        assert_eq!(app.pick.exe, 1);
        assert!(!app.pick.steam_mode, "Alpha's unsaved Steam mode came back");
        frame(&ctx, &mut app);
        assert_eq!(exe_of(&app, "Alpha"), "win64/alpha.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 30);
        assert_eq!(exe_of(&app, "Beta"), "bin/beta.exe");
        assert_eq!(minutes_of(&app, "Beta"), 10);
        assert_eq!(names(&app), ["Alpha", "Beta"]);
    }

    #[test]
    fn viewing_a_queued_game_then_picking_another_never_touches_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![
                game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[]),
                game("Gamma", &["bin/gamma.exe", "x/gamma.exe"], &[]),
            ],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        let ctx = test_ctx();
        frame(&ctx, &mut app);
        app.pick_result(1);
        app.minutes = 50.0;
        app.pick.exe = 1;
        frame(&ctx, &mut app);
        assert_eq!(names(&app), ["Alpha"]);
        assert_eq!(exe_of(&app, "Alpha"), "win64/alpha.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 30);
    }

    #[test]
    fn a_steam_error_on_a_queued_games_card_is_shown_once() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[1])],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        // Steam mode turned on and Steam couldn't be read, already set so nothing scans
        app.pick.steam_mode = true;
        app.pick.steam = Some(Err("Steam doesn't seem to be installed.".into()));
        let ctx = test_ctx();
        ctx.enable_accesskit();
        frame(&ctx, &mut app);
        let out = frame_at(&ctx, &mut app, 760.0, vec![]);
        let shown = texts(&out)
            .iter()
            .filter(|t| t.contains("Steam doesn't seem to be installed"))
            .count();
        assert_eq!(exe_of(&app, "Alpha"), "win64/alpha.exe");
        assert_eq!(
            shown,
            1,
            "the Steam error is shown {shown} times on a queued game's card: {:?}",
            texts(&out)
        );
    }

    #[test]
    fn a_queued_game_now_found_only_through_steam_isnt_stuck_with_steam_off() {
        let tmp = tempfile::tempdir().unwrap();
        // Queued while Discord listed an exe, which a refresh has since dropped
        let mut app = app_with(tmp.path(), vec![game("Alpha", &[], &[1])]);
        app.queue.push(sandboxed("Alpha", "bin/alpha.exe", 30));
        // No frame, since a Steam card would read the real Steam
        edit(&mut app, "Alpha");
        app.minutes = 45.0;
        app.sync_queued();
        assert!(
            app.pick.steam_mode || minutes_of(&app, "Alpha") == 45,
            "the card opened with Steam off and its toggle locked, so nothing on it can be saved"
        );
    }

    #[test]
    fn a_queued_sandbox_game_switched_to_steam_saves_only_a_complete_path() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(
            tmp.path(),
            vec![game("Alpha", &["bin/alpha.exe", "win64/alpha.exe"], &[1])],
        );
        app.queue.push(sandboxed("Alpha", "win64/alpha.exe", 30));
        edit(&mut app, "Alpha");
        app.pick.steam_mode = true;
        app.pick.steam = Some(Ok(SteamPick {
            folder: GameFolder {
                app_id: 1,
                folder: tmp.path().join(r"lib\steamapps\common\Alpha"),
                installed: false,
            },
            options: Vec::new(),
            note: Some("Steam lists no launch option.".into()),
        }));
        app.pick.launch = 0;
        let ctx = test_ctx();
        for typed in ["", "b", "bin/x", "bin/x.ex"] {
            app.pick.custom = typed.into();
            frame(&ctx, &mut app);
            assert_eq!(
                exe_of(&app, "Alpha"),
                "win64/alpha.exe",
                "a half-typed path {typed:?} reached the queue"
            );
        }
        for key in [
            "Executable",
            "Run from Steam Library",
            "Launch Option",
            "Custom Path",
            "Minutes",
        ] {
            assert!(
                ctx.read_response(egui::Id::new(("info", key))).is_some(),
                "no ⓘ for {key} on a queued game's card"
            );
        }
        app.pick.custom = "bin/x.exe".into();
        frame(&ctx, &mut app);
        assert_eq!(exe_of(&app, "Alpha"), "steam:bin/x.exe");
        assert_eq!(minutes_of(&app, "Alpha"), 30);
    }

    #[test]
    fn a_long_queued_name_keeps_remove_inside_the_card_at_the_narrowest_window() {
        let tmp = tempfile::tempdir().unwrap();
        // The same name the search rows' narrow window test uses, 72 characters
        // A 50 character title still fits
        let long = "An Extremely Long Game Name That Goes On and On Past Any Sensible Width";
        let mut app = app_with(tmp.path(), vec![game(long, &["bin/long.exe"], &[])]);
        app.queue.push(sandboxed(long, "bin/long.exe", 16));
        let ctx = test_ctx();
        let width = 520.0;
        for _ in 0..3 {
            frame_at(&ctx, &mut app, width, vec![]);
        }
        let rects = clickables(&ctx);
        let search = rects[0].bottom();
        let below: Vec<Rect> = rects
            .into_iter()
            .filter(|r| r.top() > search + 1.0)
            .collect();
        let name = below
            .iter()
            .find(|r| r.height() < 24.0)
            .copied()
            .expect("no name");
        let remove = below
            .iter()
            .find(|r| {
                r.height() >= 24.0 && (r.center().y - name.center().y).abs() < 10.0 && **r != name
            })
            .copied()
            .expect("no Remove on the name's row");
        // The panel's padding starts where the search row ends
        let card_right = {
            let rects = clickables(&ctx);
            let top = rects.iter().map(Rect::top).fold(f32::INFINITY, f32::min);
            rects
                .iter()
                .filter(|r| r.top() < top + 20.0)
                .map(Rect::right)
                .fold(0.0, f32::max)
        };
        assert!(
            remove.right() <= card_right + 0.5,
            "Remove {remove:?} runs past the card's edge at {card_right}"
        );
        assert!(
            !name.shrink(0.5).intersects(remove),
            "the name {name:?} runs under Remove {remove:?}"
        );
    }

    #[test]
    fn button_names_show_sooner_than_egui_default() {
        let ctx = test_ctx();
        let delay = ctx.global_style().interaction.tooltip_delay;
        assert!(delay > 0.0 && delay < 0.5, "tooltip delay is {delay}");
    }
}

#[cfg(test)]
mod snapshot;

/// The three-panel layout, the Tonight readout and strip, and Clear Queue
#[cfg(test)]
mod redesign {
    use super::info_icons::test_ctx;
    use super::queue::{app_in, busy, games, sandbox};
    use super::*;
    use egui::{Pos2, RawInput, vec2};

    /// Where each heading bar landed, by its title, from every frame's tree update since later
    /// frames only send what changed
    fn headings(ctx: &egui::Context, app: &mut QuestApp, width: f32) -> Vec<(String, Rect)> {
        ctx.enable_accesskit();
        let mut found: Vec<(String, Rect)> = Vec::new();
        for _ in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.body(ui));
            output.textures_delta.clear();
            let Some(update) = output.platform_output.accesskit_update else {
                continue;
            };
            for (_, n) in &update.nodes {
                // egui keeps a label's text as its value
                let (Some(label), Some(b)) = (n.label().or(n.value()), n.bounds()) else {
                    continue;
                };
                if !["Find a Game", "Game", "Tonight"].contains(&label) {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                let rect = Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                );
                found.retain(|(l, _)| l != label);
                found.push((label.to_owned(), rect));
            }
        }
        found
    }

    fn at(list: &[(String, Rect)], label: &str) -> Rect {
        let found = list.iter().find(|(l, _)| l == label);
        assert!(found.is_some(), "no {label} heading in {list:?}");
        found.map_or(Rect::NOTHING, |(_, r)| *r)
    }

    #[test]
    fn three_panels_wide_two_in_between_and_one_column_narrow() {
        let mut app = super::info_icons::test_app(&["bin/marathon.exe"]);
        app.selected = None;

        let wide = headings(&test_ctx(), &mut app, 1080.0);
        let (find, game, tonight) = (
            at(&wide, "Find a Game"),
            at(&wide, "Game"),
            at(&wide, "Tonight"),
        );
        assert!(
            (find.top() - game.top()).abs() < 1.0 && (game.top() - tonight.top()).abs() < 1.0,
            "three panels should share a top row: {wide:?}"
        );
        assert!(find.right() <= game.left() && game.right() <= tonight.left());

        let middle = headings(&test_ctx(), &mut app, 760.0);
        let (find, game, tonight) = (
            at(&middle, "Find a Game"),
            at(&middle, "Game"),
            at(&middle, "Tonight"),
        );
        assert!((find.left() - game.left()).abs() < 1.0 && game.top() > find.bottom());
        assert!(tonight.left() >= find.right() && (tonight.top() - find.top()).abs() < 1.0);

        let narrow = headings(&test_ctx(), &mut app, 520.0);
        let (find, game, tonight) = (
            at(&narrow, "Find a Game"),
            at(&narrow, "Game"),
            at(&narrow, "Tonight"),
        );
        assert!((find.left() - tonight.left()).abs() < 1.0);
        assert!(find.bottom() <= game.top() && game.bottom() <= tonight.top());
    }

    #[test]
    fn the_readout_says_when_tonight_ends_or_what_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        assert!(app.readout().is_none(), "an empty evening has no readout");

        app.queue.push(sandbox("Alpha"));
        app.queue.push(sandbox("Beta"));
        assert_eq!(app.readout().unwrap().0, "2:00 queued");

        app.running.push(busy("Gamma"));
        let (text, color) = app.readout().unwrap();
        // 60 s left on the running one plus two queued minutes
        assert!(
            text.starts_with("Ends in 2:5") || text == "Ends in 3:00",
            "{text}"
        );
        assert_eq!(color, theme::ACCENT);

        app.stop_all();
        app.done.push(Done {
            game: "Gamma".into(),
            played: Duration::from_secs(30),
            full: true,
        });
        assert_eq!(app.readout().unwrap().0, "All done");
    }

    #[test]
    fn the_strip_runs_done_then_live_then_queued() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.done.push(Done {
            game: "Alpha".into(),
            played: Duration::from_secs(16 * 60),
            full: true,
        });
        app.running.push(busy("Beta"));
        app.queue.push(sandbox("Gamma"));
        let steps = app.steps();
        let order: Vec<(&str, widgets::StepState)> =
            steps.iter().map(|s| (s.name.as_str(), s.state)).collect();
        assert_eq!(
            order,
            [
                ("Alpha", widgets::StepState::Done),
                ("Beta", widgets::StepState::Live),
                ("Gamma", widgets::StepState::Queued),
            ]
        );
        assert_eq!(steps[0].detail, "16:00 played");
        assert!(steps[1].detail.ends_with(" left"));
        assert_eq!(steps[2].detail, "1 min");
        app.stop_all();
    }

    #[test]
    fn a_game_stopped_early_says_so_on_the_strip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.done.push(Done {
            game: "Alpha".into(),
            played: Duration::from_secs(11 * 60 + 48),
            full: false,
        });
        let steps = app.steps();
        assert_eq!(steps[0].state, widgets::StepState::Stopped);
        assert_eq!(steps[0].detail, "Stopped 11:48");
    }

    #[test]
    fn a_game_that_ends_joins_the_strip_and_the_strip_keeps_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.running.push(busy("Alpha"));
        app.running[0].stop();
        app.tick();
        assert_eq!(app.done.len(), 1);
        assert_eq!(app.done[0].game, "Alpha");

        for i in 0..MAX_DONE + 6 {
            app.done.push(Done {
                game: format!("Game {i}"),
                played: Duration::from_secs(60),
                full: true,
            });
        }
        app.tick();
        assert_eq!(app.done.len(), MAX_DONE);
        assert_eq!(
            app.done.last().map(|d| d.game.as_str()),
            Some(format!("Game {}", MAX_DONE + 5).as_str()),
            "the newest was dropped instead of the oldest"
        );
    }

    #[test]
    fn clear_queue_asks_only_when_more_than_one_game_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.catalog.games = games(&["Alpha", "Beta"]);

        app.queue.push(sandbox("Alpha"));
        app.act_on_queue(QueueClicks {
            clear: true,
            ..QueueClicks::default()
        });
        assert!(
            app.queue.is_empty(),
            "one waiting game is cleared without asking"
        );
        assert!(!app.confirm_clear);
        assert_eq!(app.status.text, "The queue is cleared.");

        app.queue.push(sandbox("Alpha"));
        app.queue.push(sandbox("Beta"));
        app.act_on_queue(QueueClicks {
            clear: true,
            ..QueueClicks::default()
        });
        assert!(app.confirm_clear, "two waiting games should ask first");
        assert_eq!(app.queue.len(), 2, "asking must not clear yet");
        app.clear_queue();
        assert!(app.queue.is_empty());
    }

    #[test]
    fn the_clear_dialog_closes_itself_once_one_game_or_none_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        app.queue.push(sandbox("Alpha"));
        app.confirm_clear = true;
        let ctx = test_ctx();
        ctx.run_ui(RawInput::default(), |ui| app.clear_dialog(ui.ctx()))
            .textures_delta
            .clear();
        assert!(!app.confirm_clear);
        assert_eq!(app.queue.len(), 1, "closing the dialog cleared the queue");
    }
}

/// Breakpoints, the strip, danger buttons, Clear Queue and the dialogs under stress, in temp
/// folders only
#[cfg(test)]
mod redesign_edges {
    use super::info_icons::test_ctx;
    use super::queue::{app_in, busy, games, sandbox};
    use super::*;
    use egui::epaint::{ClippedShape, Primitive, Shape};
    use egui::{
        Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, TouchPhase, Vec2,
        vec2,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    const LONG: &str = "An Extremely Long Game Name That Goes On and On Past Any Sensible Width";
    const LONG_EXE: &str =
        "steamapps/common/some very long folder name/binaries/win64/game-win64-shipping.exe";

    /// What `ui` does each frame, minus eframe, a quarter second per frame
    fn window(
        ctx: &egui::Context,
        app: &mut QuestApp,
        size: Vec2,
        events: Vec<Event>,
        tick: bool,
    ) -> egui::FullOutput {
        let time = ctx.input(|i| i.time) + 0.25;
        let input = RawInput {
            events,
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.handle_close(&ctx);
            if tick {
                app.tick();
            }
            app.status_strip(ui);
            app.body(ui);
            app.quit_dialog(&ctx);
            app.clear_dialog(&ctx);
        });
        out.textures_delta.clear();
        out
    }

    /// AccessKit nodes by id, newest frame winning, since later frames only send what changed
    #[derive(Default)]
    struct Nodes(HashMap<egui::accesskit::NodeId, (String, Rect)>);

    impl Nodes {
        fn take(&mut self, out: &egui::FullOutput) {
            let Some(update) = &out.platform_output.accesskit_update else {
                return;
            };
            for (id, n) in &update.nodes {
                let (Some(text), Some(b)) = (n.label().or(n.value()), n.bounds()) else {
                    continue;
                };
                #[allow(clippy::cast_possible_truncation)]
                let rect = Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                );
                self.0.insert(*id, (text.to_owned(), rect));
            }
        }

        fn all(&self, text: &str) -> Vec<Rect> {
            let mut found: Vec<Rect> = self
                .0
                .values()
                .filter(|(t, _)| t == text)
                .map(|(_, r)| *r)
                .collect();
            found.sort_by(|a, b| a.top().total_cmp(&b.top()));
            found
        }

        fn one(&self, text: &str) -> Rect {
            let found = self.all(text);
            assert_eq!(found.len(), 1, "{text}: {found:?}");
            found[0]
        }

        fn has(&self, text: &str) -> bool {
            self.0.values().any(|(t, _)| t == text)
        }
    }

    fn settle(ctx: &egui::Context, app: &mut QuestApp, size: Vec2, frames: usize) -> Nodes {
        ctx.enable_accesskit();
        let mut nodes = Nodes::default();
        for _ in 0..frames {
            let out = window(ctx, app, size, vec![], false);
            nodes.take(&out);
        }
        nodes
    }

    fn press(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        }
    }

    fn click(ctx: &egui::Context, app: &mut QuestApp, size: Vec2, at: Pos2) -> Nodes {
        let mut nodes = Nodes::default();
        let out = window(
            ctx,
            app,
            size,
            vec![Event::PointerMoved(at), press(at, true)],
            false,
        );
        nodes.take(&out);
        let out = window(ctx, app, size, vec![press(at, false)], false);
        nodes.take(&out);
        nodes
    }

    fn flatten<'a>(shape: &'a Shape, into: &mut Vec<&'a Shape>) {
        if let Shape::Vec(list) = shape {
            for s in list {
                flatten(s, into);
            }
        } else {
            into.push(shape);
        }
    }

    /// Every painted text and where its galley lands
    fn painted_texts(out: &egui::FullOutput) -> Vec<(String, Rect)> {
        let mut flat = Vec::new();
        for ClippedShape { shape, .. } in &out.shapes {
            flatten(shape, &mut flat);
        }
        flat.into_iter()
            .filter_map(|s| match s {
                Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.rect.translate(t.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    /// Whether anything inside `area` was painted in exactly `color`
    fn painted_in(
        ctx: &egui::Context,
        out: egui::FullOutput,
        area: Rect,
        color: egui::Color32,
    ) -> bool {
        ctx.tessellate(out.shapes, out.pixels_per_point)
            .into_iter()
            .filter_map(|p| match p.primitive {
                Primitive::Mesh(mesh) => Some(mesh),
                Primitive::Callback(_) => None,
            })
            .any(|mesh| {
                mesh.vertices
                    .iter()
                    .any(|v| v.color == color && area.contains(v.pos))
            })
    }

    /// Nothing picked, search empty, sandbox games only so nothing reads the real Steam
    fn plain(tmp: &std::path::Path) -> QuestApp {
        let mut app = app_in(tmp);
        app.catalog.games = games(&["Alpha", "Beta", "Gamma", "Delta"]);
        app.query.clear();
        app.results.clear();
        app.selected = None;
        app
    }

    // ---------- danger_button hover ----------

    #[test]
    fn a_danger_button_turns_red_while_hovered() {
        let ctx = test_ctx();
        let run = |events: Vec<Event>, time: f64| {
            let input = RawInput {
                events,
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))),
                ..Default::default()
            };
            let mut rect = Rect::NOTHING;
            let mut hovered = false;
            let mut ids = (egui::Id::NULL, egui::Id::NULL);
            let mut out = ctx.run_ui(input, |ui| {
                ui.add_space(40.0);
                ui.horizontal(|ui| {
                    ui.add_space(40.0);
                    let probe = ui.next_auto_id();
                    let r = widgets::danger_button(ui, "Remove");
                    rect = r.rect;
                    hovered = r.hovered();
                    ids = (probe, r.id);
                });
            });
            out.textures_delta.clear();
            (out, rect, hovered, ids)
        };
        let (_, rect, _, _) = run(vec![], 0.0);
        let at = rect.center();
        let mut last = run(vec![Event::PointerMoved(at)], 0.1);
        for step in 2..5 {
            last = run(vec![], f64::from(step) * 0.1);
        }
        let (out, rect, hovered, (probe, button)) = last;
        assert!(hovered, "the pointer isn't over the button at {rect:?}");
        let seen = ctx.read_response(probe).is_some();

        // Control, so a miss below is the button and not the detector
        let control = {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))),
                ..Default::default()
            };
            let mut r = Rect::NOTHING;
            let mut out = ctx.run_ui(input, |ui| {
                r = ui
                    .add(
                        egui::Button::new(RichText::new("Remove").color(theme::BAD))
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0, theme::BAD)),
                    )
                    .rect;
            });
            out.textures_delta.clear();
            painted_in(&ctx, out, r.expand(1.0), theme::BAD)
        };
        assert!(control, "the detector can't see a red button");
        assert!(
            painted_in(&ctx, out, rect.expand(1.0), theme::BAD),
            "hovered for four frames and still not red: it reads the response of \
             ui.next_auto_id() {probe:?} (found last frame: {seen}) but the button made inside \
             ui.scope has id {button:?}"
        );
    }

    #[test]
    fn remove_in_the_queue_turns_red_while_hovered() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        app.queue.push(sandbox("Game B"));
        app.queue.push(sandbox("Game C"));
        let ctx = test_ctx();
        let size = vec2(760.0, 700.0);
        let nodes = settle(&ctx, &mut app, size, 3);
        let remove = nodes.all("Remove")[0];
        window(
            &ctx,
            &mut app,
            size,
            vec![Event::PointerMoved(remove.center())],
            false,
        );
        window(&ctx, &mut app, size, vec![], false);
        let out = window(&ctx, &mut app, size, vec![], false);
        assert!(
            painted_in(&ctx, out, remove.expand(1.0), theme::BAD),
            "Remove at {remove:?} stays grey under the pointer"
        );
    }

    // ---------- breakpoints ----------

    fn layout_of(width: f32) -> &'static str {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        let ctx = test_ctx();
        let nodes = settle(&ctx, &mut app, vec2(width, 700.0), 3);
        let (find, game, tonight) = (
            nodes.one("Find a Game"),
            nodes.one("Game"),
            nodes.one("Tonight"),
        );
        let same_top = |a: Rect, b: Rect| (a.top() - b.top()).abs() < 1.0;
        if same_top(find, game) && same_top(game, tonight) {
            "three"
        } else if same_top(find, tonight) && game.top() > find.bottom() {
            "two"
        } else if find.bottom() <= game.top() && game.bottom() <= tonight.top() {
            "one"
        } else {
            "broken"
        }
    }

    #[test]
    fn layouts_switch_exactly_at_the_breakpoints() {
        let got: Vec<(f32, &str)> = [520.0, 639.0, 639.9, 640.0, 899.0, 899.9, 900.0]
            .into_iter()
            .map(|w| (w, layout_of(w)))
            .collect();
        assert_eq!(
            got,
            [
                (520.0, "one"),
                (639.0, "one"),
                (639.9, "one"),
                (640.0, "two"),
                (899.0, "two"),
                (899.9, "two"),
                (900.0, "three"),
            ]
        );
    }

    // ---------- the game card in the narrow middle panel ----------

    fn steam_card(tmp: &std::path::Path, exe: &str) -> QuestApp {
        let mut app = app_in(tmp);
        app.catalog.games = vec![Game {
            name: "Marathon".into(),
            aliases: Vec::new(),
            exes: vec![exe.into()],
            steam: vec![3_065_800],
        }];
        app.query.clear();
        app.results.clear();
        app.selected = Some(0);
        app.pick = Pick {
            steam_mode: true,
            steam: Some(Ok(SteamPick {
                folder: GameFolder {
                    app_id: 3_065_800,
                    folder: PathBuf::from(r"C:\No Such Library\steamapps\common\Marathon"),
                    installed: false,
                },
                options: vec![LaunchOption {
                    exe: ExePath::parse("marathon.exe").unwrap(),
                    label: "Play".into(),
                }],
                note: None,
            })),
            launch: 1,
            ..Pick::default()
        };
        app
    }

    #[test]
    fn every_info_icon_on_the_card_is_inside_the_game_panel_just_past_900() {
        let mut cut = Vec::new();
        for width in [900.0_f32, 920.0, 960.0, 1000.0, 1080.0] {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = steam_card(tmp.path(), "bin/marathon.exe");
            let ctx = test_ctx();
            settle(&ctx, &mut app, vec2(width, 900.0), 3);
            let (find, game) = three_panel_widths(width);
            let panel_right = find + 1.0 + game;
            for key in [
                "Executable",
                "Run from Steam Library",
                "Launch Option",
                "Custom Path",
                "Minutes",
            ] {
                let found = ctx.read_response(egui::Id::new(("info", key)));
                assert!(found.is_some(), "no ⓘ for {key} at {width}");
                let rect = found.map_or(Rect::NOTHING, |r| r.rect);
                let content = panel_right - widgets::PANEL_PAD;
                if rect.right() > content + 0.5 {
                    let clipped = if rect.right() > panel_right + 0.5 {
                        "clipped by the seam"
                    } else {
                        "in the padding"
                    };
                    cut.push(format!(
                        "{width} px: {key} ⓘ ends at {:.1}, content at {content:.1}, panel at {panel_right:.1}, {clipped}",
                        rect.right()
                    ));
                }
            }
        }
        assert!(cut.is_empty(), "{cut:#?}");
    }

    /// A row's text that wraps past its 36 px row paints over the rows beside it
    fn overflowing_rows(out: &egui::FullOutput, nodes: &Nodes, names: &[&str]) -> Vec<String> {
        let texts = painted_texts(out);
        let mut bad = Vec::new();
        for name in names {
            for row in nodes.all(name) {
                if (row.height() - widgets::ROW_HEIGHT).abs() > 0.5 {
                    continue;
                }
                for (_, text) in texts
                    .iter()
                    .filter(|(t, r)| t == name && row.intersects(*r))
                {
                    if !row.expand(0.5).contains_rect(*text) {
                        bad.push(format!(
                            "{name:?}: text {text:?} spills out of its row {row:?}"
                        ));
                    }
                }
            }
        }
        bad
    }

    #[test]
    fn long_names_in_the_search_results_stay_inside_their_rows() {
        let skyrim = "The Elder Scrolls V: Skyrim Special Edition";
        let real = [
            skyrim,
            "Warhammer 40,000: Darktide - Imperial Edition",
            "The Witcher 3: Wild Hunt - Complete Edition",
            "Microsoft Flight Simulator 2024 - Standard Edition",
            "Tom Clancy's Ghost Recon Breakpoint",
            LONG,
        ];
        let mut bad = Vec::new();
        for width in [520.0_f32, 640.0, 900.0, 1080.0] {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = plain(tmp.path());
            let mut list = games(&["Alpha"]);
            for name in real {
                list.push(Game {
                    name: name.into(),
                    aliases: Vec::new(),
                    exes: vec!["bin/game.exe".into()],
                    steam: Vec::new(),
                });
            }
            app.catalog.games = list;
            app.query = "e".into();
            app.results = (0..app.catalog.games.len()).collect();
            let ctx = test_ctx();
            let size = vec2(width, 900.0);
            let mut nodes = settle(&ctx, &mut app, size, 2);
            let out = window(&ctx, &mut app, size, vec![], false);
            nodes.take(&out);
            for line in overflowing_rows(&out, &nodes, &real) {
                bad.push(format!("{width} px: {line}"));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn a_long_exe_on_the_card_stays_inside_its_row() {
        let mut bad = Vec::new();
        for width in [900.0_f32, 1080.0] {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = plain(tmp.path());
            app.catalog.games = vec![Game {
                name: "Alpha".into(),
                aliases: Vec::new(),
                exes: vec![LONG_EXE.into(), "bin/alpha.exe".into()],
                steam: Vec::new(),
            }];
            app.select(0);
            let ctx = test_ctx();
            let size = vec2(width, 900.0);
            let mut nodes = settle(&ctx, &mut app, size, 2);
            let out = window(&ctx, &mut app, size, vec![], false);
            nodes.take(&out);
            for line in overflowing_rows(&out, &nodes, &[LONG_EXE]) {
                bad.push(format!("{width} px: {line}"));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    // ---------- the strip ----------

    /// Step boxes of the last frame, told from the 14 px rails and the wider row uis by width, and
    /// a one-step row's ui folded into its step by rect
    fn step_rects(ctx: &egui::Context) -> Vec<(String, Rect)> {
        let mut rects: Vec<Rect> = ctx.viewport(|vp| {
            vp.this_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| {
                    (w.rect.height() - widgets::STEP_HEIGHT).abs() < 0.5
                        && w.rect.width() > 30.0
                        && w.rect.width() <= 190.5
                })
                .map(|w| w.rect)
                .collect()
        });
        rects.dedup_by(|a, b| a == b);
        rects.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        rects.dedup();
        rects.into_iter().map(|r| (format!("{r:?}"), r)).collect()
    }

    #[test]
    fn a_long_strip_wraps_inside_tonight_without_overlaps_at_every_width() {
        let mut problems = Vec::new();
        for width in [520.0_f32, 639.0, 640.0, 760.0, 899.0, 900.0, 1080.0, 1600.0] {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = plain(tmp.path());
            for i in 0..MAX_DONE {
                app.done.push(Done {
                    game: format!("{LONG} {i}"),
                    played: Duration::from_secs(16 * 60),
                    full: true,
                });
            }
            app.running.push(busy("Running Now"));
            for i in 0..8 {
                app.queue.push(sandbox(&format!("Queued {i} {LONG}")));
            }
            let ctx = test_ctx();
            settle(&ctx, &mut app, vec2(width, 700.0), 4);
            app.stop_all();
            let steps = step_rects(&ctx);
            if steps.len() != MAX_DONE + 9 {
                problems.push(format!("{width} px: {} steps drawn", steps.len()));
            }
            let right = width - widgets::PANEL_PAD + 0.5;
            for (i, (a_name, a)) in steps.iter().enumerate() {
                if a.right() > right || a.width() < 40.0 {
                    problems.push(format!("{width} px: {a_name} at {a:?}"));
                }
                for (b_name, b) in steps.iter().skip(i + 1) {
                    if a.shrink(0.5).intersects(*b) {
                        problems.push(format!("{width} px: {a_name} overlaps {b_name}"));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn one_step_and_no_steps_draw_without_trouble() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        let ctx = test_ctx();
        settle(&ctx, &mut app, vec2(1080.0, 700.0), 3);
        let none = step_rects(&ctx);
        assert_eq!(none.len(), 0, "{none:?}");
        assert!(app.readout().is_none());
        app.done.push(Done {
            game: "Alpha".into(),
            played: Duration::from_secs(5),
            full: true,
        });
        let nodes = settle(&ctx, &mut app, vec2(1080.0, 700.0), 3);
        let steps = step_rects(&ctx);
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert!(nodes.has("Alpha, 0:05 played"));
        assert_eq!(app.readout().map(|r| r.0), Some("All done".into()));
    }

    // ---------- readout ----------

    #[test]
    fn the_readout_counts_every_queued_minute_past_an_hour() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        for name in ["A", "B", "C", "D", "E"] {
            let mut item = sandbox(name);
            item.minutes = MAX_MINUTES;
            app.queue.push(item);
        }
        assert_eq!(app.readout().unwrap().0, "20:00:00 queued");
        app.running.push(busy("Now"));
        let text = app.readout().unwrap().0;
        app.stop_all();
        // 60 s left plus 20 h, seconds floored
        assert!(
            text == "Ends in 20:01:00" || text.starts_with("Ends in 20:00:5"),
            "{text}"
        );
    }

    #[test]
    fn the_done_cap_drops_the_oldest_when_many_end_in_one_tick() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        for i in 0..MAX_DONE {
            app.done.push(Done {
                game: format!("Old {i}"),
                played: Duration::from_secs(1),
                full: true,
            });
        }
        for name in ["New A", "New B", "New C"] {
            app.running.push(busy(name));
        }
        for run in &mut app.running {
            run.stop();
        }
        app.tick();
        app.stop_all();
        let names: Vec<&str> = app.done.iter().map(|d| d.game.as_str()).collect();
        assert_eq!(names.len(), MAX_DONE);
        assert_eq!(names[0], "Old 3");
        assert_eq!(&names[MAX_DONE - 3..], ["New A", "New B", "New C"]);
    }

    // ---------- status strip and the blocked page ----------

    #[test]
    fn a_long_status_wraps_inside_the_strip_at_520() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        let text = format!(
            "{LONG} was stopped with 12:34 left, and its files are cleaned up. {LONG} from the queue is running for 240 min."
        );
        app.status = Status::good(text.clone());
        let ctx = test_ctx();
        let nodes = settle(&ctx, &mut app, vec2(520.0, 480.0), 3);
        let label = nodes.one(&text);
        assert!(
            label.right() <= 520.0 - 18.0 + 0.5 && label.bottom() <= 480.0 + 0.5,
            "the status runs off the window: {label:?}"
        );
        assert!(label.height() > 30.0, "it didn't wrap: {label:?}");
        let tonight = nodes.one("Find a Game");
        assert!(tonight.top() >= -0.5);
    }

    #[test]
    fn the_blocked_page_shows_only_its_reason_and_wraps_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        let why = format!(
            "The app's data folder couldn't be made: {}.",
            "C:\\Users\\Ünïcødé Name\\AppData\\Local\\Discord Quest Assistant".repeat(3)
        );
        app.blocked = Some(why.clone());
        let ctx = test_ctx();
        let nodes = settle(&ctx, &mut app, vec2(520.0, 480.0), 3);
        assert!(nodes.has("Can't Start"));
        assert!(!nodes.has("Find a Game") && !nodes.has("Tonight"));
        let reason = nodes.one(&why);
        assert!(reason.right() <= 520.0 + 0.5, "{reason:?}");
    }

    // ---------- results list height in three panels ----------

    #[test]
    fn the_last_result_can_be_scrolled_fully_into_view_at_900_by_480() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        let names: Vec<String> = (0..MAX_RESULTS).map(|i| format!("Game {i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        app.catalog.games = games(&refs);
        app.query = "game".into();
        app.results = (0..MAX_RESULTS).collect();
        let ctx = test_ctx();
        let size = vec2(900.0, 480.0);
        let mut nodes = settle(&ctx, &mut app, size, 3);
        let first = nodes.one("Game 00");
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -400.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::default(),
        };
        window(
            &ctx,
            &mut app,
            size,
            vec![Event::PointerMoved(first.center())],
            false,
        );
        for _ in 0..30 {
            let out = window(&ctx, &mut app, size, vec![wheel.clone()], false);
            nodes.take(&out);
        }
        for _ in 0..8 {
            let out = window(&ctx, &mut app, size, vec![], false);
            nodes.take(&out);
        }
        let last_name = names.last().unwrap();
        let last = nodes.one(last_name);
        let status = nodes.one("Ready.");
        assert!(
            last.bottom() <= status.top() - 10.0 + 0.5,
            "scrolled to the end, the last result {last:?} still ends under the status strip at {status:?}"
        );
    }

    // ---------- Clear Queue ----------

    #[test]
    fn clear_queue_through_the_window_asks_then_clears_from_the_dialog() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        for g in ["Game B", "Game C", "Game D"] {
            app.queue.push(sandbox(g));
        }
        let ctx = test_ctx();
        let size = vec2(760.0, 700.0);
        let nodes = settle(&ctx, &mut app, size, 3);
        let main = nodes.one("Clear Queue");
        click(&ctx, &mut app, size, main.center());
        assert!(
            app.confirm_clear,
            "Clear Queue with three waiting didn't ask"
        );
        assert_eq!(app.queue.len(), 3);
        let nodes = settle(&ctx, &mut app, size, 3);
        assert!(nodes.has("Clear the Queue?"));
        assert!(nodes.has(
            "3 waiting games are taken out. None of them has started, so there's nothing to clean up."
        ));
        let in_dialog = nodes
            .all("Clear Queue")
            .into_iter()
            .find(|r| (r.center() - main.center()).length() > 2.0)
            .expect("no Clear Queue in the dialog");
        click(&ctx, &mut app, size, in_dialog.center());
        window(&ctx, &mut app, size, vec![], false);
        assert!(
            app.queue.is_empty(),
            "the dialog's Clear Queue didn't clear"
        );
        assert!(!app.confirm_clear);
        assert_eq!(app.status.text, "The queue is cleared.");
    }

    #[test]
    fn keep_queue_in_the_clear_dialog_keeps_it_and_escape_too() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        for g in ["Game B", "Game C"] {
            app.queue.push(sandbox(g));
        }
        app.confirm_clear = true;
        let ctx = test_ctx();
        let size = vec2(760.0, 700.0);
        let nodes = settle(&ctx, &mut app, size, 3);
        click(&ctx, &mut app, size, nodes.one("Keep Queue").center());
        assert!(!app.confirm_clear);
        assert_eq!(app.queue.len(), 2);

        app.confirm_clear = true;
        settle(&ctx, &mut app, size, 3);
        let escape = |pressed| Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::default(),
        };
        window(&ctx, &mut app, size, vec![escape(true)], false);
        window(&ctx, &mut app, size, vec![escape(false)], false);
        assert!(!app.confirm_clear, "Escape didn't close the dialog");
        assert_eq!(app.queue.len(), 2, "Escape cleared the queue");
    }

    #[test]
    fn clearing_the_open_cards_queued_game_puts_the_card_back_to_start() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        app.queue.push(Queued {
            game: "Beta".into(),
            minutes: 30,
            target: Target::Sandbox(ExePath::parse("bin/beta.exe").unwrap()),
        });
        app.act_on_queue(QueueClicks {
            edit: Some("Beta".into()),
            ..QueueClicks::default()
        });
        assert_eq!(app.selected, Some(1));
        let ctx = test_ctx();
        let size = vec2(1080.0, 700.0);
        let nodes = settle(&ctx, &mut app, size, 2);
        assert!(nodes.has("Changes are saved to its place in the queue."));
        app.act_on_queue(QueueClicks {
            clear: true,
            ..QueueClicks::default()
        });
        assert!(app.queue.is_empty());
        let nodes = settle(&ctx, &mut app, size, 3);
        assert!(nodes.has("Start"), "the card has no Start after the clear");
        app.minutes = 45.0;
        settle(&ctx, &mut app, size, 2);
        assert!(
            app.queue.is_empty(),
            "editing the card after the clear queued it again"
        );
        assert!(app.pick.saved.is_none());
    }

    #[test]
    fn clear_queue_pressed_as_the_head_starts_does_nothing_wrong() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        app.running.push(busy("Game A"));
        app.queue.push(sandbox("Game B"));
        app.queue.push(sandbox("Game C"));
        let ctx = test_ctx();
        let size = vec2(760.0, 700.0);
        let nodes = settle(&ctx, &mut app, size, 3);
        let clear = nodes.one("Clear Queue");
        // A ends just before the press, so the frame that takes it starts B first
        app.running[0].stop();
        app.tick();
        assert_eq!(app.queue.len(), 1);
        click(&ctx, &mut app, size, clear.center());
        let state = (
            app.queue.iter().map(|q| q.game.clone()).collect::<Vec<_>>(),
            app.running
                .iter()
                .map(|r| r.game.clone())
                .collect::<Vec<_>>(),
            app.confirm_clear,
        );
        app.stop_all();
        assert!(!state.2, "a dialog opened for one waiting game");
        assert_eq!(state.1, ["Game B"], "the running game changed");
        assert!(state.0.is_empty() || state.0 == ["Game C"], "{:?}", state.0);
    }

    #[test]
    fn the_clear_and_quit_dialogs_asked_in_one_frame_close_one_at_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        for g in ["Game B", "Game C"] {
            app.queue.push(sandbox(g));
        }
        app.confirm_quit = true;
        app.confirm_clear = true;
        let ctx = test_ctx();
        let size = vec2(760.0, 700.0);
        let nodes = settle(&ctx, &mut app, size, 3);
        assert!(nodes.has("Quit?") && nodes.has("Clear the Queue?"));
        let escape = |pressed| Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::default(),
        };
        window(&ctx, &mut app, size, vec![escape(true)], false);
        window(&ctx, &mut app, size, vec![escape(false)], false);
        assert_eq!(
            u8::from(app.confirm_quit) + u8::from(app.confirm_clear),
            1,
            "one Escape closed quit={} clear={}",
            !app.confirm_quit,
            !app.confirm_clear
        );
        assert_eq!(app.queue.len(), 2);
        assert!(!app.quitting);
    }

    // ---------- keyboard ----------

    fn tab(ctx: &egui::Context, app: &mut QuestApp, size: Vec2, shift: bool) {
        let modifiers = if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::default()
        };
        for pressed in [true, false] {
            let event = Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            };
            window(ctx, app, size, vec![event], false);
        }
    }

    fn focused_rect(ctx: &egui::Context) -> Option<Rect> {
        ctx.memory(egui::Memory::focused)
            .and_then(|id| ctx.read_response(id))
            .map(|r| r.rect)
    }

    #[test]
    fn tab_walks_find_then_game_then_tonight_in_three_panels() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = evening(tmp.path());
        let ctx = test_ctx();
        let width = 1080.0;
        let size = vec2(width, 3000.0);
        app.focused_search = false;
        settle(&ctx, &mut app, size, 3);
        let panel = |r: Rect| {
            if r.center().x < width * 0.33 {
                0
            } else if r.center().x < width * 0.61 + 1.0 {
                1
            } else {
                2
            }
        };
        let mut order = Vec::new();
        for _ in 0..60 {
            if let Some(r) = focused_rect(&ctx) {
                order.push((panel(r), r));
            }
            tab(&ctx, &mut app, size, false);
        }
        app.stop_all();
        let panels: Vec<u8> = order.iter().map(|(p, _)| *p).collect();
        assert!(panels.contains(&2), "Tab never reached Tonight: {panels:?}");
        let first_wrap = panels
            .windows(2)
            .position(|w| w[1] < w[0])
            .map_or(panels.len(), |i| i + 1);
        let lap = &panels[..first_wrap];
        assert!(
            lap.first() == Some(&0) && lap.last() == Some(&2),
            "one lap of Tab should start in Find and end in Tonight: {panels:?}"
        );
    }

    #[test]
    fn tab_stays_inside_the_clear_dialog() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = plain(tmp.path());
        for g in ["Game B", "Game C"] {
            app.queue.push(sandbox(g));
        }
        app.confirm_clear = true;
        let ctx = test_ctx();
        let size = vec2(760.0, 700.0);
        settle(&ctx, &mut app, size, 3);
        let area = ctx
            .memory(|m| m.area_rect(egui::Id::new("confirm-clear")))
            .expect("no dialog area")
            .expand(0.5);
        let mut outside = Vec::new();
        for _ in 0..8 {
            tab(&ctx, &mut app, size, false);
            if let Some(r) = focused_rect(&ctx)
                && !area.contains_rect(r)
            {
                outside.push(r);
            }
        }
        assert!(outside.is_empty(), "Tab left the dialog for {outside:?}");
        assert!(app.confirm_clear && app.queue.len() == 2);
    }

    // ---------- the play window's title bar ----------

    #[test]
    fn a_200_character_title_never_runs_under_the_time_left() {
        let ctx = test_ctx();
        let title = "W".repeat(200);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(460.0, 196.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            widgets::heading_bar(ui, &title, Some(("3:59:59 left", theme::ACCENT)));
        });
        out.textures_delta.clear();
        let texts = painted_texts(&out);
        let left = texts
            .iter()
            .find(|(t, _)| t == "3:59:59 left")
            .map(|(_, r)| *r)
            .expect("no readout painted");
        let name = texts
            .iter()
            .find(|(t, _)| t.starts_with("WWW"))
            .map(|(_, r)| *r)
            .expect("no title painted");
        assert!(left.right() <= 460.0 - widgets::PANEL_PAD + 0.5, "{left:?}");
        assert!(
            name.right() <= left.left() + 0.5,
            "title {name:?} runs under the readout {left:?}"
        );
    }

    // ---------- every control apart, at and around the breakpoints ----------

    fn evening(tmp: &std::path::Path) -> QuestApp {
        let mut app = plain(tmp);
        let mut list = games(&["Alpha", "Beta", "Gamma", "Delta", "Epsilon"]);
        list.push(Game {
            name: LONG.into(),
            aliases: Vec::new(),
            exes: vec![LONG_EXE.into()],
            steam: Vec::new(),
        });
        app.catalog.games = list;
        app.query = "a".into();
        app.results = (0..app.catalog.games.len()).collect();
        for name in ["Alpha", LONG] {
            app.done.push(Done {
                game: name.into(),
                played: Duration::from_secs(16 * 60),
                full: true,
            });
        }
        app.running.push(busy(LONG));
        for name in ["Beta", "Gamma", "Delta"] {
            app.queue.push(sandbox(name));
        }
        app.leftovers.push("s-missing".into());
        app.select(1);
        app
    }

    #[test]
    fn no_two_controls_overlap_at_or_around_any_breakpoint() {
        let mut problems = Vec::new();
        for (width, height) in [
            (520.0_f32, 480.0_f32),
            (639.0, 480.0),
            (640.0, 480.0),
            (899.0, 480.0),
            (900.0, 480.0),
            (900.0, 700.0),
            (1080.0, 700.0),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = evening(tmp.path());
            let ctx = test_ctx();
            settle(&ctx, &mut app, vec2(width, height), 4);
            let rects: Vec<Rect> = ctx.viewport(|vp| {
                vp.this_pass
                    .widgets
                    .layers()
                    .flat_map(|(_, w)| w.iter())
                    .filter(|w| w.sense.senses_click() && w.enabled)
                    .map(|w| w.interact_rect)
                    .filter(Rect::is_positive)
                    .collect()
            });
            app.stop_all();
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, height));
            for (i, a) in rects.iter().enumerate() {
                if !screen.expand(0.5).contains_rect(*a) {
                    problems.push(format!("{width}x{height}: {a:?} off the window"));
                }
                for b in rects.iter().skip(i + 1) {
                    if a.shrink(0.5).intersects(*b) && !a.contains_rect(*b) && !b.contains_rect(*a)
                    {
                        problems.push(format!("{width}x{height}: {a:?} overlaps {b:?}"));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }
}
