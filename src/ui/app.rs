//! App state, theme, and message plumbing between the UI thread and the engine/background threads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};

use super::review::Review;
use super::textures::Textures;
use super::{home, progress, review};
use crate::engine::{self, Event, RunConfig};
use crate::output::OutputSummary;
use crate::rank::{Keep, Preset, RankSummary};
use crate::scan;
use crate::state::State;

pub const PANEL: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
pub const DIM: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
pub const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xff);
pub const PICK: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
pub const MAYBE: Color32 = Color32::from_rgb(0xd2, 0x99, 0x22);
pub const REJECT: Color32 = Color32::from_rgb(0xf8, 0x51, 0x49);
pub const CARD: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
pub const DEEP: Color32 = Color32::from_rgb(0x15, 0x15, 0x15);

pub enum Screen {
    Home,
    Progress,
    Review,
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
    pub last_thumb: Option<PathBuf>,
    pub failures: Vec<(PathBuf, String)>,
    pub failed: usize,
    pub missing_codec: usize,
    pub warnings: Vec<String>,
    pub ranking: bool,
    pub stopping: bool,
    pub started: Option<Instant>,
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
    pub textures: Textures,
    pub review: Option<Review>,
    pub message: Option<(String, bool, Instant)>,
    pub saver: Saver,
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

fn recent_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("bestofburst").join("recent.json"))
}

fn load_recent() -> Vec<PathBuf> {
    recent_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|d| serde_json::from_slice::<Vec<String>>(&d).ok())
        .map(|v| v.into_iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn save_recent(list: Vec<PathBuf>) {
    std::thread::spawn(move || {
        let Some(p) = recent_path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let v: Vec<String> = list.iter().map(|p| p.to_string_lossy().to_string()).collect();
        if let Ok(data) = serde_json::to_vec_pretty(&v) {
            let _ = std::fs::write(p, data);
        }
    });
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = Color32::from_rgb(0x24, 0x24, 0x24);
    v.extreme_bg_color = DEEP;
    v.faint_bg_color = Color32::from_rgb(0x23, 0x23, 0x23);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.linear_multiply(0.6);
    v.selection.stroke.color = TEXT;
    v.widgets.hovered.bg_stroke.color = ACCENT.linear_multiply(0.8);
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
    });
}

impl BestOfBurstApp {
    pub fn new(cc: &eframe::CreationContext<'_>, folder: Option<PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut app = BestOfBurstApp {
            screen: Screen::Home,
            folder: None,
            keep_text: "10%".into(),
            preset: Preset::General,
            recent: load_recent(),
            folder_info: None,
            checking_folder: false,
            engine_rx: None,
            stop: Arc::new(AtomicBool::new(false)),
            threads: engine::default_threads(),
            retry_failed: false,
            bg_tx,
            bg_rx,
            prog: ProgressInfo::default(),
            textures: Textures::new(&cc.egui_ctx),
            review: None,
            message: None,
            saver: Saver::new(),
        };
        if let Some(f) = folder {
            let f = std::path::absolute(&f).unwrap_or(f);
            app.choose_folder(f, &cc.egui_ctx);
        }
        app
    }

    pub fn out_dir(&self) -> Option<PathBuf> {
        self.folder.as_ref().map(|f| scan::default_out(f))
    }

    pub fn keep(&self) -> Option<Keep> {
        Keep::parse(&self.keep_text)
    }

    pub fn flash(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some((text.into(), error, Instant::now()));
    }

    /// A folder was picked: remember it and count what is new, off the UI thread.
    pub fn choose_folder(&mut self, folder: PathBuf, ctx: &egui::Context) {
        self.recent.retain(|p| p != &folder);
        self.recent.insert(0, folder.clone());
        self.recent.truncate(10);
        save_recent(self.recent.clone());
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
            self.flash("Keep must be a number, or a percentage like 10%.", true);
            return;
        };
        let mut cfg = RunConfig::new(folder);
        cfg.keep = keep;
        cfg.preset = self.preset;
        cfg.threads = self.threads;
        cfg.retry_failed = self.retry_failed;
        self.stop = Arc::new(AtomicBool::new(false));
        self.prog = ProgressInfo { scanning: true, started: Some(Instant::now()), ..Default::default() };
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

    fn enter_review(&mut self, state: State) {
        let Some(folder) = self.folder.clone() else { return };
        let keep = self.keep().unwrap_or_default();
        self.review = Some(Review::new(state, folder, keep, self.preset));
        self.screen = Screen::Review;
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
                if last_thumb.is_some() {
                    self.prog.last = last;
                    self.prog.last_thumb = last_thumb;
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
            EngineMsg::Ranked(_) => self.prog.ranking = true,
            EngineMsg::Finished(state) => {
                self.engine_rx = None;
                self.enter_review(*state);
            }
            EngineMsg::Stopped => {
                self.engine_rx = None;
                self.screen = Screen::Home;
                self.flash("Stopped and saved. Press Start to resume.", false);
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
                        let mut text = format!("Wrote {} files to {}", s.written, s.picks_dir.display());
                        if s.copied_fallback {
                            text.push_str(" (hard links not supported here; copied)");
                        }
                        if !s.errors.is_empty() {
                            text.push_str(&format!(", {} could not be placed", s.errors.len()));
                        }
                        self.flash(text, !s.errors.is_empty());
                    }
                    Err(e) => self.flash(e, true),
                }
            }
        }
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

impl eframe::App for BestOfBurstApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
            if at.elapsed() > Duration::from_secs(8) {
                self.message = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        match self.screen {
            Screen::Home => home::show(self, ui),
            Screen::Progress => progress::show(self, ui),
            Screen::Review => review::show(self, ui),
        }
    }
}
