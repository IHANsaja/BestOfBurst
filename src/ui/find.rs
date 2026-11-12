//! Find originals: the step after the selection. Drop the photos a client sent back (previews
//! with a watermark, re-saved copies or the originals themselves) and each is matched to its
//! original in this shoot, ready to copy out.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use eframe::egui::{self, pos2, vec2, Align2, Color32, Key, Rect, Sense, Stroke, StrokeKind, Ui};

use super::app::{self, BestOfBurstApp, BgMsg, Screen};
use super::glim::{self, Mood};
use super::marks;
use super::review::{self, run_view, Nav, Review, FULL_UV};
use super::textures::Loaded;
use super::theme::{self, Btn, Pal};
use crate::find::{self, FindReport, FoundSummary, Index, Match, Verdict};
use crate::rank::Status;

const ROW_H: f32 = 112.0;

#[derive(Default)]
pub struct FindState {
    pub running: bool,
    /// Photos matched so far, and how many were sent.
    pub progress: Arc<(AtomicUsize, AtomicUsize)>,
    pub stop: Arc<AtomicBool>,
    pub report: Option<FindReport>,
    /// The shoot's thumbnails, read once and kept for the next batch.
    pub index: Option<Arc<Index>>,
    pub copy: bool,
    pub circle: bool,
    /// Also remove the watermark from the sent previews.
    pub clean: bool,
    /// Also save the originals' file names as a list, with this ending (empty: their own).
    pub list: bool,
    pub list_ending: String,
    pub writing: bool,
    pub delivered: Option<FoundSummary>,
    /// Entry index of each original path, for thumbnails and marks.
    lookup: HashMap<String, usize>,
}

impl FindState {
    pub fn on_found(&mut self, report: FindReport, index: Arc<Index>) {
        self.running = false;
        self.index = Some(index);
        self.report = Some(report);
        self.delivered = None;
    }
}

/// Match `paths` (files and folders) against the shoot on a background thread.
fn start(app: &mut BestOfBurstApp, rv: &Review, paths: Vec<PathBuf>, ctx: &egui::Context) {
    let fs = &mut app.find;
    if fs.running || paths.is_empty() {
        return;
    }
    fs.running = true;
    fs.report = None;
    fs.delivered = None;
    fs.progress = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
    fs.stop = Arc::new(AtomicBool::new(false));
    if fs.lookup.len() != rv.ranking.entries.len() {
        fs.lookup = rv.ranking.entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
    }
    let (progress, stop, index) = (fs.progress.clone(), fs.stop.clone(), fs.index.clone());
    let state = if index.is_none() { Some(rv.state.clone()) } else { None };
    let out = rv.out.clone();
    let (tx, ctx) = (app.bg_tx.clone(), ctx.clone());
    std::thread::spawn(move || {
        let queries = find::collect_queries(&paths);
        progress.1.store(queries.len(), Ordering::Relaxed);
        ctx.request_repaint();
        let index = match (index, state) {
            (Some(i), _) => i,
            (None, Some(s)) => Arc::new(Index::build(&s, &out)),
            (None, None) => return,
        };
        let report = find::find_originals(&index, &queries, Some(&out.join("sent thumbs")), &stop, |done, _| {
            progress.0.store(done, Ordering::Relaxed);
            ctx.request_repaint();
        });
        let _ = tx.send(BgMsg::Found { report, index });
        ctx.request_repaint();
    });
}

fn deliver(app: &mut BestOfBurstApp, rv: &mut Review, ctx: &egui::Context) {
    let Some(report) = &app.find.report else { return };
    if app.find.writing {
        return;
    }
    if app.find.circle {
        let picks: Vec<usize> =
            report.matches.iter().filter_map(|m| m.original()).filter_map(|p| app.find.lookup.get(p).copied()).collect();
        // One circle per frame: the twin of a RAW + JPEG pair is delivered, not circled twice.
        for i in picks {
            if rv.ranking.entries[i].effective() != Status::Pick {
                rv.set_status(i, Some(Status::Pick));
            }
        }
    }
    app.find.writing = true;
    let (input, out, matches, copy) = (rv.input.clone(), rv.out.clone(), report.matches.clone(), app.find.copy);
    let (clean, list) = (app.find.clean, app.find.list.then(|| app.find.list_ending.clone()));
    let (tx, ctx) = (app.bg_tx.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = (|| -> anyhow::Result<FoundSummary> {
            let mut s = find::write_found(&input, &out, &matches, copy)?;
            if let Some(ending) = list {
                let names: Vec<String> = matches.iter().flat_map(|m| m.originals()).map(|n| n.to_string()).collect();
                crate::output::write_name_lists(&out, "client selection", &names, &ending)?;
                s.notes.push("The names are in client selection.txt and client selection.csv".into());
            }
            if clean {
                let stop = AtomicBool::new(false);
                let c = crate::clean::clean_all(&input, &matches, &out.join(crate::clean::CLEAN_DIR), &stop, |_, _| {})?;
                s.notes.push(format!(
                    "{} without the watermark in the \"{}\" folder",
                    theme::plural(c.written, "preview"),
                    crate::clean::CLEAN_DIR
                ));
                if !c.errors.is_empty() {
                    s.notes.push(format!("{} couldn't be cleaned", c.errors.len()));
                }
            }
            Ok(s)
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(BgMsg::FoundWritten(r));
        ctx.request_repaint();
    });
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    run_view(app, ui, view);
}

fn view(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui) -> Nav {
    let ctx = ui.ctx().clone();
    let p = theme::pal(false);
    let mut nav = Nav::Stay;
    let mut go = false;
    if !ctx.egui_wants_keyboard_input() {
        let (keys, cmd) = review::pressed_keys(&ctx);
        for k in keys {
            if cmd && k == Key::E {
                go = true;
            }
            if k == Key::Escape {
                nav = Nav::To(Screen::Review);
            }
        }
    }

    // Anything dropped on this screen is a batch to match.
    let dropped: Vec<PathBuf> =
        ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if !dropped.is_empty() {
        start(app, rv, dropped, &ctx);
    }

    let shoot = app.shoot_name();
    let to_copy = app
        .find
        .report
        .as_ref()
        .map(|r| r.matches.iter().flat_map(|m| m.originals()).collect::<std::collections::HashSet<_>>().len())
        .unwrap_or(0);
    let (mut home, mut back) = (false, false);
    app::chrome_bar(
        ui,
        "find_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, &shoot, true);
            app::text_cell(ui, "Find originals", false);
        },
        |ui| {
            if to_copy > 0 {
                let label =
                    if app.find.writing { "Copying...".to_string() } else { format!("Copy {} originals", theme::thousands(to_copy)) };
                go |= app::red_cell(ui, &label, "Ctrl E", !app.find.writing);
            }
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            back = theme::button(ui, "Back to sheet", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    if home {
        return Nav::Home;
    }
    if back {
        nav = Nav::To(Screen::Review);
    }

    let mut choose: Option<Vec<PathBuf>> = None;
    side_panel(app, rv, ui, p, &mut go, &mut choose);

    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(egui::Margin { left: 48, right: 32, top: 32, bottom: 16 });
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.label(theme::mono("After the selection", 11.0, theme::GRAPHITE));
        ui.label(theme::xp("Find the originals", 34.0, theme::INK));
        ui.add_space(4.0);
        if app.find.running {
            let (done, total) = (app.find.progress.0.load(Ordering::Relaxed), app.find.progress.1.load(Ordering::Relaxed));
            let text = if total == 0 {
                "Reading the shoot's photos...".to_string()
            } else {
                format!("Matching {} of {}...", theme::thousands(done), theme::thousands(total))
            };
            ui.add_space(24.0);
            ui.label(theme::xp(text, 22.0, theme::INK));
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width().min(560.0), 6.0), Sense::hover());
            ui.painter().rect_filled(bar, 0.0, theme::RULE_DAY);
            if total > 0 {
                let w = bar.width() * (done as f32 / total as f32).clamp(0.0, 1.0);
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(w, bar.height())), 0.0, theme::RED);
            }
            ui.add_space(12.0);
            if theme::button(ui, "Stop", None, Btn::OnDay, 40.0, None, true).clicked() {
                app.find.stop.store(true, Ordering::Relaxed);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        } else if app.find.report.is_some() {
            results(app, rv, ui, p, &mut nav);
        } else {
            ui.label(theme::body(
                "Clients often send back the small preview they picked (it downloads faster), sometimes with your watermark on it, sometimes the full photo under another name. Drop whatever they sent, one photo or a whole folder, and BestOfBurst finds the original of each in this shoot.",
                16.0,
                theme::INK_2,
            ));
            ui.add_space(24.0);
            if let Some(paths) = drop_zone(ui, p, hovering) {
                choose = Some(paths);
            }
        }
    });

    if let Some(paths) = choose {
        start(app, rv, paths, &ctx);
    }
    if go && to_copy > 0 {
        deliver(app, rv, &ctx);
    }
    nav
}

fn pick_files() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new().set_title("Choose the photos your client sent").pick_files()
}

fn pick_folder() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new().set_title("Choose the folder your client sent").pick_folder().map(|f| vec![f])
}

fn drop_zone(ui: &mut Ui, p: Pal, hovering: bool) -> Option<Vec<PathBuf>> {
    let mut chosen = None;
    let w = ui.available_width();
    let (zone, resp) = ui.allocate_exact_size(vec2(w, 190.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Choose the photos your client sent"));
    if hovering {
        ui.painter().rect_filled(zone, 0.0, p.mark.gamma_multiply(0.08));
        ui.painter().rect_stroke(zone, 0.0, Stroke::new(2.0, p.mark), StrokeKind::Inside);
    } else {
        let s = Stroke::new(2.0, theme::INK);
        let pts = [zone.left_top(), zone.right_top(), zone.right_bottom(), zone.left_bottom(), zone.left_top()];
        for seg in pts.windows(2) {
            ui.painter().extend(egui::Shape::dashed_line(&[seg[0], seg[1]], s, 10.0, 6.0));
        }
    }
    theme::text_at(
        ui,
        zone.center() - vec2(0.0, 18.0),
        Align2::CENTER_CENTER,
        "DROP THE PHOTOS YOU WERE SENT",
        theme::f_xp(22.0),
        theme::INK,
        0.6,
    );
    theme::text_at(
        ui,
        zone.center() + vec2(0.0, 18.0),
        Align2::CENTER_CENTER,
        "previews, screenshots of a gallery, or full photos · or click to choose",
        theme::f_mono(11.0),
        theme::GRAPHITE,
        0.4,
    );
    theme::focus_ring(ui, &resp);
    if resp.clicked() {
        chosen = pick_files();
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if theme::button(ui, "Choose photos", None, Btn::OnDay, 44.0, None, true).clicked() {
            chosen = pick_files();
        }
        if theme::button(ui, "Choose a folder", None, Btn::OnDay, 44.0, None, true).clicked() {
            chosen = pick_folder();
        }
    });
    chosen
}

fn side_panel(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui, p: Pal, go: &mut bool, choose: &mut Option<Vec<PathBuf>>) {
    let side = egui::Frame::new().fill(theme::PAPER_DAY).inner_margin(28.0);
    egui::Panel::right("find_side").exact_size(420.0).resizable(false).frame(side).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.min.x - 28.0, r.min.y - 28.0), pos2(r.min.x - 28.0, r.max.y + 28.0)], Stroke::new(1.5, theme::INK));
        let (found, check, missing, failed) = match &app.find.report {
            Some(r) => (r.count(Verdict::Found), r.count(Verdict::Check), r.count(Verdict::NotFound), r.count(Verdict::Failed)),
            None => (0, 0, 0, 0),
        };
        let msg = if app.find.running {
            "Looking through the shoot for each one...".to_string()
        } else if app.find.report.is_none() {
            "Send me what your client sent you. I'll find the real photos.".to_string()
        } else if found + check == 0 {
            "None of these look like photos from this shoot.".to_string()
        } else if check > 0 {
            format!("Found {}. {} need a look: two frames of a burst are nearly the same.", theme::plural(found + check, "original"), check)
        } else {
            format!("Found all {}. Copy them out when you're ready.", theme::plural(found, "original"))
        };
        let _ = glim::card(ui, if found + check > 0 { Mood::Cheer } else { Mood::Idle }, false, &msg, 60.0, app.time(), app.reduced_motion);
        ui.add_space(14.0);

        if let Some(report) = &app.find.report {
            let room = (ui.available_height() - 200.0).max(160.0);
            egui::ScrollArea::vertical().max_height(room).auto_shrink([false, true]).show(ui, |ui| {
            for (n, label) in [(found, "found"), (check, "to check"), (missing, "not in this shoot"), (failed, "couldn't be opened")] {
                if n > 0 || label == "found" {
                    ui.horizontal(|ui| {
                        ui.label(theme::xp(theme::thousands(n), 28.0, theme::INK));
                        ui.label(theme::mono(label, 11.0, theme::GRAPHITE));
                    });
                }
            }
            for (shape, wm) in &report.watermarks {
                ui.add_space(6.0);
                ui.label(theme::body(
                    format!(
                        "Watermark spotted on the {} previews (about {:.0}% of the frame). It was left out of the comparison so it can't blur the match.",
                        shape.name(),
                        wm.coverage * 100.0
                    ),
                    13.5,
                    theme::INK_2,
                ));
            }
            ui.add_space(8.0);
            let dir = rv.out.join(find::FOUND_DIR).display().to_string();
            let mut always = true;
            theme::checklist_row(ui, &mut always, true, "Found originals folder", "The original of every photo you were sent, under its own file name, plus a list (found.csv).", Some(&dir));
            theme::checklist_row(ui, &mut app.find.copy, false, "Make full copies", "Normally the folder holds shortcuts that open like the real photos and take no extra space. Turn this on if you'll edit them there.", None);
            theme::checklist_row(ui, &mut app.find.circle, false, "Circle them on the contact sheet", "Mark every found original as a keeper too.", None);
            theme::checklist_row(ui, &mut app.find.clean, false, "Remove the watermark", "A clean copy of each sent preview, rebuilt under the mark from its original, in the \"clean previews\" folder.", None);
            theme::checklist_row(ui, &mut app.find.list, false, "List of file names", "The chosen photos' names in client selection.txt and .csv.", None);
            if app.find.list {
                theme::ending_field(ui, &mut app.find.list_ending);
            }
            });
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let w = ui.available_width();
            if let Some(d) = &app.find.delivered {
                ui.horizontal(|ui| {
                    if theme::button(ui, "Show folder", None, Btn::OnDay, 44.0, None, true).clicked() {
                        let _ = std::process::Command::new("explorer").arg(&d.dir).spawn();
                    }
                });
            }
            app::message_line(app, ui, p);
            if app.find.report.is_some() && !app.find.running {
                let n = found + check;
                let label = if app.find.writing { "Copying...".to_string() } else { "Copy the originals".to_string() };
                if theme::button(ui, &label, Some("Ctrl E"), Btn::Primary, 64.0, Some(w), n > 0 && !app.find.writing).clicked() {
                    *go = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let hw = ((w - 8.0) / 2.0).floor();
                    if theme::button(ui, "More photos", None, Btn::OnDay, 44.0, Some(hw), true).clicked() {
                        *choose = pick_files();
                    }
                    if theme::button(ui, "A folder", None, Btn::OnDay, 44.0, Some(hw), true).clicked() {
                        *choose = pick_folder();
                    }
                });
            }
        });
    });
}

/// A thumbnail fitted into `area` on film.
fn film_thumb(ui: &Ui, area: Rect, loaded: Option<Loaded>, tint: Color32) {
    ui.painter().rect_filled(area, 0.0, theme::FILM);
    if let Some(Loaded::Ready(id, size)) = loaded {
        ui.painter().image(id, theme::fit(size, area.shrink(4.0)), FULL_UV, tint);
    }
}

fn original_thumb(app: &mut BestOfBurstApp, rv: &Review, path: &str) -> Option<Loaded> {
    let idx = *app.find.lookup.get(path)?;
    rv.thumb(&mut app.textures, idx)
}

fn results(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui, p: Pal, nav: &mut Nav) {
    let Some(mut report) = app.find.report.take() else { return };
    if report.matches.is_empty() {
        ui.add_space(24.0);
        ui.label(theme::xp("No photos in what you dropped.", 20.0, theme::GRAPHITE));
    }
    // To-check first, then found, then the rest.
    let rank = |m: &Match| match m.verdict {
        Verdict::Check => 0,
        Verdict::Found => 1,
        Verdict::NotFound => 2,
        Verdict::Failed => 3,
    };
    let mut order: Vec<usize> = (0..report.matches.len()).collect();
    order.sort_by_key(|&i| (rank(&report.matches[i]), report.matches[i].query.clone()));
    ui.add_space(8.0);
    ui.label(theme::mono("Sent photo  →  original in this shoot", 10.5, theme::GRAPHITE));
    theme::rule(ui, theme::INK, 1.5);
    let mut changed = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("found_rows").show_rows(ui, ROW_H, order.len(), |ui, range| {
        for k in range {
            let m = &mut report.matches[order[k]];
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
            ui.painter().line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, theme::RULE_DAY));
            let tw = 132.0;
            let cell = |n: f32| Rect::from_min_size(pos2(row.min.x + n, row.min.y + 8.0), vec2(tw, ROW_H - 16.0));
            let sent = m.thumb.as_ref().map(|t| app.textures.thumb(t, None));
            film_thumb(ui, cell(0.0), sent, Color32::WHITE);
            theme::text_at(ui, pos2(row.min.x + tw + 18.0, row.center().y), Align2::CENTER_CENTER, "→", theme::f_xp(20.0), theme::INK, 0.0);
            let shown = matches!(m.verdict, Verdict::Found | Verdict::Check);
            let orig = cell(tw + 36.0);
            match (&m.best, shown) {
                (Some(b), true) => {
                    let t = original_thumb(app, rv, &b.path);
                    film_thumb(ui, orig, t, Color32::WHITE);
                    if m.verdict == Verdict::Found {
                        marks::keeper(ui, orig.shrink(2.0), theme::SAFELIGHT, 2.6, 1.0);
                    } else {
                        marks::question(ui, pos2(orig.max.x - 4.0, orig.min.y + 2.0), 22.0);
                    }
                }
                _ => {
                    ui.painter().rect_filled(orig, 0.0, theme::FILM);
                    marks::cross(ui, orig.shrink(18.0), theme::TABLE, 2.4, 1.0);
                }
            }
            let tx = row.min.x + tw * 2.0 + 54.0;
            let sent_name = m.query.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let (title, sub) = match m.verdict {
                Verdict::Found | Verdict::Check => {
                    let b = m.best.as_ref();
                    let mut name = b.map(|b| review::file_name(&b.path).to_string()).unwrap_or_default();
                    for t in b.map(|b| b.twins.as_slice()).unwrap_or_default() {
                        name.push_str(&format!(" + {}", review::file_name(t)));
                    }
                    let sim = b.map(|b| b.similarity).unwrap_or(0.0);
                    let how = match (m.verdict == Verdict::Found, b.is_some_and(|b| b.cropped())) {
                        (true, false) => "match",
                        (true, true) => "match (the sent photo is cropped)",
                        (false, false) => "likely match, please check",
                        (false, true) => "likely match of a cropped photo, please check",
                    };
                    (name, format!("{sent_name}  ·  {sim:.0}% alike  ·  {how}"))
                }
                Verdict::NotFound => ("Not in this shoot".to_string(), sent_name),
                Verdict::Failed => {
                    ("Couldn't open this file".to_string(), format!("{sent_name}  ·  {}", m.error.clone().unwrap_or_default()))
                }
            };
            theme::text_at(ui, pos2(tx, row.min.y + 22.0), Align2::LEFT_CENTER, &title, theme::f_xp(16.0), theme::INK, 0.3);
            theme::text_at(ui, pos2(tx, row.min.y + 44.0), Align2::LEFT_CENTER, &sub, theme::f_mono(10.5), theme::GRAPHITE, 0.3);

            // Row actions, right to left: the other burst frame, not it, show on the sheet.
            let x = std::cell::Cell::new(row.max.x);
            let act = |ui: &mut Ui, label: &str, w: f32| -> bool {
                x.set(x.get() - w - 8.0);
                let r = Rect::from_min_size(pos2(x.get(), row.max.y - 46.0), vec2(w, 34.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                    ui.visuals_mut().override_text_color = Some(theme::INK);
                    theme::button(ui, label, None, Btn::MonoSmall, 34.0, Some(w), true).clicked()
                })
                .inner
            };
            if shown {
                if act(ui, "Not it", 84.0) {
                    m.verdict = Verdict::NotFound;
                    changed = true;
                }
                if act(ui, "Show on sheet", 132.0) {
                    if let Some(i) = m.best.as_ref().and_then(|b| app.find.lookup.get(&b.path).copied()) {
                        rv.set_tab(review::Tab::All);
                        rv.reveal(i);
                        *nav = Nav::To(Screen::Review);
                    }
                }
            }
            if m.verdict == Verdict::Check {
                if let Some(r) = m.runner_up.clone().filter(|r| r.similarity >= find::CHECK_MIN) {
                    let other = Rect::from_min_size(pos2(x.get() - 96.0, row.min.y + 8.0), vec2(88.0, ROW_H - 62.0));
                    let t = original_thumb(app, rv, &r.path);
                    film_thumb(ui, other, t, Color32::WHITE);
                    theme::text_at(
                        ui,
                        pos2(other.min.x - 8.0, other.center().y),
                        Align2::RIGHT_CENTER,
                        &format!("or {}", review::file_name(&r.path)),
                        theme::f_mono(10.0),
                        theme::GRAPHITE,
                        0.3,
                    );
                    if act(ui, "Use the other", 132.0) {
                        m.swap();
                        changed = true;
                    }
                }
                if act(ui, "This is it", 108.0) {
                    m.verdict = Verdict::Found;
                    changed = true;
                }
            } else if m.verdict == Verdict::NotFound
                && m.best.as_ref().is_some_and(|b| b.similarity >= find::CHECK_MIN)
                && act(ui, "Undo", 72.0)
            {
                m.verdict = Verdict::Check;
                changed = true;
            }
        }
    });
    if changed {
        app.find.delivered = None;
    }
    let _ = p;
    app.find.report = Some(report);
}
