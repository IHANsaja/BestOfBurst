//! Review: tabs, virtualized grid, detail panel, same-shot strip, full preview, Compare,
//! overrides with undo/redo, and Write picks folder.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Key, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::app::{BestOfBurstApp, BgMsg, Screen, ACCENT, CARD, DEEP, DIM, MAYBE, PICK, REJECT, TEXT};
use super::home::fmt_n;
use super::textures::Loaded;
use crate::output::{self, OutputOptions};
use crate::rank::{self, better, better_in_group, Entry, Keep, Preset, Ranking, Status, PRESETS};
use crate::scan::{self, Format};
use crate::state::State;

const CELL_W: f32 = 196.0;
const CELL_H: f32 = 178.0;
const THUMB_W: f32 = 184.0;
const THUMB_H: f32 = 138.0;
const UNDO_STEPS: usize = 50;
const SAVE_DELAY: Duration = Duration::from_secs(1);
const FULL_UV: Rect = Rect { min: egui::Pos2 { x: 0.0, y: 0.0 }, max: egui::Pos2 { x: 1.0, y: 1.0 } };

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

pub struct Compare {
    pub group: u64,
    pub members: Vec<usize>,
    pub zoom: f32,
    pub center: Vec2,
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
    cols: usize,
    visible_rows: std::ops::Range<usize>,
    scroll_to: Option<usize>,
    undo: Vec<BTreeMap<String, String>>,
    redo: Vec<BTreeMap<String, String>>,
    dirty_since: Option<Instant>,
    pub fullscreen: bool,
    pub compare: Option<Compare>,
    pub zoom_crop: bool,
    pub writing: bool,
    pub write_opts: OutputOptions,
    /// group -> ok members, best group score first.
    groups: HashMap<u64, Vec<usize>>,
    formats: Vec<Format>,
    rerank_ms: f32,
}

/// Fit an image of `size` inside `rect`, centred, keeping its aspect ratio.
pub fn fit(size: Vec2, rect: Rect) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let s = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * s)
}

fn status_color(s: Status) -> Color32 {
    match s {
        Status::Pick => PICK,
        Status::Maybe => MAYBE,
        Status::Reject => REJECT,
        Status::Failed => REJECT.linear_multiply(0.6),
    }
}

fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
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
            .map(|e| {
                state
                    .files
                    .get(&e.path)
                    .and_then(|f| Format::from_name(&f.format))
                    .unwrap_or(Format::Jpeg)
            })
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
            sort: SortBy::Score,
            stacked: true,
            close_only: false,
            expanded: HashSet::new(),
            selected: None,
            items: Vec::new(),
            items_dirty: true,
            cols: 1,
            visible_rows: 0..0,
            scroll_to: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty_since: None,
            fullscreen: false,
            compare: None,
            zoom_crop: false,
            writing: false,
            write_opts: OutputOptions::default(),
            groups: HashMap::new(),
            formats,
            rerank_ms: 0.0,
        };
        r.rebuild_groups();
        r.rebuild_items();
        r.selected = r.items.first().map(|i| i.idx);
        r
    }

    fn entries(&self) -> &[Entry] {
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
            members.sort_by(|&a, &b| {
                e[b].eligible().cmp(&e[a].eligible()).then_with(|| better_in_group(&e[a], &e[b]))
            });
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
                // Keep a selection that is a hidden group member; otherwise fall back.
                let g = self.ranking.entries[sel].group;
                if let Some(it) = self.items.iter().find(|i| self.ranking.entries[i.idx].group == g) {
                    self.selected = Some(it.idx);
                }
            }
        }
    }

    fn tab_count(&self, t: Tab) -> usize {
        let s = &self.ranking.summary;
        match t {
            Tab::Picks => s.picks,
            Tab::Maybe => s.maybe,
            Tab::Rejects => s.rejects + s.failed,
            Tab::All => self.ranking.entries.len(),
        }
    }

    /// Set (or clear) the override on one file. Picking a frame turns the group's previous
    /// pick into Maybe.
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
        rank::apply_overrides(&mut self.ranking.entries, &self.state.overrides);
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

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.state.overrides, prev));
            self.apply_overrides();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.state.overrides, next));
            self.apply_overrides();
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let pos = self
            .selected
            .and_then(|s| self.items.iter().position(|i| i.idx == s))
            .unwrap_or(0) as isize;
        let next = (pos + delta).clamp(0, self.items.len() as isize - 1) as usize;
        self.selected = Some(self.items[next].idx);
        self.scroll_to = Some(next);
    }

    fn close_call_groups(&self) -> Vec<u64> {
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

    fn open_compare(&mut self, group: u64) {
        let Some(m) = self.groups.get(&group) else { return };
        if m.len() < 2 {
            return;
        }
        let members: Vec<usize> = m.iter().take(4).copied().collect();
        let pick = m
            .iter()
            .copied()
            .find(|&i| self.ranking.entries[i].effective() == Status::Pick)
            .unwrap_or(m[0]);
        self.selected = Some(pick);
        self.compare = Some(Compare { group, members, zoom: 1.0, center: vec2(0.5, 0.5) });
        self.fullscreen = false;
    }

    fn next_close_call(&mut self) {
        let groups = self.close_call_groups();
        if groups.is_empty() {
            self.compare = None;
            return;
        }
        let current = self
            .compare
            .as_ref()
            .map(|c| c.group)
            .or_else(|| self.selected.map(|s| self.ranking.entries[s].group));
        let next = match current.and_then(|g| groups.iter().position(|x| *x == g)) {
            Some(p) => groups[(p + 1) % groups.len()],
            None => groups[0],
        };
        self.open_compare(next);
    }

    fn abs_path(&self, idx: usize) -> PathBuf {
        self.input.join(&self.ranking.entries[idx].path)
    }

    fn thumb_path(&self, idx: usize) -> Option<PathBuf> {
        let e = &self.ranking.entries[idx];
        self.state.files.get(&e.path).and_then(|f| f.thumb.as_ref()).map(|t| self.out.join(t))
    }

    fn center_of(&self, idx: usize) -> [f32; 2] {
        let g = self.ranking.entries[idx].group;
        self.ranking.centers.get(&g).copied().unwrap_or([0.5, 0.5])
    }

    /// Save overrides 1 s after the last change (on the saver thread).
    fn maybe_save(&mut self, saver: &super::app::Saver, ctx: &egui::Context) {
        if let Some(t) = self.dirty_since {
            if t.elapsed() >= SAVE_DELAY {
                self.dirty_since = None;
                rank::store(&mut self.state, &self.ranking);
                saver.save(self.state.clone(), self.out.clone());
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }
    }
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let Some(mut rv) = app.review.take() else {
        app.screen = Screen::Home;
        return;
    };
    let mut go_home = false;

    handle_keys(&mut rv, &ctx);
    if rv.items_dirty {
        rv.rebuild_items();
    }

    if let Some(mut cmp) = rv.compare.take() {
        let keep_open = compare_view(&mut rv, &mut cmp, app, ui);
        if keep_open && rv.compare.is_none() {
            rv.compare = Some(cmp);
        }
    } else if rv.fullscreen {
        fullscreen_view(&mut rv, app, ui);
    } else {
        toolbar(&mut rv, app, ui, &mut go_home);
        detail_panel(&mut rv, app, ui);
        strip_panel(&mut rv, app, ui);
        grid(&mut rv, app, ui);
    }

    rv.maybe_save(&app.saver, &ctx);
    if go_home {
        if rv.dirty_since.is_some() {
            rank::store(&mut rv.state, &rv.ranking);
            app.saver.save(rv.state.clone(), rv.out.clone());
        }
        app.screen = Screen::Home;
        if let Some(f) = app.folder.clone() {
            app.choose_folder(f, &ctx);
        }
        app.review = None;
    } else {
        app.review = Some(rv);
    }
}

fn handle_keys(rv: &mut Review, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let (pressed, cmd): (Vec<Key>, bool) = ctx.input(|i| {
        let keys = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            })
            .collect();
        (keys, i.modifiers.command)
    });
    for key in pressed {
        if cmd {
            match key {
                Key::Z => rv.undo(),
                Key::Y => rv.redo(),
                _ => {}
            }
            continue;
        }
        if rv.compare.is_some() {
            match key {
                Key::Escape => rv.compare = None,
                Key::Enter => rv.next_close_call(),
                Key::N => rv.next_close_call(),
                Key::Z => rv.zoom_crop = !rv.zoom_crop,
                Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 => {
                    let k = match key {
                        Key::Num1 => 0,
                        Key::Num2 => 1,
                        Key::Num3 => 2,
                        _ => 3,
                    };
                    let target = rv.compare.as_ref().and_then(|c| c.members.get(k).copied());
                    if let Some(i) = target {
                        rv.set_status(i, Some(Status::Pick));
                        rv.selected = Some(i);
                    }
                }
                _ => {}
            }
            continue;
        }
        let sel = rv.selected;
        match key {
            Key::ArrowLeft => rv.move_selection(-1),
            Key::ArrowRight => rv.move_selection(1),
            Key::ArrowUp => rv.move_selection(if rv.fullscreen { -1 } else { -(rv.cols as isize) }),
            Key::ArrowDown => rv.move_selection(if rv.fullscreen { 1 } else { rv.cols as isize }),
            Key::P => {
                if let Some(i) = sel {
                    rv.set_status(i, Some(Status::Pick));
                }
            }
            Key::U => {
                if let Some(i) = sel {
                    rv.set_status(i, Some(Status::Maybe));
                }
            }
            Key::X => {
                if let Some(i) = sel {
                    rv.set_status(i, Some(Status::Reject));
                }
            }
            Key::Num0 => {
                if let Some(i) = sel {
                    rv.set_status(i, None);
                }
            }
            Key::Space => rv.fullscreen = !rv.fullscreen,
            Key::Escape => rv.fullscreen = false,
            Key::Z if rv.fullscreen => rv.zoom_crop = !rv.zoom_crop,
            Key::G => {
                if let Some(i) = sel {
                    let g = rv.ranking.entries[i].group;
                    if !rv.expanded.remove(&g) {
                        rv.expanded.insert(g);
                    }
                    rv.items_dirty = true;
                }
            }
            Key::Num1 => set_tab(rv, Tab::Picks),
            Key::Num2 => set_tab(rv, Tab::Maybe),
            Key::Num3 => set_tab(rv, Tab::Rejects),
            Key::Num4 => set_tab(rv, Tab::All),
            Key::N => rv.next_close_call(),
            Key::C => {
                if let Some(i) = sel {
                    let g = rv.ranking.entries[i].group;
                    rv.open_compare(g);
                }
            }
            _ => {}
        }
    }
}

fn set_tab(rv: &mut Review, t: Tab) {
    if rv.tab != t {
        rv.tab = t;
        rv.items_dirty = true;
        rv.rebuild_items();
        rv.selected = rv.items.first().map(|i| i.idx);
        rv.scroll_to = Some(0);
    }
}

fn toolbar(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut egui::Ui, go_home: &mut bool) {
    let ctx = ui.ctx().clone();
    egui::Panel::top("review_toolbar").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Home").clicked() {
                *go_home = true;
            }
            ui.separator();
            for (t, name) in [(Tab::Picks, "Picks"), (Tab::Maybe, "Maybe"), (Tab::Rejects, "Rejects"), (Tab::All, "All")] {
                let label = format!("{name} {}", fmt_n(rv.tab_count(t)));
                if ui.selectable_label(rv.tab == t, label).clicked() {
                    set_tab(rv, t);
                }
            }
            ui.separator();
            let n_close = rv.ranking.summary.close_call_groups;
            if ui
                .selectable_label(rv.close_only, format!("Close calls ({n_close})"))
                .on_hover_text("Show only same-shot groups whose top frames score within 5 points. N walks through them.")
                .clicked()
            {
                rv.close_only = !rv.close_only;
                rv.items_dirty = true;
            }
            ui.separator();
            let keep_label = ui.label("Keep");
            let resp = ui
                .add(egui::TextEdit::singleline(&mut rv.keep_text).desired_width(64.0))
                .labelled_by(keep_label.id);
            if resp.changed() {
                if let Some(k) = Keep::parse(&rv.keep_text) {
                    if k != rv.keep {
                        rv.keep = k;
                        rv.rerank();
                    }
                }
            }
            let preset_label = ui.label("Preset");
            let before = rv.preset;
            egui::ComboBox::from_id_salt("review_preset")
                .selected_text(rv.preset.label())
                .width(96.0)
                .show_ui(ui, |ui| {
                    for p in PRESETS {
                        ui.selectable_value(&mut rv.preset, p, p.label());
                    }
                })
                .response
                .labelled_by(preset_label.id);
            if rv.preset != before {
                rv.rerank();
            }
            let sort_label = ui.label("Sort");
            let before = rv.sort;
            egui::ComboBox::from_id_salt("review_sort")
                .selected_text(match rv.sort {
                    SortBy::Score => "Score",
                    SortBy::Time => "Time",
                    SortBy::Name => "Name",
                })
                .width(70.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut rv.sort, SortBy::Score, "Score");
                    ui.selectable_value(&mut rv.sort, SortBy::Time, "Time");
                    ui.selectable_value(&mut rv.sort, SortBy::Name, "Name");
                })
                .response
                .labelled_by(sort_label.id);
            if rv.sort != before {
                rv.items_dirty = true;
            }
            let label = if rv.stacked { "Groups: stacked" } else { "Groups: expanded" };
            if ui.button(label).on_hover_text("G expands or collapses the selected group").clicked() {
                rv.stacked = !rv.stacked;
                rv.expanded.clear();
                rv.items_dirty = true;
            }
        });
        ui.horizontal(|ui| {
            let write = egui::Button::new(RichText::new(if rv.writing { "Writing…" } else { "Write picks folder" }).color(Color32::WHITE)).fill(ACCENT);
            if ui.add_enabled(!rv.writing, write).clicked() {
                start_write(rv, app, &ctx);
            }
            ui.menu_button("Options", |ui| {
                ui.checkbox(&mut rv.write_opts.maybe, "Also build maybe folder");
                ui.checkbox(&mut rv.write_opts.rejects, "Also build rejects folder");
                ui.checkbox(&mut rv.write_opts.copy, "Copy instead of hard links");
                ui.checkbox(&mut rv.write_opts.keep_names, "Keep original names");
                ui.separator();
                ui.horizontal(|ui| {
                    let max_label = ui.label("Max picks per group");
                    let mut m = rv.max_per_group;
                    ui.add(egui::DragValue::new(&mut m).range(1..=10)).labelled_by(max_label.id);
                    if m != rv.max_per_group {
                        rv.max_per_group = m;
                        rv.rerank();
                    }
                });
            });
            if ui.button("Show folder").clicked() {
                let _ = std::process::Command::new("explorer").arg(&rv.out).spawn();
            }
            ui.separator();
            let s = &rv.ranking.summary;
            let mut text = format!(
                "{} groups · target {} · {} overrides",
                fmt_n(s.groups),
                fmt_n(s.target),
                fmt_n(rv.state.overrides.len())
            );
            if rv.rerank_ms > 0.0 {
                text.push_str(&format!(" · re-ranked in {:.0} ms", rv.rerank_ms));
            }
            ui.label(RichText::new(text).color(DIM));
            if let Some((msg, err, _)) = &app.message {
                ui.separator();
                ui.colored_label(if *err { REJECT } else { PICK }, msg);
            }
        });
        ui.add_space(2.0);
    });
}

fn start_write(rv: &mut Review, app: &mut BestOfBurstApp, ctx: &egui::Context) {
    rv.writing = true;
    rank::store(&mut rv.state, &rv.ranking);
    rv.dirty_since = None;
    let mut state = rv.state.clone();
    let entries = rv.ranking.entries.clone();
    let (input, out, opts) = (rv.input.clone(), rv.out.clone(), rv.write_opts);
    let tx = app.bg_tx.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let r = (|| -> anyhow::Result<output::OutputSummary> {
            let s = output::write_folders(&input, &out, &entries, opts)?;
            output::write_report(&out, &state, &entries)?;
            state.save(&out)?;
            Ok(s)
        })();
        let _ = tx.send(BgMsg::Written(r.map_err(|e| format!("{e:#}"))));
        ctx.request_repaint();
    });
}

fn paint_thumb(ui: &egui::Ui, app: &mut BestOfBurstApp, path: Option<PathBuf>, rect: Rect) {
    ui.painter().rect_filled(rect, 4.0, DEEP);
    if let Some(p) = path {
        if let Loaded::Ready(id, size) = app.textures.thumb(&p) {
            ui.painter().image(id, fit(size, rect), FULL_UV, Color32::WHITE);
        }
    }
}

fn badge(ui: &egui::Ui, pos: egui::Pos2, align: Align2, text: &str, fill: Color32, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let size = galley.size() + vec2(8.0, 2.0);
    let rect = align.anchor_size(pos, size);
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter().galley(rect.min + vec2(4.0, 1.0), galley, color);
}

fn thumb_card(ui: &mut egui::Ui, rv: &mut Review, app: &mut BestOfBurstApp, item: GridItem) {
    let (rect, resp) = ui.allocate_exact_size(vec2(CELL_W, CELL_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let e = &rv.ranking.entries[item.idx];
    let selected = rv.selected == Some(item.idx);
    let painter = ui.painter();
    painter.rect_filled(rect.shrink(2.0), 6.0, if resp.hovered() { Color32::from_gray(48) } else { CARD });
    let thumb_rect = Rect::from_min_size(rect.min + vec2((CELL_W - THUMB_W) / 2.0, 6.0), vec2(THUMB_W, THUMB_H));
    let status = e.effective();
    let (score, close, count, reason, ok) =
        (e.score, e.close_call, item.count, e.reasons.first().cloned(), e.ok);
    let name = file_name(&e.path).to_string();
    let overridden = e.override_status.is_some();
    let thumb = rv.thumb_path(item.idx);
    paint_thumb(ui, app, thumb, thumb_rect);

    let tl = thumb_rect.min + vec2(4.0, 4.0);
    if ok {
        badge(ui, tl, Align2::LEFT_TOP, &format!("{score:.0}"), Color32::from_black_alpha(190), TEXT);
    }
    let dot = pos2(thumb_rect.max.x - 10.0, thumb_rect.min.y + 10.0);
    ui.painter().circle_filled(dot, 6.0, status_color(status));
    if overridden {
        ui.painter().circle_stroke(dot, 8.0, Stroke::new(1.5, Color32::WHITE));
    }
    if close {
        badge(ui, pos2(dot.x - 14.0, thumb_rect.min.y + 4.0), Align2::RIGHT_TOP, "?", MAYBE, Color32::BLACK);
    }
    if count > 1 {
        badge(ui, thumb_rect.max - vec2(4.0, 4.0), Align2::RIGHT_BOTTOM, &format!("x{count}"), Color32::from_black_alpha(190), TEXT);
    }
    let text_y = thumb_rect.max.y + 6.0;
    let line = if matches!(status, Status::Reject | Status::Failed) {
        reason.unwrap_or_else(|| "manual".into()).replace('_', " ")
    } else {
        name
    };
    let color = if matches!(status, Status::Reject | Status::Failed) { REJECT } else { DIM };
    let clipped = ui.painter().with_clip_rect(rect.shrink(4.0));
    clipped.text(pos2(rect.min.x + 8.0, text_y), Align2::LEFT_TOP, line, FontId::proportional(12.0), color);
    if selected {
        ui.painter().rect_stroke(rect.shrink(1.0), 6.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
    }
    if resp.clicked() {
        rv.selected = Some(item.idx);
    }
    if resp.double_clicked() {
        rv.selected = Some(item.idx);
        rv.fullscreen = true;
    }
}

fn grid(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        if rv.items.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Nothing here.").color(DIM));
            });
            return;
        }
        let spacing = ui.spacing().item_spacing;
        let cols = ((ui.available_width() / (CELL_W + spacing.x)).floor() as usize).max(1);
        rv.cols = cols;
        let rows = rv.items.len().div_ceil(cols);
        let row_h = CELL_H + spacing.y;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("grid");
        if let Some(pos) = rv.scroll_to.take() {
            let row = pos / cols;
            let vis = rv.visible_rows.clone();
            if !(vis.start < row && row + 1 < vis.end) || vis.is_empty() {
                let target = if row <= vis.start { row as f32 * row_h } else {
                    (row as f32 + 1.0) * row_h - ui.available_height()
                };
                area = area.vertical_scroll_offset(target.max(0.0));
            }
        }
        area.show_rows(ui, CELL_H, rows, |ui, range| {
            rv.visible_rows = range.clone();
            for r in range {
                ui.horizontal(|ui| {
                    for c in 0..cols {
                        if let Some(item) = rv.items.get(r * cols + c).copied() {
                            thumb_card(ui, rv, app, item);
                        }
                    }
                });
            }
        });
    });
}

fn bar(ui: &mut egui::Ui, label: &str, v: f32) {
    ui.horizontal(|ui| {
        ui.add_sized([78.0, 16.0], egui::Label::new(RichText::new(label).color(DIM)));
        let (rect, _) = ui.allocate_exact_size(vec2(190.0, 10.0), Sense::hover());
        ui.painter().rect_filled(rect, 3.0, DEEP);
        let mut fill = rect;
        fill.set_width(rect.width() * v.clamp(0.0, 1.0));
        ui.painter().rect_filled(fill, 3.0, ACCENT);
        ui.label(format!("{:.0}", v * 100.0));
    });
}

fn detail_panel(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    egui::Panel::right("detail").exact_size(390.0).resizable(false).show(ui, |ui| {
        let Some(idx) = rv.selected else {
            ui.label(RichText::new("Select a photo").color(DIM));
            return;
        };
        ui.add_space(6.0);
        let e = rv.ranking.entries[idx].clone();
        ui.label(RichText::new(file_name(&e.path)).size(16.0).strong());
        ui.label(RichText::new(&e.path).color(DIM).small());
        ui.horizontal(|ui| {
            let s = e.effective();
            ui.colored_label(status_color(s), RichText::new(s.name().to_uppercase()).strong());
            if e.ok {
                ui.label(format!("score {:.1}", e.score));
                if e.group_size > 1 {
                    ui.label(format!("group {:.1}", e.group_score));
                }
            }
            if let Some(r) = e.rank {
                ui.label(format!("rank {r}"));
            }
            if e.override_status.is_some() {
                ui.colored_label(MAYBE, "override");
            }
        });

        // Preview (1,600 px), falling back to the thumbnail while it loads.
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, w * 0.72), Sense::click());
        ui.painter().rect_filled(rect, 4.0, DEEP);
        let fmt = rv.formats[idx];
        match app.textures.preview(&rv.abs_path(idx), fmt) {
            Loaded::Ready(id, size) => {
                ui.painter().image(id, fit(size, rect), FULL_UV, Color32::WHITE);
            }
            Loaded::Failed(err) => {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Preview failed", FontId::proportional(14.0), REJECT);
                resp.clone().on_hover_text(err);
            }
            Loaded::Loading => {
                if let Some(t) = rv.thumb_path(idx) {
                    if let Loaded::Ready(id, size) = app.textures.thumb(&t) {
                        ui.painter().image(id, fit(size, rect), FULL_UV, Color32::WHITE);
                    }
                }
            }
        }
        if resp.double_clicked() {
            rv.fullscreen = true;
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let b = |t: &str, c: Color32| egui::Button::new(RichText::new(t).color(Color32::BLACK).strong()).fill(c);
            if ui.add(b("Pick (P)", PICK)).clicked() {
                rv.set_status(idx, Some(Status::Pick));
            }
            if ui.add(b("Maybe (U)", MAYBE)).clicked() {
                rv.set_status(idx, Some(Status::Maybe));
            }
            if ui.add(b("Reject (X)", REJECT)).clicked() {
                rv.set_status(idx, Some(Status::Reject));
            }
            if ui.add_enabled(e.override_status.is_some(), egui::Button::new("Clear (0)")).clicked() {
                rv.set_status(idx, None);
            }
        });
        ui.add_space(6.0);
        if e.ok {
            let labels = ["Sharpness", "Exposure", "Contrast", "Noise", "Color"];
            for (k, l) in labels.iter().enumerate() {
                bar(ui, l, e.components[k]);
            }
        }
        if !e.reasons.is_empty() {
            ui.add_space(4.0);
            ui.colored_label(REJECT, format!("Reasons: {}", e.reasons.join(", ").replace('_', " ")));
        }
        if let Some(f) = rv.state.files.get(&e.path) {
            if let Some(err) = &f.error {
                ui.colored_label(REJECT, err);
            }
            ui.add_space(4.0);
            let m = &e.metrics;
            ui.label(
                RichText::new(format!(
                    "{} · {} · {}\nsharpness {:.0} · luma {:.2} · contrast {:.2} · noise {:.1}\nclipped {:.1}% highlights, {:.1}% shadows",
                    f.format,
                    f.decoder.as_deref().unwrap_or("-"),
                    f.capture_ms.map(crate::analyze::format_capture).unwrap_or_default(),
                    m.sharpness,
                    m.mean_luma,
                    m.contrast,
                    m.noise,
                    m.clip_highlights * 100.0,
                    m.clip_shadows * 100.0
                ))
                .color(DIM)
                .small(),
            );
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new("Space preview · C compare · N next close call · G group · Ctrl+Z undo")
                .color(DIM)
                .small(),
        );
    });
}

fn strip_panel(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let Some(sel) = rv.selected else { return };
    let g = rv.ranking.entries[sel].group;
    let members = rv.groups.get(&g).cloned().unwrap_or_default();
    if members.len() < 2 {
        return;
    }
    egui::Panel::bottom("strip").exact_size(150.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Same shot · {} frames", members.len())).strong());
            if rv.ranking.entries[sel].close_call {
                ui.colored_label(MAYBE, "close call");
            }
            ui.label(RichText::new("click to preview · P makes it the pick · C compares").color(DIM).small());
        });
        egui::ScrollArea::horizontal().id_salt("strip_scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                for &m in &members {
                    let (rect, resp) = ui.allocate_exact_size(vec2(140.0, 110.0), Sense::click());
                    let e = &rv.ranking.entries[m];
                    let (status, gs) = (e.effective(), e.group_score);
                    let thumb = rv.thumb_path(m);
                    let inner = rect.shrink(4.0);
                    paint_thumb(ui, app, thumb, Rect::from_min_size(inner.min, vec2(inner.width(), inner.height() - 16.0)));
                    ui.painter().text(
                        pos2(inner.min.x, inner.max.y - 14.0),
                        Align2::LEFT_TOP,
                        format!("{gs:.1}"),
                        FontId::proportional(12.0),
                        status_color(status),
                    );
                    if status == Status::Pick {
                        ui.painter().rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0, PICK), StrokeKind::Inside);
                    }
                    if m == sel {
                        ui.painter().rect_stroke(rect.shrink(3.5), 4.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        rv.selected = Some(m);
                    }
                    if resp.double_clicked() {
                        rv.fullscreen = true;
                    }
                }
            });
        });
    });
}

/// Paint either the preview (fit) or the 100% subject crop of one frame.
fn paint_big(ui: &egui::Ui, rv: &Review, app: &mut BestOfBurstApp, idx: usize, rect: Rect, uv: Rect) -> Option<String> {
    let path = rv.abs_path(idx);
    let fmt = rv.formats[idx];
    let loaded = if rv.zoom_crop { app.textures.crop(&path, fmt, rv.center_of(idx)) } else { app.textures.preview(&path, fmt) };
    match loaded {
        Loaded::Ready(id, size) => {
            let visible = vec2(size.x * uv.width(), size.y * uv.height());
            let target = if rv.zoom_crop {
                // 100%: one image pixel per screen point when it fits.
                let s = (rect.width() / visible.x).min(rect.height() / visible.y).min(1.0);
                Rect::from_center_size(rect.center(), visible * s)
            } else {
                fit(visible, rect)
            };
            ui.painter().image(id, target, uv, Color32::WHITE);
            None
        }
        Loaded::Loading => {
            if let Some(t) = rv.thumb_path(idx) {
                if !rv.zoom_crop {
                    if let Loaded::Ready(id, size) = app.textures.thumb(&t) {
                        ui.painter().image(id, fit(size, rect), FULL_UV, Color32::WHITE);
                    }
                }
            }
            ui.painter().text(rect.left_bottom() + vec2(8.0, -8.0), Align2::LEFT_BOTTOM, "Loading…", FontId::proportional(13.0), DIM);
            None
        }
        Loaded::Failed(e) => Some(e),
    }
}

fn fullscreen_view(rv: &mut Review, app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let frame = egui::Frame::new().fill(Color32::from_gray(18));
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let rect = ui.max_rect().shrink(12.0);
        let Some(idx) = rv.selected else {
            rv.fullscreen = false;
            return;
        };
        let resp = ui.allocate_rect(rect, Sense::click());
        if let Some(err) = paint_big(ui, rv, app, idx, rect.shrink2(vec2(0.0, 28.0)), FULL_UV) {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, err, FontId::proportional(14.0), REJECT);
        }
        let e = &rv.ranking.entries[idx];
        let s = e.effective();
        ui.painter().text(
            rect.left_top(),
            Align2::LEFT_TOP,
            format!(
                "{}   {}   score {:.1}{}",
                file_name(&e.path),
                s.name().to_uppercase(),
                e.score,
                e.rank.map(|r| format!("   rank {r}")).unwrap_or_default()
            ),
            FontId::proportional(15.0),
            status_color(s),
        );
        ui.painter().text(
            rect.left_bottom(),
            Align2::LEFT_BOTTOM,
            if rv.zoom_crop { "100% crop of the subject · Z fit · arrows move · P/U/X · Esc close" } else { "Z 100% crop · arrows move · P/U/X · Esc close" },
            FontId::proportional(13.0),
            DIM,
        );
        if resp.double_clicked() {
            rv.fullscreen = false;
        }
    });
}

/// Returns false when Compare should close.
fn compare_view(rv: &mut Review, cmp: &mut Compare, app: &mut BestOfBurstApp, ui: &mut egui::Ui) -> bool {
    let mut keep_open = true;
    let frame = egui::Frame::new().fill(Color32::from_gray(18));
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let full = ui.max_rect().shrink(10.0);
        let header = Rect::from_min_size(full.min, vec2(full.width(), 30.0));
        let body = Rect::from_min_max(pos2(full.min.x, header.max.y + 4.0), pos2(full.max.x, full.max.y - 22.0));
        ui.painter().text(
            header.left_center(),
            Align2::LEFT_CENTER,
            format!(
                "Compare · same shot of {} frames{}",
                rv.groups.get(&cmp.group).map(|m| m.len()).unwrap_or(0),
                if rv.ranking.entries[cmp.members[0]].close_call { " · close call" } else { "" }
            ),
            FontId::proportional(16.0),
            TEXT,
        );
        ui.painter().text(
            full.left_bottom(),
            Align2::LEFT_BOTTOM,
            "1–4 make that frame the pick · Enter keep and next close call · N next · Z 100% crop · wheel zoom, drag pan · Esc close",
            FontId::proportional(13.0),
            DIM,
        );

        // Synced zoom and pan.
        let resp = ui.allocate_rect(body, Sense::click_and_drag());
        if resp.hovered() {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                cmp.zoom = (cmp.zoom * (1.0 + scroll * 0.002)).clamp(1.0, 12.0);
            }
        }
        let n = cmp.members.len();
        let pane_w = body.width() / n as f32;
        if resp.dragged() {
            let d = resp.drag_delta();
            cmp.center -= vec2(d.x / pane_w, d.y / body.height()) / cmp.zoom;
        }
        if resp.double_clicked() {
            cmp.zoom = 1.0;
            cmp.center = vec2(0.5, 0.5);
        }
        let half = 0.5 / cmp.zoom;
        cmp.center.x = cmp.center.x.clamp(half, 1.0 - half);
        cmp.center.y = cmp.center.y.clamp(half, 1.0 - half);
        let uv = Rect::from_center_size(pos2(cmp.center.x, cmp.center.y), vec2(1.0 / cmp.zoom, 1.0 / cmp.zoom));

        let members = cmp.members.clone();
        for (k, &m) in members.iter().enumerate() {
            let pane = Rect::from_min_size(pos2(body.min.x + k as f32 * pane_w, body.min.y), vec2(pane_w, body.height())).shrink(4.0);
            let img_rect = pane.shrink2(vec2(0.0, 14.0)).translate(vec2(0.0, 12.0));
            let clip = ui.painter().clip_rect();
            ui.set_clip_rect(pane);
            if let Some(err) = paint_big(ui, rv, app, m, img_rect, uv) {
                ui.painter().text(pane.center(), Align2::CENTER_CENTER, err, FontId::proportional(13.0), REJECT);
            }
            ui.set_clip_rect(clip);
            let e = &rv.ranking.entries[m];
            let s = e.effective();
            ui.painter().text(
                pane.left_top(),
                Align2::LEFT_TOP,
                format!("{}   group {:.1}   score {:.1}   {}", k + 1, e.group_score, e.score, if s == Status::Pick { "PICK" } else { s.name() }),
                FontId::proportional(14.0),
                status_color(s),
            );
            if s == Status::Pick {
                ui.painter().rect_stroke(pane, 4.0, Stroke::new(2.0, PICK), StrokeKind::Inside);
            }
            let hit = ui.interact(pane, ui.scope_id().with(("cmp", k)), Sense::click());
            if hit.clicked() {
                rv.selected = Some(m);
            }
        }
        if ui.ctx().input(|i| i.key_pressed(Key::Escape)) {
            keep_open = false;
        }
    });
    keep_open
}
