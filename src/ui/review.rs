//! Review state and logic (ranking in memory, overrides with undo/redo, close-call walk), plus
//! the contact sheet (screen 3): timeline histogram, time rows of six marked frames and the side
//! panel with Glim, the close-call count and the selected frame.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, Color32, Key, Rect, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo};

use super::app::{self, BestOfBurstApp, BgMsg, Screen};
use super::glim::{self, Mood};
use super::marks;
use super::textures::Loaded;
use super::theme::{self, Btn, Pal};
use crate::output::{self, OutputOptions};
use crate::rank::{self, better, better_in_group, Entry, Keep, Preset, Ranking, Status, PRESETS};
use crate::scan::{self, Format};
use crate::state::State;

const UNDO_STEPS: usize = 50;
const SAVE_DELAY: Duration = Duration::from_secs(1);
const COLS: usize = 6;
const GUTTER: f32 = 92.0;
const BINS: usize = 80;
pub const FULL_UV: Rect = Rect { min: egui::Pos2 { x: 0.0, y: 0.0 }, max: egui::Pos2 { x: 1.0, y: 1.0 } };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Picks,
    Maybe,
    Rejects,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortBy {
    Score,
    Time,
    Name,
}

#[derive(Clone, Copy)]
pub struct GridItem {
    pub idx: usize,
    /// Members of the same-shot group this card stands for (1 when expanded or alone).
    pub count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmpMode {
    Wipe,
    Blink,
}

pub struct FocusState {
    pub group: u64,
    /// The frame being looked at (entry index).
    pub view: usize,
    pub big_loupe: bool,
}

pub struct CompareState {
    pub group: u64,
    pub a: usize,
    pub b: usize,
    pub mode: CmpMode,
    pub show_b: bool,
    pub auto_blink: bool,
    pub last_flip: Instant,
    pub zoom: f32,
    pub center: Vec2,
    pub crop: bool,
}

/// Where a view wants to go after this frame.
pub enum Nav {
    Stay,
    Home,
    To(Screen),
    Corner,
}

pub struct Review {
    pub state: State,
    pub input: PathBuf,
    pub out: PathBuf,
    pub ranking: Ranking,
    pub keep: Keep,
    pub keep_text: String,
    pub preset: Preset,
    pub max_per_group: usize,
    pub tab: Tab,
    pub sort: SortBy,
    pub stacked: bool,
    pub close_only: bool,
    pub expanded: HashSet<u64>,
    pub selected: Option<usize>,
    pub items: Vec<GridItem>,
    items_dirty: bool,
    visible_rows: std::ops::Range<usize>,
    scroll_to: Option<usize>,
    undo: Vec<BTreeMap<String, String>>,
    redo: Vec<BTreeMap<String, String>>,
    dirty_since: Option<Instant>,
    pub focus: Option<FocusState>,
    pub cmp: Option<CompareState>,
    pub writing: bool,
    pub write_opts: OutputOptions,
    /// group -> ok members, eligible first, best group score first.
    pub groups: HashMap<u64, Vec<usize>>,
    pub formats: Vec<Format>,
    pub rerank_ms: f32,
    /// Decision changes to draw on (entry index -> when).
    pub mark_anim: HashMap<usize, Instant>,
    /// Close-call groups the user has settled.
    pub settled: HashSet<u64>,
    pub pending_treats: u32,
}

pub fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

/// Frame number for edge print: the last four digits of the file name, else the index.
pub fn frame_no(path: &str, idx: usize) -> String {
    let digits: String = file_name(path).chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 3 {
        digits[digits.len().saturating_sub(4)..].to_string()
    } else {
        format!("{:04}", idx + 1)
    }
}

pub fn hhmmss(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.naive_utc().format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Status words used in the window.
pub fn status_word(s: Status) -> &'static str {
    match s {
        Status::Pick => "keeper",
        Status::Maybe => "maybe",
        Status::Reject => "set aside",
        Status::Failed => "unreadable",
    }
}

impl Review {
    pub fn new(state: State, input: PathBuf, keep: Keep, preset: Preset) -> Self {
        let out = scan::default_out(&input);
        let (keep, preset, max_per_group) = match &state.last_rank {
            Some(lr) => {
                let n_done = state.files.values().filter(|f| f.is_done()).count();
                // Show the last ranking's count unless it was the default percentage of the same set.
                let k = if keep.target(n_done) == lr.target { keep } else { Keep::Count(lr.target) };
                (k, Preset::parse(&lr.preset).unwrap_or(preset), lr.max_per_group.max(1))
            }
            None => (keep, preset, 1),
        };
        let ranking = rank::rank(&state, preset, keep, max_per_group);
        let formats = ranking
            .entries
            .iter()
            .map(|e| state.files.get(&e.path).and_then(|f| Format::from_name(&f.format)).unwrap_or(Format::Jpeg))
            .collect();
        let mut r = Review {
            state,
            input,
            out,
            ranking,
            keep,
            keep_text: keep.display(),
            preset,
            max_per_group,
            tab: Tab::Picks,
            sort: SortBy::Time,
            stacked: true,
            close_only: false,
            expanded: HashSet::new(),
            selected: None,
            items: Vec::new(),
            items_dirty: true,
            visible_rows: 0..0,
            scroll_to: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty_since: None,
            focus: None,
            cmp: None,
            writing: false,
            write_opts: OutputOptions::default(),
            groups: HashMap::new(),
            formats,
            rerank_ms: 0.0,
            mark_anim: HashMap::new(),
            settled: HashSet::new(),
            pending_treats: 0,
        };
        r.rebuild_groups();
        r.rebuild_items();
        r.selected = r.items.first().map(|i| i.idx);
        r
    }

    pub fn entries(&self) -> &[Entry] {
        &self.ranking.entries
    }

    fn rebuild_groups(&mut self) {
        let mut g: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, e) in self.ranking.entries.iter().enumerate() {
            if e.ok {
                g.entry(e.group).or_default().push(i);
            }
        }
        let e = &self.ranking.entries;
        for members in g.values_mut() {
            members.sort_by(|&a, &b| e[b].eligible().cmp(&e[a].eligible()).then_with(|| better_in_group(&e[a], &e[b])));
        }
        self.groups = g;
    }

    pub fn rerank(&mut self) {
        let t = Instant::now();
        self.ranking = rank::rank(&self.state, self.preset, self.keep, self.max_per_group);
        self.rebuild_groups();
        self.items_dirty = true;
        self.rerank_ms = t.elapsed().as_secs_f32() * 1000.0;
        self.mark_dirty();
    }

    fn mark_dirty(&mut self) {
        self.dirty_since = Some(Instant::now());
    }

    fn tab_matches(&self, e: &Entry) -> bool {
        let s = e.effective();
        let in_tab = match self.tab {
            Tab::Picks => s == Status::Pick,
            Tab::Maybe => s == Status::Maybe,
            Tab::Rejects => matches!(s, Status::Reject | Status::Failed),
            Tab::All => true,
        };
        in_tab && (!self.close_only || e.close_call)
    }

    fn sort_cmp(&self, a: usize, b: usize) -> std::cmp::Ordering {
        let e = self.entries();
        match self.sort {
            SortBy::Score => better(&e[a], &e[b]),
            SortBy::Time => e[a].capture_ms.cmp(&e[b].capture_ms).then_with(|| e[a].path.cmp(&e[b].path)),
            SortBy::Name => e[a].path.cmp(&e[b].path),
        }
    }

    pub fn rebuild_items(&mut self) {
        let e = &self.ranking.entries;
        let mut list: Vec<usize> = (0..e.len()).filter(|&i| self.tab_matches(&e[i])).collect();
        list.sort_by(|&a, &b| self.sort_cmp(a, b));
        let mut items = Vec::with_capacity(list.len());
        if self.stacked {
            // One card per group: its effective pick, else its best member in this list.
            let mut members: HashMap<u64, Vec<usize>> = HashMap::new();
            for &i in &list {
                members.entry(e[i].group).or_default().push(i);
            }
            let mut reps: Vec<(usize, u64)> = members
                .iter()
                .map(|(g, m)| {
                    let rep = *m
                        .iter()
                        .min_by(|&&a, &&b| {
                            (e[b].effective() == Status::Pick)
                                .cmp(&(e[a].effective() == Status::Pick))
                                .then_with(|| better_in_group(&e[a], &e[b]))
                        })
                        .unwrap_or(&m[0]);
                    (rep, *g)
                })
                .collect();
            reps.sort_by(|a, b| self.sort_cmp(a.0, b.0));
            for (rep, g) in reps {
                let m = &members[&g];
                if self.expanded.contains(&g) && m.len() > 1 {
                    let mut all = m.clone();
                    all.sort_by(|&a, &b| better_in_group(&e[a], &e[b]));
                    items.extend(all.into_iter().map(|idx| GridItem { idx, count: 1 }));
                } else {
                    items.push(GridItem { idx: rep, count: m.len() });
                }
            }
        } else {
            items.extend(list.into_iter().map(|idx| GridItem { idx, count: 1 }));
        }
        self.items = items;
        self.items_dirty = false;
        if let Some(sel) = self.selected {
            if !self.items.iter().any(|i| i.idx == sel) {
                let g = self.ranking.entries[sel].group;
                if let Some(it) = self.items.iter().find(|i| self.ranking.entries[i.idx].group == g) {
                    self.selected = Some(it.idx);
                }
            }
        }
    }

    pub fn tab_count(&self, t: Tab) -> usize {
        let s = &self.ranking.summary;
        match t {
            Tab::Picks => s.picks,
            Tab::Maybe => s.maybe,
            Tab::Rejects => s.rejects + s.failed,
            Tab::All => self.ranking.entries.len(),
        }
    }

    pub fn set_tab(&mut self, t: Tab) {
        if self.tab != t {
            self.tab = t;
            self.rebuild_items();
            self.selected = self.items.first().map(|i| i.idx);
            self.scroll_to = Some(0);
        }
    }

    /// Set (or clear) the override on one file. Picking a frame turns the group's previous
    /// pick into Maybe ("move the circle").
    pub fn set_status(&mut self, idx: usize, status: Option<Status>) {
        self.push_undo();
        let path = self.ranking.entries[idx].path.clone();
        match status {
            None => {
                self.state.overrides.remove(&path);
            }
            Some(s) => {
                if s == Status::Pick && self.max_per_group == 1 {
                    let g = self.ranking.entries[idx].group;
                    let others: Vec<String> = self
                        .groups
                        .get(&g)
                        .map(|m| {
                            m.iter()
                                .filter(|&&i| i != idx && self.ranking.entries[i].effective() == Status::Pick)
                                .map(|&i| self.ranking.entries[i].path.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    for o in others {
                        self.state.overrides.insert(o, "maybe".into());
                    }
                }
                self.state.overrides.insert(path, s.name().to_string());
            }
        }
        self.apply_overrides();
    }

    fn apply_overrides(&mut self) {
        let before: Vec<Status> = self.ranking.entries.iter().map(|e| e.effective()).collect();
        rank::apply_overrides(&mut self.ranking.entries, &self.state.overrides);
        let now = Instant::now();
        for (i, e) in self.ranking.entries.iter().enumerate() {
            if e.effective() != before[i] {
                self.mark_anim.insert(i, now);
            }
        }
        self.ranking.summary = rank::summarize(&self.ranking.entries, self.ranking.target);
        self.items_dirty = true;
        self.mark_dirty();
    }

    fn push_undo(&mut self) {
        self.undo.push(self.state.overrides.clone());
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.state.overrides, prev));
            self.apply_overrides();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.state.overrides, next));
            self.apply_overrides();
        }
    }

    /// Draw-on progress (0..1) of an entry's mark.
    pub fn mark_t(&self, idx: usize, reduced: bool) -> f32 {
        if reduced {
            return 1.0;
        }
        match self.mark_anim.get(&idx) {
            Some(t) => marks::ease_out(t.elapsed().as_secs_f32() * 1000.0 / marks::DRAW_MS),
            None => 1.0,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let pos = self.selected.and_then(|s| self.items.iter().position(|i| i.idx == s)).unwrap_or(0) as isize;
        let next = (pos + delta).clamp(0, self.items.len() as isize - 1) as usize;
        self.selected = Some(self.items[next].idx);
        self.scroll_to = Some(next);
    }

    /// Close-call groups, best first.
    pub fn close_call_groups(&self) -> Vec<u64> {
        let e = self.entries();
        let mut groups: Vec<(u64, usize)> = self
            .groups
            .iter()
            .filter(|(_, m)| m.len() >= 2 && e[m[0]].close_call)
            .map(|(g, m)| (*g, m[0]))
            .collect();
        groups.sort_by(|a, b| better(&e[a.1], &e[b.1]));
        groups.into_iter().map(|(g, _)| g).collect()
    }

    pub fn open_close_calls(&self) -> usize {
        self.close_call_groups().iter().filter(|g| !self.settled.contains(g)).count()
    }

    /// The group's circled frame (its effective pick, else its best member).
    pub fn keeper_of(&self, group: u64) -> Option<usize> {
        let m = self.groups.get(&group)?;
        m.iter().copied().find(|&i| self.ranking.entries[i].effective() == Status::Pick).or(m.first().copied())
    }

    pub fn runner_up(&self, group: u64, keeper: usize) -> Option<usize> {
        self.groups.get(&group)?.iter().copied().find(|&i| i != keeper)
    }

    pub fn members(&self, group: u64) -> Vec<usize> {
        self.groups.get(&group).cloned().unwrap_or_default()
    }

    pub fn open_focus(&mut self, group: u64, view: Option<usize>) -> bool {
        let Some(k) = self.keeper_of(group) else { return false };
        let view = view.filter(|v| self.ranking.entries[*v].group == group).unwrap_or(k);
        self.focus = Some(FocusState { group, view, big_loupe: false });
        self.selected = Some(view);
        true
    }

    /// The next close call after `current` (unsettled first), wrapping around.
    pub fn next_close_call(&self, current: Option<u64>) -> Option<u64> {
        let all = self.close_call_groups();
        if all.is_empty() {
            return None;
        }
        let start = current.and_then(|g| all.iter().position(|x| *x == g)).map(|p| p + 1).unwrap_or(0);
        let n = all.len();
        (0..n)
            .map(|k| all[(start + k) % n])
            .find(|g| !self.settled.contains(g) && Some(*g) != current)
            .or_else(|| (0..n).map(|k| all[(start + k) % n]).find(|g| Some(*g) != current))
    }

    /// Settling a close call earns Glim one treat.
    pub fn settle(&mut self, group: u64) {
        let is_close = self.groups.get(&group).map(|m| m.len() >= 2 && self.ranking.entries[m[0]].close_call).unwrap_or(false);
        if is_close && self.settled.insert(group) {
            self.pending_treats += 1;
        }
    }

    pub fn open_compare(&mut self, group: u64, b: Option<usize>) -> bool {
        let Some(a) = self.keeper_of(group) else { return false };
        let Some(b) = b.filter(|x| *x != a).or_else(|| self.runner_up(group, a)) else { return false };
        self.cmp = Some(CompareState {
            group,
            a,
            b,
            mode: CmpMode::Wipe,
            show_b: false,
            auto_blink: true,
            last_flip: Instant::now(),
            zoom: 1.0,
            center: vec2(0.5, 0.5),
            crop: false,
        });
        true
    }

    pub fn abs_path(&self, idx: usize) -> PathBuf {
        self.input.join(&self.ranking.entries[idx].path)
    }

    pub fn thumb_path(&self, idx: usize) -> Option<PathBuf> {
        let e = &self.ranking.entries[idx];
        self.state.files.get(&e.path).and_then(|f| f.thumb.as_ref()).map(|t| self.out.join(t))
    }

    pub fn center_of(&self, idx: usize) -> [f32; 2] {
        let g = self.ranking.entries[idx].group;
        self.ranking.centers.get(&g).copied().unwrap_or([0.5, 0.5])
    }

    /// Save overrides 1 s after the last change (on the saver thread).
    pub fn maybe_save(&mut self, saver: &app::Saver, ctx: &egui::Context) {
        if let Some(t) = self.dirty_since {
            if t.elapsed() >= SAVE_DELAY {
                self.flush(saver);
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }
    }

    pub fn flush(&mut self, saver: &app::Saver) {
        if self.dirty_since.take().is_some() {
            rank::store(&mut self.state, &self.ranking);
            saver.save(self.state.clone(), self.out.clone());
        }
    }

    /// Run the Output step with the user's overrides and options, on a worker.
    pub fn deliver(&mut self, bg: std::sync::mpsc::Sender<BgMsg>, ctx: &egui::Context) {
        if self.writing {
            return;
        }
        self.writing = true;
        rank::store(&mut self.state, &self.ranking);
        self.dirty_since = None;
        let mut state = self.state.clone();
        let entries = self.ranking.entries.clone();
        let (input, out, opts) = (self.input.clone(), self.out.clone(), self.write_opts);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = (|| -> anyhow::Result<output::OutputSummary> {
                let s = output::write_folders(&input, &out, &entries, opts)?;
                output::write_report(&out, &state, &entries)?;
                state.save(&out)?;
                Ok(s)
            })();
            let _ = bg.send(BgMsg::Written(r.map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
    }

    /// Five components of `idx` and the delta label against `other`.
    pub fn component_rows(&self, idx: usize, other: Option<usize>, other_name: &str) -> Vec<(&'static str, f32, Option<f32>, String)> {
        let e = &self.ranking.entries[idx];
        let o = other.map(|i| &self.ranking.entries[i]);
        let names = ["Sharpness", "Exposure", "Contrast", "Low noise", "Colour"];
        let mut rows: Vec<_> = (0..5)
            .map(|k| {
                let ov = o.map(|x| x.components[k]);
                (names[k], e.components[k], ov, theme::delta_label(e.components[k], ov, other_name))
            })
            .collect();
        if o.is_some() || e.group_size > 1 {
            let gv = e.group_score / 100.0;
            let og = o.map(|x| x.group_score / 100.0);
            rows.push(("Sharp on subject", gv, og, theme::delta_label(gv, og, other_name)));
        }
        rows
    }
}

/// Take the review out of the app, run a view, put it back and act on its navigation.
pub fn run_view(app: &mut BestOfBurstApp, ui: &mut Ui, view: fn(&mut BestOfBurstApp, &mut Review, &mut Ui) -> Nav) {
    let ctx = ui.ctx().clone();
    let Some(mut rv) = app.review.take() else {
        app.screen = Screen::Home;
        return;
    };
    let nav = view(app, &mut rv, ui);
    rv.maybe_save(&app.saver, &ctx);
    app.review = Some(rv);
    match nav {
        Nav::Stay => {}
        Nav::Home => app.go_home(&ctx),
        Nav::To(s) => app.screen = s,
        Nav::Corner => app.open_corner(),
    }
}

/// One frame on film: well, sprockets, image, marks, "?", stack count and edge print.
pub struct CardOpts {
    pub count: usize,
    pub selected: bool,
    pub letter: Option<char>,
    pub show_score: bool,
    /// Hide marks, "?" and reasons (the Sharp Eye game).
    pub plain: bool,
}

pub fn frame_card(ui: &mut Ui, rv: &Review, app: &mut BestOfBurstApp, idx: usize, size: Vec2, o: CardOpts) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let e = &rv.ranking.entries[idx];
    let status = e.effective();
    let name = file_name(&e.path).to_string();
    let label = format!(
        "Frame {}, {}{}{}",
        frame_no(&e.path, idx),
        match status {
            Status::Pick => "circled",
            Status::Maybe => "maybe",
            Status::Reject => "set aside",
            Status::Failed => "unreadable",
        },
        if e.close_call { ", close call" } else { "" },
        if o.count > 1 { format!(", stack of {}", o.count) } else { String::new() }
    );
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, o.selected, &label));
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let well = rect.shrink(3.0);
    theme::film_well(ui, well);
    if resp.hovered() {
        ui.painter().rect_filled(well, 0.0, Color32::from_white_alpha(10));
    }
    let edge_h = if size.y > 90.0 { 18.0 } else { 14.0 };
    let img_area = Rect::from_min_max(well.min + vec2(9.0, 10.0), well.max - vec2(9.0, 10.0 + edge_h));
    let aside = matches!(status, Status::Reject | Status::Failed);
    let mut img_rect = img_area;
    if let Some(t) = rv.thumb_path(idx) {
        if let Loaded::Ready(id, sz) = app.textures.thumb(&t) {
            img_rect = theme::fit(sz, img_area);
            ui.painter().image(id, img_rect, FULL_UV, if aside && !o.plain { theme::ASIDE_TINT } else { Color32::WHITE });
        }
    }
    let t = rv.mark_t(idx, app.reduced_motion);
    let big = size.x > 150.0;
    if o.plain {
        if o.selected {
            ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.5, theme::CYAN), StrokeKind::Inside);
        }
        return resp;
    }
    let clip = ui.painter().clip_rect();
    ui.set_clip_rect(rect.intersect(clip));
    match status {
        Status::Pick => marks::keeper(ui, img_rect.expand2(vec2(8.0, 6.0)), theme::SAFELIGHT, if big { 3.5 } else { 2.4 }, t),
        Status::Maybe => {
            let w = Rect::from_min_max(pos2(img_rect.min.x + img_rect.width() * 0.1, img_rect.max.y - 3.0), pos2(img_rect.max.x - img_rect.width() * 0.1, img_rect.max.y + 9.0));
            marks::wave(ui, w, theme::SAFELIGHT, if big { 3.0 } else { 2.2 }, t)
        }
        Status::Reject | Status::Failed => marks::cross(ui, img_rect.shrink(4.0), theme::TABLE, if big { 3.0 } else { 2.2 }, t),
    }
    ui.set_clip_rect(clip);
    if e.close_call {
        marks::question(ui, pos2(well.max.x - 6.0, well.min.y + 4.0), if big { 26.0 } else { 18.0 });
    }
    let ey = well.max.y - 8.0 - edge_h / 2.0 + 4.0;
    if o.count > 1 {
        theme::text_at(ui, pos2(well.min.x + 8.0, ey), Align2::LEFT_CENTER, &format!("×{}", o.count), theme::f_mono_semi(10.0), theme::TABLE, 0.4);
    }
    let mut edge = match o.letter {
        Some(c) => format!("{c} {}", frame_no(&e.path, idx)),
        None => frame_no(&e.path, idx),
    };
    if o.show_score && e.ok {
        edge.push_str(&format!("  {:.0}", e.score));
    }
    let er = theme::text_at(ui, pos2(well.max.x - 8.0, ey), Align2::RIGHT_CENTER, &edge, theme::f_mono(10.0), theme::EDGE, 0.5);
    theme::edge_arrow(ui, pos2(er.min.x - 9.0, er.center().y), theme::EDGE);
    if aside && big {
        let reason = e.reasons.first().map(|r| r.replace('_', " ")).unwrap_or_else(|| "manual".into());
        theme::text_at(ui, pos2(well.min.x + 8.0, well.min.y + 12.0), Align2::LEFT_TOP, &reason.to_uppercase(), theme::f_mono_semi(10.0), theme::SAFELIGHT_SOFT, 0.6);
    }
    if o.selected {
        ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.5, theme::CYAN), StrokeKind::Inside);
    }
    let _ = name;
    resp
}

// ---- Contact sheet ----

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    run_view(app, ui, sheet);
}

fn sheet(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui) -> Nav {
    let ctx = ui.ctx().clone();
    let mut nav = Nav::Stay;
    let p = app.pal();

    // Keys.
    if !ctx.egui_wants_keyboard_input() {
        let (keys, cmd) = pressed_keys(&ctx);
        for key in keys {
            if cmd {
                match key {
                    Key::Z => rv.undo(),
                    Key::Y => rv.redo(),
                    Key::E => nav = Nav::To(Screen::Done),
                    _ => {}
                }
                continue;
            }
            let sel = rv.selected;
            match key {
                Key::ArrowLeft => rv.move_selection(-1),
                Key::ArrowRight => rv.move_selection(1),
                Key::ArrowUp => rv.move_selection(-(COLS as isize)),
                Key::ArrowDown => rv.move_selection(COLS as isize),
                Key::P => sel.into_iter().for_each(|i| rv.set_status(i, Some(Status::Pick))),
                Key::U => sel.into_iter().for_each(|i| rv.set_status(i, Some(Status::Maybe))),
                Key::X => sel.into_iter().for_each(|i| rv.set_status(i, Some(Status::Reject))),
                Key::Num0 => sel.into_iter().for_each(|i| rv.set_status(i, None)),
                Key::Space => {
                    if let Some(i) = sel {
                        let g = rv.ranking.entries[i].group;
                        if rv.open_focus(g, Some(i)) {
                            nav = Nav::To(Screen::Focus);
                        }
                    }
                }
                Key::G => {
                    if let Some(i) = sel {
                        let g = rv.ranking.entries[i].group;
                        if !rv.expanded.remove(&g) {
                            rv.expanded.insert(g);
                        }
                        rv.items_dirty = true;
                    }
                }
                Key::Num1 => rv.set_tab(Tab::Picks),
                Key::Num2 => rv.set_tab(Tab::Maybe),
                Key::Num3 => rv.set_tab(Tab::Rejects),
                Key::Num4 => rv.set_tab(Tab::All),
                Key::N => {
                    if let Some(g) = rv.next_close_call(None) {
                        rv.open_focus(g, None);
                        nav = Nav::To(Screen::Focus);
                    }
                }
                Key::C => {
                    if let Some(i) = sel {
                        let g = rv.ranking.entries[i].group;
                        if rv.open_compare(g, None) {
                            nav = Nav::To(Screen::Compare);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if rv.items_dirty {
        rv.rebuild_items();
    }

    // Header.
    let shoot = app.shoot_name();
    let status = app::engine_status(app);
    let mut night = app.settings.night;
    let mut deliver = false;
    let mut home = false;
    let picks = rv.ranking.summary.picks;
    app::chrome_bar(
        ui,
        "review_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, &shoot, true);
        },
        |ui| {
            deliver = app::red_cell(ui, &format!("Deliver {}", theme::thousands(picks)), "Ctrl E", true);
            app::text_cell(ui, &status, false);
            ui.add_space(16.0);
            theme::table_toggle(ui, &mut night);
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    app.settings.night = night;
    if home {
        nav = Nav::Home;
    }
    if deliver {
        nav = Nav::To(Screen::Done);
    }

    if let Some(min) = app.break_due() {
        let mut action = theme::BannerAction::None;
        egui::Panel::top("break_banner").frame(egui::Frame::new()).show(ui, |ui| {
            action = theme::safelight_banner(ui, min);
        });
        match action {
            theme::BannerAction::Done => app.review_clock = Some(Instant::now()),
            theme::BannerAction::Play => {
                app.review_clock = Some(Instant::now());
                nav = Nav::Corner;
            }
            theme::BannerAction::None => {}
        }
    }

    filter_bar(rv, ui, p);
    if let Some(n) = side_panel(rv, app, ui, p) {
        nav = n;
    }

    let main = egui::Frame::new().fill(p.surface).inner_margin(egui::Margin { left: 24, right: 20, top: 16, bottom: 8 });
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        timeline(rv, ui, p);
        ui.add_space(10.0);
        if let Some(n) = contact_rows(rv, app, ui, p) {
            nav = n;
        }
    });
    nav
}

pub fn pressed_keys(ctx: &egui::Context) -> (Vec<Key>, bool) {
    ctx.input(|i| {
        let keys = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            })
            .collect();
        (keys, i.modifiers.command)
    })
}

fn filter_bar(rv: &mut Review, ui: &mut Ui, p: Pal) {
    let frame = egui::Frame::new().fill(p.surface).inner_margin(egui::Margin { left: 24, right: 20, top: 8, bottom: 0 });
    egui::Panel::top("filter_bar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (t, name, key) in [(Tab::Picks, "Keepers", "1"), (Tab::Maybe, "Maybe", "2"), (Tab::Rejects, "Set aside", "3"), (Tab::All, "All", "4")] {
                let text = format!("{} {}", name.to_uppercase(), theme::thousands(rv.tab_count(t)));
                if filter_cell(ui, &text, rv.tab == t, p, &format!("{name} (key {key})")) {
                    rv.set_tab(t);
                }
            }
            let close = format!("CLOSE CALLS {}", rv.ranking.summary.close_call_groups);
            if filter_cell(ui, &close, rv.close_only, p, "Close calls filter") {
                rv.close_only = !rv.close_only;
                rv.items_dirty = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if rv.stacked { "STACKED" } else { "EXPANDED" };
                if filter_cell(ui, label, false, p, "Groups stacked or expanded") {
                    rv.stacked = !rv.stacked;
                    rv.expanded.clear();
                    rv.items_dirty = true;
                }
                let before = rv.sort;
                let sort_label = egui::ComboBox::from_id_salt("sort")
                    .selected_text(theme::mono(match rv.sort { SortBy::Time => "Time", SortBy::Score => "Score", SortBy::Name => "Name" }, 10.5, p.on_surface))
                    .width(78.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut rv.sort, SortBy::Time, "Time");
                        ui.selectable_value(&mut rv.sort, SortBy::Score, "Score");
                        ui.selectable_value(&mut rv.sort, SortBy::Name, "Name");
                    });
                let sl = ui.label(theme::mono("Sort", 10.5, p.muted));
                sort_label.response.labelled_by(sl.id);
                if rv.sort != before {
                    rv.items_dirty = true;
                }
                ui.add_space(10.0);
                let before = rv.preset;
                let pr = egui::ComboBox::from_id_salt("preset")
                    .selected_text(theme::mono(rv.preset.label(), 10.5, p.on_surface))
                    .width(96.0)
                    .show_ui(ui, |ui| {
                        for x in PRESETS {
                            ui.selectable_value(&mut rv.preset, x, x.label());
                        }
                    });
                let pl = ui.label(theme::mono("Shot type", 10.5, p.muted));
                pr.response.labelled_by(pl.id);
                if rv.preset != before {
                    rv.rerank();
                }
                ui.add_space(10.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut rv.keep_text)
                        .font(theme::f_mono_semi(12.0))
                        .desired_width(56.0),
                );
                let kl = ui.label(theme::mono("Keep", 10.5, p.muted));
                let resp = resp.labelled_by(kl.id);
                if resp.changed() {
                    if let Some(k) = Keep::parse(&rv.keep_text) {
                        if k != rv.keep {
                            rv.keep = k;
                            rv.rerank();
                        }
                    }
                }
            });
        });
        theme::rule(ui, p.rule, 1.0);
    });
}

fn filter_cell(ui: &mut Ui, text: &str, active: bool, p: Pal, a11y: &str) -> bool {
    let g = theme::galley(ui, text, theme::f_xp(12.0), p.on_surface, 0.4);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 40.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, active, a11y));
    let c = if active || resp.hovered() { p.on_surface } else { p.muted };
    let g = theme::galley(ui, text, theme::f_xp(12.0), c, 0.4);
    ui.painter().galley(pos2(rect.min.x + 12.0, rect.center().y - g.size().y / 2.0), g, c);
    if active {
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.min.x + 12.0, rect.max.y - 2.0), pos2(rect.max.x - 12.0, rect.max.y)), 0.0, p.mark);
    }
    theme::focus_ring(ui, &resp);
    resp.clicked()
}

/// 6.6 Timeline: 80 bars across the shoot; bars with close calls in the mark colour.
fn timeline(rv: &mut Review, ui: &mut Ui, p: Pal) {
    let e = &rv.ranking.entries;
    let times: Vec<i64> = e.iter().filter(|x| x.ok).map(|x| x.capture_ms).collect();
    let (Some(&t0), Some(&t1)) = (times.iter().min(), times.iter().max()) else { return };
    let span = (t1 - t0).max(1) as f64;
    let mut counts = [0usize; BINS];
    let mut close = [false; BINS];
    for x in e.iter().filter(|x| x.ok) {
        let b = (((x.capture_ms - t0) as f64 / span) * (BINS as f64 - 1.0)).round() as usize;
        counts[b.min(BINS - 1)] += 1;
        if x.close_call {
            close[b.min(BINS - 1)] = true;
        }
    }
    let max = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    ui.label(theme::mono("The shoot", 10.5, p.muted));
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 56.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Slider, true, "Shoot timeline"));
    let bw = rect.width() / BINS as f32;
    for b in 0..BINS {
        if counts[b] == 0 {
            continue;
        }
        let h = (counts[b] as f32 / max).sqrt() * rect.height();
        let r = Rect::from_min_max(pos2(rect.min.x + b as f32 * bw + 1.0, rect.max.y - h), pos2(rect.min.x + (b + 1) as f32 * bw - 1.0, rect.max.y));
        ui.painter().rect_filled(r, 0.0, if close[b] { p.mark } else { p.bar });
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, p.rule));
    ui.horizontal(|ui| {
        ui.label(theme::mono(hhmmss(t0), 10.0, p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(theme::mono(hhmmss(t1), 10.0, p.muted));
        });
    });
    if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
        let b = ((pos.x - rect.min.x) / bw).floor().clamp(0.0, BINS as f32 - 1.0);
        let at = t0 + (b as f64 / (BINS as f64 - 1.0) * span) as i64;
        if rv.sort != SortBy::Time {
            rv.sort = SortBy::Time;
            rv.rebuild_items();
        }
        if let Some(k) = rv.items.iter().position(|it| rv.ranking.entries[it.idx].capture_ms >= at) {
            rv.selected = Some(rv.items[k].idx);
            rv.scroll_to = Some(k);
        }
    } else if let Some(pos) = resp.hover_pos() {
        let b = ((pos.x - rect.min.x) / bw).floor().clamp(0.0, BINS as f32 - 1.0) as usize;
        let at = t0 + (b as f64 / (BINS as f64 - 1.0) * span) as i64;
        resp.on_hover_text(format!("{}  ·  {} frames", hhmmss(at), counts[b]));
    }
}

fn contact_rows(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut Ui, p: Pal) -> Option<Nav> {
    let mut nav = None;
    if rv.items.is_empty() {
        ui.add_space(40.0);
        ui.label(theme::xp("Nothing on this part of the table.", 20.0, p.muted));
        return None;
    }
    let cell_w = ((ui.available_width() - GUTTER) / COLS as f32).floor();
    let cell = vec2(cell_w, (cell_w * 0.74 + 18.0).floor());
    let spacing = ui.spacing().item_spacing.y;
    let row_h = cell.y + spacing;
    let rows = rv.items.len().div_ceil(COLS);
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("sheet");
    if let Some(pos) = rv.scroll_to.take() {
        let row = pos / COLS;
        let vis = rv.visible_rows.clone();
        if vis.is_empty() || !(vis.start < row && row + 1 < vis.end) {
            let target = if row <= vis.start { row as f32 * row_h } else { (row as f32 + 1.0) * row_h - ui.available_height() };
            area = area.vertical_scroll_offset(target.max(0.0));
        }
    }
    area.show_rows(ui, cell.y, rows, |ui, range| {
        rv.visible_rows = range.clone();
        for r in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let first = rv.items[r * COLS].idx;
                let label = match rv.sort {
                    SortBy::Time => hhmmss(rv.ranking.entries[first].capture_ms),
                    _ => format!("#{:03}", r * COLS + 1),
                };
                let (g, _) = ui.allocate_exact_size(vec2(GUTTER, cell.y), Sense::hover());
                ui.painter().line_segment([g.left_top(), pos2(g.max.x - 10.0, g.min.y)], Stroke::new(1.0, p.rule));
                theme::text_at(ui, g.left_top() + vec2(0.0, 8.0), Align2::LEFT_TOP, &label, theme::f_mono(10.5), p.muted, 0.6);
                for c in 0..COLS {
                    let Some(item) = rv.items.get(r * COLS + c).copied() else { break };
                    let selected = rv.selected == Some(item.idx);
                    let resp = frame_card(ui, rv, app, item.idx, cell, CardOpts { count: item.count, selected, letter: None, show_score: true, plain: false });
                    if resp.clicked() {
                        rv.selected = Some(item.idx);
                    }
                    if resp.double_clicked() {
                        let g = rv.ranking.entries[item.idx].group;
                        if rv.open_focus(g, Some(item.idx)) {
                            nav = Some(Nav::To(Screen::Focus));
                        }
                    }
                }
            });
        }
    });
    nav
}

fn side_panel(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut Ui, p: Pal) -> Option<Nav> {
    let mut nav = None;
    let frame = egui::Frame::new().fill(p.surface).inner_margin(24.0);
    egui::Panel::right("review_side").exact_size(400.0).resizable(false).frame(frame).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.min.x - 24.0, r.min.y - 24.0), pos2(r.min.x - 24.0, r.max.y + 24.0)], Stroke::new(1.0, p.rule));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(352.0);
            let open = rv.open_close_calls();
            let msg = if open > 0 {
                format!("I circled {}. {} {} your eyes.", theme::plural(rv.ranking.summary.picks, "keeper"), theme::plural(open, "close call"), if open == 1 { "needs" } else { "need" })
            } else {
                format!("I circled {}. No close calls left. Nice work.", theme::plural(rv.ranking.summary.picks, "keeper"))
            };
            if glim::card(ui, Mood::Idle, p.night, &msg, 60.0, app.time(), app.reduced_motion).clicked() {
                nav = Some(Nav::Corner);
            }
            ui.add_space(16.0);
            ui.label(theme::mono("Needs your eyes", 11.0, p.muted));
            ui.label(theme::xp(theme::thousands(open), 96.0, p.on_surface).line_height(Some(86.0)));
            ui.add_space(6.0);
            if theme::button(ui, "Open the loupe", Some("N"), Btn::Primary, 56.0, Some(ui.available_width()), open > 0 || !rv.close_call_groups().is_empty()).clicked() {
                if let Some(g) = rv.next_close_call(None) {
                    rv.open_focus(g, None);
                    nav = Some(Nav::To(Screen::Focus));
                }
            }
            app::message_line(app, ui, p);
            ui.add_space(18.0);
            theme::rule(ui, p.rule, 1.5);
            ui.add_space(14.0);
            if let Some(n) = detail(rv, app, ui, p) {
                nav = Some(n);
            }
        });
    });
    nav
}

fn detail(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut Ui, p: Pal) -> Option<Nav> {
    let mut nav = None;
    let Some(idx) = rv.selected else {
        ui.label(theme::body("Select a frame on the sheet.", 14.0, p.muted));
        return None;
    };
    let e = rv.ranking.entries[idx].clone();
    ui.label(theme::mono("Selected frame", 11.0, p.muted));
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, w * 0.68), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, format!("Preview of {}", file_name(&e.path))));
    ui.painter().rect_filled(rect, 0.0, theme::FILM);
    let fmt = rv.formats[idx];
    match app.textures.preview(&rv.abs_path(idx), fmt) {
        Loaded::Ready(id, size) => {
            ui.painter().image(id, theme::fit(size, rect.shrink(6.0)), FULL_UV, Color32::WHITE);
        }
        Loaded::Failed(_) => app::center_text(ui, rect, "PREVIEW FAILED", p),
        Loaded::Loading => {
            if let Some(t) = rv.thumb_path(idx) {
                if let Loaded::Ready(id, size) = app.textures.thumb(&t) {
                    ui.painter().image(id, theme::fit(size, rect.shrink(6.0)), FULL_UV, Color32::WHITE);
                }
            }
        }
    }
    if resp.double_clicked() && rv.open_focus(e.group, Some(idx)) {
        nav = Some(Nav::To(Screen::Focus));
    }
    ui.add_space(6.0);
    ui.label(theme::xp(file_name(&e.path), 18.0, p.on_surface));
    let mut meta = format!("{}  ·  score {:.1}", status_word(e.effective()), e.score);
    if let Some(r) = e.rank {
        meta.push_str(&format!("  ·  rank {r}"));
    }
    if e.group_size > 1 {
        meta.push_str(&format!("  ·  stack of {}", e.group_size));
    }
    if e.override_status.is_some() {
        meta.push_str("  ·  your call");
    }
    ui.label(theme::mono(meta, 10.5, p.muted));
    ui.add_space(8.0);
    let full = ui.available_width();
    let kind = if p.night { Btn::OnDark } else { Btn::OnDay };
    if theme::button(ui, "Keep", Some("P"), Btn::Primary, 48.0, Some(full), true).clicked() {
        rv.set_status(idx, Some(Status::Pick));
    }
    ui.horizontal(|ui| {
        let hw = ((full - 8.0) / 2.0).floor();
        if theme::button(ui, "Maybe", Some("U"), kind, 44.0, Some(hw), true).clicked() {
            rv.set_status(idx, Some(Status::Maybe));
        }
        if theme::button(ui, "Set aside", Some("X"), kind, 44.0, Some(hw), true).clicked() {
            rv.set_status(idx, Some(Status::Reject));
        }
    });
    ui.horizontal(|ui| {
        ui.visuals_mut().override_text_color = Some(p.on_surface);
        let tw = ((full - 16.0) / 3.0).floor();
        if theme::button(ui, "Clear", Some("0"), Btn::MonoSmall, 40.0, Some(tw), e.override_status.is_some()).clicked() {
            rv.set_status(idx, None);
        }
        if theme::button(ui, "Loupe", Some("Spc"), Btn::MonoSmall, 40.0, Some(tw), e.ok).clicked() && rv.open_focus(e.group, Some(idx)) {
            nav = Some(Nav::To(Screen::Focus));
        }
        if theme::button(ui, "Compare", Some("C"), Btn::MonoSmall, 40.0, Some(tw), e.group_size > 1).clicked() && rv.open_compare(e.group, None) {
            nav = Some(Nav::To(Screen::Compare));
        }
    });
    ui.add_space(10.0);
    if e.ok {
        let keeper = rv.keeper_of(e.group);
        let other = if keeper == Some(idx) { rv.runner_up(e.group, idx) } else { keeper };
        let name = if keeper == Some(idx) { "runner-up" } else { "keeper" };
        for (title, v, o, d) in rv.component_rows(idx, other, name) {
            theme::ruler(ui, p, title, v, o, &d);
        }
    }
    if !e.reasons.is_empty() {
        ui.add_space(6.0);
        ui.label(theme::mono(format!("Set aside: {}", e.reasons.join(", ").replace('_', " ")), 10.5, p.mark));
    }
    if let Some(f) = rv.state.files.get(&e.path) {
        if let Some(err) = &f.error {
            ui.label(theme::body(err, 13.0, p.mark));
        }
        ui.add_space(6.0);
        let m = &e.metrics;
        ui.label(
            egui::RichText::new(format!(
                "{} · {} · {}\nsharpness {:.0} · luma {:.2} · contrast {:.2} · noise {:.1}\nclipped {:.1}% highlights, {:.1}% shadows",
                f.format.to_uppercase(),
                f.decoder.as_deref().unwrap_or("-").to_uppercase(),
                f.capture_ms.map(crate::analyze::format_capture).unwrap_or_default(),
                m.sharpness,
                m.mean_luma,
                m.contrast,
                m.noise,
                m.clip_highlights * 100.0,
                m.clip_shadows * 100.0
            ))
            .font(theme::f_mono(10.0))
            .color(p.muted),
        );
    }
    // Same-shot strip.
    let members = rv.members(e.group);
    if members.len() > 1 {
        ui.add_space(12.0);
        ui.label(theme::mono(format!("Same shot · {} frames", members.len()), 10.5, p.muted));
        let cw = ((ui.available_width() - 4.0) / 3.0).floor();
        for chunk in members.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for &m in chunk {
                    let resp = frame_card(ui, rv, app, m, vec2(cw, cw * 0.78), CardOpts { count: 1, selected: m == idx, letter: None, show_score: true, plain: false });
                    if resp.clicked() {
                        rv.selected = Some(m);
                    }
                    if resp.double_clicked() && rv.open_focus(e.group, Some(m)) {
                        nav = Some(Nav::To(Screen::Focus));
                    }
                }
            });
        }
    }
    nav
}
