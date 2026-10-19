//! The one pipeline shared by the CLI and the window: scan, analyze (resumable), rank, output.

// SPEC-GAP: the spec lists nine source files with the run loop in main.rs; the loop lives here
// instead so the window and the CLI call one implementation.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::analyze::{analyze, capture_ms, dhash, fnv1a_hex, write_thumb};
use crate::decode::{self, Decoder};
use crate::output::{self, OutputOptions, OutputSummary};
use crate::rank::{self, Keep, Preset, Ranking};
use crate::scan::{self, Format, ScanItem};
use crate::state::{FileEntry, State};

const SAVE_EVERY: usize = 200;
const SAVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub input: PathBuf,
    pub out: PathBuf,
    pub keep: Keep,
    pub preset: Preset,
    pub threads: usize,
    pub max_per_group: usize,
    pub output: OutputOptions,
    pub retry_failed: bool,
}

impl RunConfig {
    pub fn new(input: PathBuf) -> Self {
        let out = scan::default_out(&input);
        RunConfig {
            input,
            out,
            keep: Keep::default(),
            preset: Preset::General,
            threads: default_threads(),
            max_per_group: 1,
            output: OutputOptions::default(),
            retry_failed: false,
        }
    }
}

/// Logical cores - 1, at least 1.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).max(1)
}

#[derive(Clone, Debug)]
pub enum Event {
    Warning(String),
    Scanned { total: usize, new: usize, already: usize },
    Progress {
        done: usize,
        total: usize,
        per_sec: f32,
        eta_s: Option<u32>,
        last: Option<String>,
        last_thumb: Option<PathBuf>,
    },
    Failed { path: String, error: String },
}

pub struct AnalyzeOutcome {
    pub state: State,
    pub stopped: bool,
    pub total: usize,
    pub new: usize,
    pub failed_now: usize,
}

fn primary_decoder(format: Format) -> Decoder {
    match format {
        Format::Jpeg => Decoder::ZuneJpeg,
        Format::Jxl => Decoder::JxlOxide,
        Format::Raw => Decoder::RawPreview,
        Format::Heif | Format::Avif | Format::Jxr => Decoder::Wic,
        _ => Decoder::Image,
    }
}

fn analyze_one(item: &ScanItem, out: &Path) -> Result<FileEntry> {
    let d = decode::decode_for_analysis(&item.abs, item.format)?;
    let (metrics, tiles) = analyze(&d.image);
    let hash = dhash(&d.image);
    let captured = capture_ms(&item.abs, item.format, item.mtime_ms);
    let id = fnv1a_hex(&item.rel);
    let thumb_path = out.join("thumbs").join(format!("{id}.jpg"));
    let thumb = write_thumb(&d.image, &thumb_path).ok().map(|_| format!("thumbs\\{id}.jpg"));
    Ok(FileEntry {
        size: item.size,
        mtime_ms: item.mtime_ms,
        status: "done".into(),
        format: item.format.name().into(),
        decoder: Some(d.decoder.name().into()),
        capture_ms: Some(captured),
        dhash: Some(format!("{hash:016x}")),
        metrics: Some(metrics),
        error: None,
        thumb,
        tiles: Some(tiles.to_vec()),
    })
}

/// Decode and analyze one file. A decoder error or panic is recorded as `failed`.
pub fn process(item: &ScanItem, out: &Path) -> FileEntry {
    let failed = |error: String| FileEntry {
        size: item.size,
        mtime_ms: item.mtime_ms,
        status: "failed".into(),
        format: item.format.name().into(),
        decoder: Some(primary_decoder(item.format).name().into()),
        capture_ms: None,
        dhash: None,
        metrics: None,
        error: Some(error),
        thumb: None,
        tiles: None,
    };
    match catch_unwind(AssertUnwindSafe(|| analyze_one(item, out))) {
        Ok(Ok(entry)) => entry,
        Ok(Err(e)) => failed(format!("{e:#}").split_whitespace().collect::<Vec<_>>().join(" ")),
        Err(_) => failed("decoder crashed".into()),
    }
}

/// Scan and analyze every new or changed file, saving state every 200 files or 15 seconds.
/// Returns early (with state saved) when `stop` is set.
pub fn analyze_folder(cfg: &RunConfig, stop: &Arc<AtomicBool>, mut on_event: impl FnMut(Event)) -> Result<AnalyzeOutcome> {
    output::check_out(&cfg.input, &cfg.out)?;
    if !cfg.input.is_dir() {
        anyhow::bail!("{} is not a folder", cfg.input.display());
    }
    std::fs::create_dir_all(&cfg.out).with_context(|| format!("cannot create {}", cfg.out.display()))?;
    let (mut state, warning) = State::load(&cfg.input, &cfg.out)?;
    if let Some(w) = warning {
        on_event(Event::Warning(w));
    }

    let items = scan::scan(&cfg.input, &cfg.out, Some(stop));
    if stop.load(Ordering::Relaxed) {
        // A partial scan must not drop entries of files it never reached.
        return Ok(AnalyzeOutcome { state, stopped: true, total: items.len(), new: 0, failed_now: 0 });
    }
    // Entries for files that no longer exist are dropped.
    let present: std::collections::HashSet<&str> = items.iter().map(|i| i.rel.as_str()).collect();
    state.files.retain(|k, _| present.contains(k.as_str()));
    state.overrides.retain(|k, _| present.contains(k.as_str()));

    let todo: Vec<ScanItem> = items
        .iter()
        .filter(|i| {
            !state.is_current(&i.rel, i.size, i.mtime_ms)
                || (cfg.retry_failed && state.files.get(&i.rel).map(|e| e.status == "failed").unwrap_or(false))
        })
        .cloned()
        .collect();
    let total_new = todo.len();
    on_event(Event::Scanned { total: items.len(), new: total_new, already: items.len() - total_new });

    let mut outcome = AnalyzeOutcome { state, stopped: false, total: items.len(), new: total_new, failed_now: 0 };
    if todo.is_empty() || stop.load(Ordering::Relaxed) {
        outcome.stopped = stop.load(Ordering::Relaxed);
        outcome.state.save(&cfg.out)?;
        return Ok(outcome);
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cfg.threads.max(1))
        .thread_name(|i| format!("bob-analyze-{i}"))
        .build()
        .context("cannot start worker threads")?;

    // Decoder panics are caught per file; keep their default messages off the console.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let (tx, rx) = mpsc::channel::<(ScanItem, FileEntry)>();
    let worker_stop = stop.clone();
    let out_dir = cfg.out.clone();
    let worker = std::thread::spawn(move || {
        pool.install(|| {
            todo.par_iter().for_each_with(tx, |tx, item| {
                if worker_stop.load(Ordering::Relaxed) {
                    return;
                }
                let entry = process(item, &out_dir);
                let _ = tx.send((item.clone(), entry));
            });
        });
    });

    let started = Instant::now();
    let mut last_save = Instant::now();
    let mut since_save = 0usize;
    let mut done = 0usize;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok((item, entry)) => {
                done += 1;
                since_save += 1;
                let failed = !entry.is_done();
                if failed {
                    outcome.failed_now += 1;
                    on_event(Event::Failed {
                        path: item.rel.clone(),
                        error: entry.error.clone().unwrap_or_default(),
                    });
                }
                let thumb = entry.thumb.as_ref().map(|t| cfg.out.join(t));
                outcome.state.files.insert(item.rel.clone(), entry);
                let secs = started.elapsed().as_secs_f32().max(0.001);
                let per_sec = done as f32 / secs;
                let eta_s = (per_sec > 0.0).then(|| ((total_new - done) as f32 / per_sec).round() as u32);
                on_event(Event::Progress {
                    done,
                    total: total_new,
                    per_sec,
                    eta_s,
                    last: Some(item.rel),
                    last_thumb: thumb,
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if since_save >= SAVE_EVERY || (since_save > 0 && last_save.elapsed() >= SAVE_INTERVAL) {
            outcome.state.save(&cfg.out)?;
            since_save = 0;
            last_save = Instant::now();
        }
    }
    let _ = worker.join();
    std::panic::set_hook(prev_hook);

    outcome.stopped = stop.load(Ordering::Relaxed) && done < total_new;
    outcome.state.save(&cfg.out)?;
    Ok(outcome)
}

/// Rank from state, write `picks` (and optional folders), `report.csv`, and save the state.
pub fn rank_and_output(state: &mut State, cfg: &RunConfig) -> Result<(Ranking, OutputSummary)> {
    let ranking = rank::rank(state, cfg.preset, cfg.keep, cfg.max_per_group);
    rank::store(state, &ranking);
    let summary = output::write_folders(&cfg.input, &cfg.out, &ranking.entries, cfg.output)?;
    output::write_report(&cfg.out, state, &ranking.entries)?;
    state.save(&cfg.out)?;
    Ok((ranking, summary))
}
