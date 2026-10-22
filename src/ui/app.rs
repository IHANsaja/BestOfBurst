//! App state, settings, message plumbing between the UI thread and the engine/background
//! threads, and the shared chrome bar (DESIGN.md 4.3).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, Rect, Role, Sense, Ui, WidgetInfo};
use serde::{Deserialize, Serialize};

use super::review::Review;
use super::textures::Textures;
use super::theme::{self, Pal};
use super::{compare, corner, done, focus, help, home, marks, progress, review, viewer};
use crate::engine::EngineChoice;
use crate::sysmon::{self, Monitor, Snapshot};
use crate::engine::{self, Event, RunConfig};
use crate::output::OutputSummary;
use crate::rank::{Keep, Preset, RankSummary};
use crate::scan;
use crate::state::State;

/// A safelight break is suggested after this much continuous review.
pub const BREAK_AFTER: Duration = Duration::from_secs(20 * 60);
pub const HEADER_H: f32 = 56.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    Progress,
    Review,
    Focus,
    Compare,
    Viewer,
    Done,
    Corner,
    Help,
}

pub enum EngineMsg {
    Scanned { total: usize, new: usize },
    Progress { done: usize, total: usize, per_sec: f32, eta_s: Option<u32>, last: Option<PathBuf>, last_thumb: Option<PathBuf> },
    Failed { path: PathBuf, error: String },
    Warning(String),
    Ranked(RankSummary),
    // SPEC-GAP: the finished state travels back to the UI so Review opens without re-reading disk.
    Finished(Box<State>),
    Stopped,
    Error(String),
}

/// Results of work the UI hands to background threads (it never touches disk itself).
pub enum BgMsg {
    FolderInfo { folder: PathBuf, info: FolderInfo },
    StateLoaded(Result<Box<State>, String>),
    Written(Result<OutputSummary, String>),
}

#[derive(Clone, Debug, Default)]
pub struct FolderInfo {
    pub total: usize,
    pub analyzed: usize,
    pub failed: usize,
    pub new: usize,
    pub has_rank: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct ProgressInfo {
    pub scanning: bool,
    pub total_files: usize,
    pub done: usize,
    pub total: usize,
    pub per_sec: f32,
    pub eta_s: Option<u32>,
    pub last: Option<PathBuf>,
    pub recent_thumbs: VecDeque<PathBuf>,
    pub failures: Vec<(PathBuf, String)>,
    pub failed: usize,
    pub missing_codec: usize,
    pub warnings: Vec<String>,
    pub ranking: bool,
    pub finished: bool,
    pub summary: Option<RankSummary>,
    pub stopping: bool,
    pub started: Option<Instant>,
}

/// Small per-user preferences and Glim's state, in %APPDATA%\bestofburst\settings.json.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub night: bool,
    pub engine: EngineChoice,
    pub treats: u32,
    pub joy: f32,
    pub energy: f32,
    pub settled: u32,
    pub sharp_eye_best: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { night: false, engine: EngineChoice::Auto, treats: 0, joy: 60.0, energy: 70.0, settled: 0, sharp_eye_best: 0 }
    }
}

fn app_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("bestofburst"))
}

fn load_json<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    app_dir().and_then(|d| std::fs::read(d.join(name)).ok()).and_then(|d| serde_json::from_slice(&d).ok())
}

fn save_json<T: Serialize + Send + 'static>(name: &'static str, value: T) {
    std::thread::spawn(move || {
        let Some(dir) = app_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(data) = serde_json::to_vec_pretty(&value) {
            let tmp = dir.join(format!("{name}.tmp"));
            if std::fs::write(&tmp, data).is_ok() {
                let _ = std::fs::rename(tmp, dir.join(name));
            }
        }
    });
}

pub struct BestOfBurstApp {
    pub screen: Screen,
    pub folder: Option<PathBuf>,
    pub keep_text: String,
    pub preset: Preset,
    pub recent: Vec<PathBuf>,
    pub folder_info: Option<FolderInfo>,
    pub checking_folder: bool,
    pub engine_rx: Option<Receiver<EngineMsg>>,
    pub stop: Arc<AtomicBool>,
    pub threads: usize,
    pub retry_failed: bool,
    pub bg_tx: Sender<BgMsg>,
    bg_rx: Receiver<BgMsg>,
    pub prog: ProgressInfo,
    pub finished_state: Option<Box<State>>,
    pub textures: Textures,
    pub review: Option<Review>,
    pub message: Option<(String, bool, Instant)>,
    pub saver: Saver,
    pub settings: Settings,
    saved_settings: Settings,
    pub reduced_motion: bool,
    started: Instant,
    /// Start of the current continuous review stretch.
    pub review_clock: Option<Instant>,
    pub corner_return: Screen,
    pub corner: corner::CornerState,
    pub last_delivery: Option<OutputSummary>,
    pub help_return: Screen,
    pub monitor: Monitor,
    /// Whether the window was last told to fill the monitor (the viewer and full-screen compare).
    fullscreen: bool,
}

/// Writes `state.json` on its own thread, always the newest snapshot.
pub struct Saver {
    tx: Sender<(State, PathBuf)>,
}

impl Saver {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel::<(State, PathBuf)>();
        let _ = std::thread::Builder::new().name("bob-saver".into()).spawn(move || {
            while let Ok(mut job) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    job = newer;
                }
                let (mut state, out) = job;
                let _ = state.save(&out);
            }
        });
        Saver { tx }
    }
    pub fn save(&self, state: State, out: PathBuf) {
        let _ = self.tx.send((state, out));
    }
}

impl BestOfBurstApp {
    pub fn new(cc: &eframe::CreationContext<'_>, folder: Option<PathBuf>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let settings: Settings = load_json("settings.json").unwrap_or_default();
        theme::apply_visuals(&cc.egui_ctx, theme::pal(settings.night));
        let recent: Vec<PathBuf> = load_json::<Vec<String>>("recent.json")
            .map(|v| v.into_iter().map(PathBuf::from).collect())
            .unwrap_or_default();
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut app = BestOfBurstApp {
            screen: Screen::Home,
            folder: None,
            keep_text: "10%".into(),
            preset: Preset::General,
            recent,
            folder_info: None,
            checking_folder: false,
            engine_rx: None,
            stop: Arc::new(AtomicBool::new(false)),
            threads: engine::default_threads(),
            retry_failed: false,
            bg_tx,
            bg_rx,
            prog: ProgressInfo::default(),
            finished_state: None,
            textures: Textures::new(&cc.egui_ctx),
            review: None,
            message: None,
            saver: Saver::new(),
            saved_settings: settings.clone(),
            settings,
            reduced_motion: theme::reduced_motion(),
            started: Instant::now(),
            review_clock: None,
            corner_return: Screen::Home,
            corner: corner::CornerState::default(),
            last_delivery: None,
            help_return: Screen::Home,
            monitor: Monitor::start(),
            fullscreen: false,
        };
        // Start the graphics card in the background so Home can show it without waiting.
        std::thread::spawn(|| {
            let _ = crate::gpu::get();
        });
        if let Some(f) = folder {
            let f = std::path::absolute(&f).unwrap_or(f);
            app.choose_folder(f, &cc.egui_ctx);
        }
        app
    }

    /// Seconds since start, for Glim's idle loops and mark draw-on.
    pub fn time(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn pal(&self) -> Pal {
        theme::pal(self.settings.night)
    }

    pub fn keep(&self) -> Option<Keep> {
        Keep::parse(&self.keep_text)
    }

    pub fn flash(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some((text.into(), error, Instant::now()));
    }

    pub fn shoot_name(&self) -> String {
        self.folder
            .as_ref()
            .and_then(|f| f.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "No shoot".into())
    }

    pub fn open_help(&mut self) {
        if self.screen != Screen::Help {
            self.help_return = self.screen;
        }
        self.screen = Screen::Help;
    }

    pub fn sys(&self) -> Snapshot {
        self.monitor.get()
    }

    pub fn open_corner(&mut self) {
        if self.screen != Screen::Corner {
            self.corner_return = self.screen;
        }
        self.screen = Screen::Corner;
    }

    /// A folder was picked: remember it and count what is new, off the UI thread.
    pub fn choose_folder(&mut self, folder: PathBuf, ctx: &egui::Context) {
        self.recent.retain(|p| p != &folder);
        self.recent.insert(0, folder.clone());
        self.recent.truncate(10);
        save_json("recent.json", self.recent.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>());
        if self.folder.as_ref() != Some(&folder) {
            self.textures.clear();
            self.review = None;
        }
        self.folder = Some(folder.clone());
        self.folder_info = None;
        self.checking_folder = true;
        let tx = self.bg_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let info = folder_info(&folder);
            let _ = tx.send(BgMsg::FolderInfo { folder, info });
            ctx.request_repaint();
        });
    }

    /// Start (or resume) analysis on the engine thread.
    pub fn start(&mut self, ctx: &egui::Context) {
        let Some(folder) = self.folder.clone() else { return };
        let Some(keep) = self.keep() else {
            self.flash("Photos to keep needs a number (like 300) or a share (like 10%).", true);
            return;
        };
        let mut cfg = RunConfig::new(folder);
        cfg.keep = keep;
        cfg.preset = self.preset;
        cfg.threads = self.threads;
        cfg.retry_failed = self.retry_failed;
        cfg.engine = self.settings.engine;
        self.stop = Arc::new(AtomicBool::new(false));
        self.prog = ProgressInfo { scanning: true, started: Some(Instant::now()), ..Default::default() };
        self.finished_state = None;
        let (tx, rx) = mpsc::channel();
        self.engine_rx = Some(rx);
        self.screen = Screen::Progress;
        let stop = self.stop.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let send = |m: EngineMsg| {
                let _ = tx.send(m);
                ctx.request_repaint();
            };
            let res = engine::analyze_folder(&cfg, &stop, |ev| match ev {
                Event::Warning(w) => send(EngineMsg::Warning(w)),
                Event::Engine(e) => send(EngineMsg::Warning(e)),
                Event::Scanned { total, new, .. } => send(EngineMsg::Scanned { total, new }),
                Event::Progress { done, total, per_sec, eta_s, last, last_thumb } => send(EngineMsg::Progress {
                    done,
                    total,
                    per_sec,
                    eta_s,
                    last: last.map(PathBuf::from),
                    last_thumb,
                }),
                Event::Failed { path, error } => send(EngineMsg::Failed { path: PathBuf::from(path), error }),
            });
            match res {
                Err(e) => send(EngineMsg::Error(format!("{e:#}"))),
                Ok(o) if o.stopped => send(EngineMsg::Stopped),
                Ok(o) => {
                    let mut state = o.state;
                    match engine::rank_and_output(&mut state, &cfg) {
                        Ok((ranking, _)) => {
                            send(EngineMsg::Ranked(ranking.summary));
                            send(EngineMsg::Finished(Box::new(state)));
                        }
                        Err(e) => send(EngineMsg::Error(format!("{e:#}"))),
                    }
                }
            }
        });
    }

    /// Open Review from the existing state.json.
    pub fn open_results(&mut self, ctx: &egui::Context) {
        let Some(folder) = self.folder.clone() else { return };
        let out = scan::default_out(&folder);
        let tx = self.bg_tx.clone();
        let ctx = ctx.clone();
        self.checking_folder = true;
        std::thread::spawn(move || {
            let r = match State::load_existing(&folder, &out) {
                Ok(Some((s, _))) => Ok(Box::new(s)),
                Ok(None) => Err("No state.json found.".to_string()),
                Err(e) => Err(format!("{e:#}")),
            };
            let _ = tx.send(BgMsg::StateLoaded(r));
            ctx.request_repaint();
        });
    }

    pub fn request_stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.prog.stopping = true;
    }

    /// Processing finished: open the contact sheet.
    pub fn open_contact_sheet(&mut self) {
        if let Some(s) = self.finished_state.take() {
            self.enter_review(*s);
        }
    }

    fn enter_review(&mut self, state: State) {
        let Some(folder) = self.folder.clone() else { return };
        let keep = self.keep().unwrap_or_default();
        self.review = Some(Review::new(state, folder, keep, self.preset));
        self.screen = Screen::Review;
    }

    pub fn go_home(&mut self, ctx: &egui::Context) {
        if let Some(rv) = &mut self.review {
            rv.flush(&self.saver);
        }
        self.review = None;
        self.review_clock = None;
        self.screen = Screen::Home;
        if let Some(f) = self.folder.clone() {
            self.choose_folder(f, ctx);
        }
    }

    fn on_engine(&mut self, msg: EngineMsg, ctx: &egui::Context) {
        match msg {
            EngineMsg::Scanned { total, new } => {
                self.prog.scanning = false;
                self.prog.total_files = total;
                self.prog.total = new;
            }
            EngineMsg::Progress { done, total, per_sec, eta_s, last, last_thumb } => {
                self.prog.done = done;
                self.prog.total = total;
                self.prog.per_sec = per_sec;
                self.prog.eta_s = eta_s;
                self.prog.last = last;
                if let Some(t) = last_thumb {
                    self.prog.recent_thumbs.push_front(t);
                    self.prog.recent_thumbs.truncate(12);
                }
                if done == total {
                    self.prog.ranking = true;
                }
            }
            EngineMsg::Failed { path, error } => {
                self.prog.failed += 1;
                if crate::decode::wic::is_missing_codec(&error) {
                    self.prog.missing_codec += 1;
                }
                self.prog.failures.push((path, error));
                if self.prog.failures.len() > 200 {
                    self.prog.failures.remove(0);
                }
            }
            EngineMsg::Warning(w) => self.prog.warnings.push(w),
            EngineMsg::Ranked(s) => {
                self.prog.ranking = true;
                self.prog.summary = Some(s);
            }
            EngineMsg::Finished(state) => {
                self.engine_rx = None;
                self.prog.finished = true;
                self.prog.ranking = false;
                self.finished_state = Some(state);
            }
            EngineMsg::Stopped => {
                self.engine_rx = None;
                self.screen = Screen::Home;
                self.flash("Paused and saved. Press Start culling to carry on.", false);
                if let Some(f) = self.folder.clone() {
                    self.choose_folder(f, ctx);
                }
            }
            EngineMsg::Error(e) => {
                self.engine_rx = None;
                self.screen = Screen::Home;
                self.flash(e, true);
            }
        }
    }

    fn on_bg(&mut self, msg: BgMsg) {
        match msg {
            BgMsg::FolderInfo { folder, info } => {
                if self.folder.as_ref() == Some(&folder) {
                    self.checking_folder = false;
                    self.folder_info = Some(info);
                }
            }
            BgMsg::StateLoaded(r) => {
                self.checking_folder = false;
                match r {
                    Ok(s) => self.enter_review(*s),
                    Err(e) => self.flash(e, true),
                }
            }
            BgMsg::Written(r) => {
                if let Some(rv) = &mut self.review {
                    rv.writing = false;
                }
                match r {
                    Ok(s) => {
                        let mut text = format!("Done: {} photos are in {}", s.written, s.picks_dir.display());
                        if s.copied_fallback {
                            text.push_str(" (full copies, since shortcuts aren't possible on this drive)");
                        }
                        if !s.errors.is_empty() {
                            text.push_str(&format!(", {} couldn't be added", s.errors.len()));
                        }
                        self.flash(text, !s.errors.is_empty());
                        self.last_delivery = Some(s);
                    }
                    Err(e) => self.flash(e, true),
                }
            }
        }
    }

    /// Minutes into the current review stretch, when a break is due.
    pub fn break_due(&self) -> Option<u64> {
        let t = self.review_clock?;
        (t.elapsed() >= BREAK_AFTER).then(|| t.elapsed().as_secs() / 60)
    }
}

/// Count photos and how many are already analyzed (runs on a background thread).
fn folder_info(folder: &Path) -> FolderInfo {
    let out = scan::default_out(folder);
    if !folder.is_dir() {
        return FolderInfo { error: Some("Folder not found.".into()), ..Default::default() };
    }
    let state = match State::load_existing(folder, &out) {
        Ok(Some((s, _))) => Some(s),
        Ok(None) => None,
        Err(e) => return FolderInfo { error: Some(format!("{e:#}")), ..Default::default() },
    };
    let items = scan::scan(folder, &out, None);
    let mut info = FolderInfo { total: items.len(), ..Default::default() };
    for i in &items {
        match &state {
            Some(s) if s.is_current(&i.rel, i.size, i.mtime_ms) => {
                if s.files.get(&i.rel).map(|e| e.is_done()).unwrap_or(false) {
                    info.analyzed += 1;
                } else {
                    info.failed += 1;
                }
            }
            _ => info.new += 1,
        }
    }
    info.has_rank = state.as_ref().map(|s| s.last_rank.is_some()).unwrap_or(false);
    info
}

// ---- Chrome bar (4.3): a row of cells separated by 1 px rules ----

/// Asked for Help from a chrome-bar cell this frame.
fn help_flag() -> egui::Id {
    #[allow(deprecated)]
    egui::Id::new("bob_help_request")
}

/// Show the chrome bar, which is also the window's title bar: `left` fills cells from the
/// left, `right` from the right, before Help and the window controls.
pub fn chrome_bar(ui: &mut Ui, id: &str, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let frame = egui::Frame::new().fill(theme::INK);
    egui::Panel::top(id.to_string()).exact_size(HEADER_H).frame(frame).show(ui, |ui| {
        theme::dark_scope(ui);
        ui.spacing_mut().item_spacing.x = 0.0;
        let full = ui.max_rect();
        // Empty parts of the bar move the window; double-click maximizes.
        let drag = ui.interact(full, ui.scope_id().with("titlebar_drag"), Sense::click_and_drag());
        if drag.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let maxed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
        }
        ui.painter().line_segment(
            [pos2(full.min.x, full.max.y - 0.5), pos2(full.max.x, full.max.y - 0.5)],
            egui::Stroke::new(1.0, theme::RULE_DARK),
        );
        ui.horizontal_centered(|ui| {
            left(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                window_controls(ui);
                if help_cell(ui) {
                    ui.ctx().data_mut(|d| d.insert_temp(help_flag(), true));
                }
                right(ui);
            });
        });
    });
}

#[derive(Clone, Copy, PartialEq)]
enum WinButton {
    Minimize,
    Maximize,
    Close,
}

/// Minimize, maximize/restore and close, drawn in the Grease Pencil style.
fn window_controls(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    for b in [WinButton::Close, WinButton::Maximize, WinButton::Minimize] {
        let (rect, resp) = ui.allocate_exact_size(vec2(52.0, HEADER_H), Sense::click());
        let label = match b {
            WinButton::Minimize => "Minimize",
            WinButton::Maximize => if maxed { "Restore" } else { "Maximize" },
            WinButton::Close => "Close",
        };
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
        let hover = resp.hovered();
        if hover {
            let bg = if b == WinButton::Close { theme::RED } else { theme::RULE_DARK };
            ui.painter().rect_filled(rect, 0.0, bg);
        }
        let c = rect.center();
        let col = if hover && b == WinButton::Close { theme::PAPER_WHITE } else { theme::TABLE };
        let s = egui::Stroke::new(1.6, col);
        let p = ui.painter();
        match b {
            WinButton::Minimize => {
                p.line_segment([c + vec2(-6.0, 0.5), c + vec2(6.0, 0.5)], s);
            }
            WinButton::Maximize => {
                if maxed {
                    p.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(9.0, 9.0)), 0.0, s, egui::StrokeKind::Middle);
                    p.line_segment([c + vec2(-3.0, -4.5), c + vec2(6.0, -4.5)], s);
                    p.line_segment([c + vec2(6.0, -4.5), c + vec2(6.0, 4.5)], s);
                } else {
                    p.rect_stroke(Rect::from_center_size(c, vec2(11.0, 11.0)), 0.0, s, egui::StrokeKind::Middle);
                }
            }
            WinButton::Close => {
                p.line_segment([c + vec2(-6.0, -6.0), c + vec2(6.0, 6.0)], s);
                p.line_segment([c + vec2(6.0, -6.0), c + vec2(-6.0, 6.0)], s);
            }
        }
        theme::focus_ring(ui, &resp);
        if resp.clicked() {
            match b {
                WinButton::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                WinButton::Maximize => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed)),
                WinButton::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }
    cell_rule(ui);
}

/// HELP cell with its F1 key cap.
fn help_cell(ui: &mut Ui) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(96.0, HEADER_H), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Help"));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::RULE_DARK);
    }
    theme::text_at(ui, pos2(rect.min.x + 16.0, rect.center().y), Align2::LEFT_CENTER, "HELP", theme::f_mono_semi(10.5), theme::TABLE, 0.84);
    theme::keycap(ui, pos2(rect.max.x - 14.0, rect.center().y), "F1", theme::TABLE);
    theme::focus_ring(ui, &resp);
    cell_rule(ui);
    resp.clicked()
}

/// Resize from the window edges (the window has no system frame).
fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let r = ctx.content_rect();
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    const E: f32 = 5.0;
    let (l, rt, t, b) = (p.x - r.min.x < E, r.max.x - p.x < E, p.y - r.min.y < E, r.max.y - p.y < E);
    use egui::viewport::ResizeDirection as D;
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => Some(D::NorthWest),
        (_, true, true, _) => Some(D::NorthEast),
        (true, _, _, true) => Some(D::SouthWest),
        (_, true, _, true) => Some(D::SouthEast),
        (true, _, _, _) => Some(D::West),
        (_, true, _, _) => Some(D::East),
        (_, _, true, _) => Some(D::North),
        (_, _, _, true) => Some(D::South),
        _ => None,
    };
    let Some(dir) = dir else { return };
    ctx.set_cursor_icon(match dir {
        D::North | D::South => egui::CursorIcon::ResizeVertical,
        D::East | D::West => egui::CursorIcon::ResizeHorizontal,
        D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
        D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
    }
}

/// Vertical 1 px rule between cells.
pub fn cell_rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, HEADER_H), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::RULE_DARK);
}

/// Logo + wordmark cell; clicking goes Home.
pub fn logo_cell(ui: &mut Ui) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(214.0, HEADER_H), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Link, true, "BestOfBurst home"));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::RULE_DARK);
    }
    marks::wordmark(ui, pos2(rect.min.x + 18.0, rect.center().y), theme::TABLE, true);
    theme::focus_ring(ui, &resp);
    cell_rule(ui);
    resp.clicked()
}

/// A text cell in the chrome bar.
pub fn text_cell(ui: &mut Ui, text: &str, big: bool) {
    ui.add_space(20.0);
    if big {
        ui.label(theme::xp(text, 15.0, theme::TABLE));
    } else {
        ui.label(theme::mono(text, 10.5, theme::ASH));
    }
    ui.add_space(20.0);
    cell_rule(ui);
}

/// The full-height grease-red action cell (used in a right-to-left layout).
pub fn red_cell(ui: &mut Ui, label: &str, key: &str, enabled: bool) -> bool {
    let text = label.to_uppercase();
    let g = theme::galley(ui, &text, theme::f_xp(14.0), theme::PAPER_WHITE, 0.5);
    let key_w = 16.0 + 8.0 * key.len() as f32;
    let w = g.size().x + key_w + 52.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, HEADER_H), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let fill = if !enabled {
        theme::RED.gamma_multiply(0.4)
    } else if resp.hovered() {
        theme::RED_HOVER
    } else {
        theme::RED
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter().galley(pos2(rect.min.x + 20.0, rect.center().y - g.size().y / 2.0), g, theme::PAPER_WHITE);
    theme::keycap(ui, pos2(rect.max.x - 18.0, rect.center().y), key, theme::PAPER_WHITE);
    theme::focus_ring(ui, &resp);
    enabled && resp.clicked()
}

/// Glim drawn small inside the chrome bar.
pub fn glim_cell(app: &mut BestOfBurstApp, ui: &mut Ui, mood: super::glim::Mood) {
    let (rect, resp) = ui.allocate_exact_size(vec2(64.0, HEADER_H), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Link, true, "Glim: open Glim's corner"));
    let g = Rect::from_center_size(rect.center(), vec2(42.0, 42.0));
    super::glim::paint(ui, g, mood, true, app.time(), app.reduced_motion, 1.0);
    if resp.clicked() {
        app.open_corner();
    }
    cell_rule(ui);
}

/// The live status line shown in the header.
pub fn engine_status(app: &BestOfBurstApp) -> String {
    let s = app.sys();
    let gpu = match (app.settings.engine, crate::gpu::status()) {
        (EngineChoice::Cpu, _) => "Graphics off".to_string(),
        (_, Some(Ok(_))) => match s.gpu_total {
            Some(u) => format!("Graphics {u:.0}%"),
            None => "Graphics on".into(),
        },
        (_, Some(Err(_))) => "No graphics card".into(),
        (_, None) => "Graphics ...".into(),
    };
    format!("{gpu} · Processor {:.0}%", s.cpu_total)
}

/// Short GPU description for Home and the live panel.
pub fn gpu_line(s: &Snapshot) -> (String, String) {
    match crate::gpu::status() {
        None => ("Checking the graphics card...".into(), String::new()),
        Some(Err(_)) => ("Not available on this PC".into(), "The processor will do all the work".into()),
        Some(Ok(info)) => {
            let cores = s.gpu_cores.or_else(|| sysmon::known_cores(&info.name));
            let mut parts = Vec::new();
            if let Some(c) = cores {
                parts.push(format!("{} cores", theme::thousands(c as usize)));
            }
            if let Some(t) = s.gpu_mem_total_mb {
                parts.push(if t >= 1000 { format!("{:.0} GB memory", t as f32 / 1024.0) } else { format!("{t} MB memory") });
            }
            parts.push("ready".into());
            let detail = parts.join(" · ");
            (info.name, detail)
        }
    }
}

/// Toast line for the last message, if any.
pub fn message_line(app: &BestOfBurstApp, ui: &mut Ui, p: Pal) {
    if let Some((msg, err, _)) = &app.message {
        let c = if *err { p.mark } else { p.on_surface };
        ui.label(theme::body(msg, 14.0, c));
    }
}

pub fn center_text(ui: &Ui, rect: Rect, text: &str, p: Pal) {
    theme::text_at(ui, rect.center(), Align2::CENTER_CENTER, text, theme::f_mono(11.0), p.muted, 0.88);
}

impl eframe::App for BestOfBurstApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(rx) = &self.engine_rx {
            let msgs: Vec<EngineMsg> = rx.try_iter().collect();
            for msg in msgs {
                self.on_engine(msg, &ctx);
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let bg: Vec<BgMsg> = self.bg_rx.try_iter().collect();
        for msg in bg {
            self.on_bg(msg);
        }
        self.textures.poll(&ctx); // uploads finished thumbnail decodes

        if let Some((_, _, at)) = &self.message {
            if at.elapsed() > Duration::from_secs(10) {
                self.message = None;
            }
        }

        // Treats earned by settling close calls.
        if let Some(rv) = &mut self.review {
            if rv.pending_treats > 0 {
                self.settings.treats += rv.pending_treats;
                self.settings.settled += rv.pending_treats;
                rv.pending_treats = 0;
            }
        }
        match self.screen {
            Screen::Review | Screen::Focus | Screen::Compare | Screen::Viewer => {
                if self.review_clock.is_none() {
                    self.review_clock = Some(Instant::now());
                }
            }
            Screen::Home => self.review_clock = None,
            _ => {}
        }

        // Done is always the light table; Processing and Focus are always the darkroom.
        let surface_night = match self.screen {
            Screen::Done | Screen::Corner | Screen::Help => false,
            Screen::Progress | Screen::Focus | Screen::Compare | Screen::Viewer => true,
            _ => self.settings.night,
        };
        theme::apply_visuals(&ctx, theme::pal(surface_night));

        // The viewer and full-screen compare fill the monitor; every other screen gives it back.
        let want_full = match self.screen {
            Screen::Viewer => true,
            Screen::Compare => self.review.as_ref().and_then(|r| r.cmp.as_ref()).is_some_and(|c| c.full),
            _ => false,
        };
        if want_full != self.fullscreen {
            self.fullscreen = want_full;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(want_full));
        }

        match self.screen {
            Screen::Home => home::show(self, ui),
            Screen::Progress => progress::show(self, ui),
            Screen::Review => review::show(self, ui),
            Screen::Focus => focus::show(self, ui),
            Screen::Compare => compare::show(self, ui),
            Screen::Viewer => viewer::show(self, ui),
            Screen::Done => done::show(self, ui),
            Screen::Corner => corner::show(self, ui),
            Screen::Help => help::show(self, ui),
        }
        resize_edges(&ctx);
        let asked = ctx.data_mut(|d| d.remove_temp::<bool>(help_flag()).unwrap_or(false));
        if asked || (!ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::F1))) {
            self.open_help();
        }

        if self.settings != self.saved_settings {
            self.saved_settings = self.settings.clone();
            save_json("settings.json", self.settings.clone());
        }
        if !self.reduced_motion {
            // Glim's calm idle loop and mark draw-on.
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            // Live readings still refresh.
            ctx.request_repaint_after(Duration::from_millis(700));
        }
    }
}
