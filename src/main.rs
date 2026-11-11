// Console subsystem on purpose: the CLI prints normally, and a double-click (no arguments)
// detaches from the console with FreeConsole before opening the window.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use bestofburst::engine::{self, Event, RunConfig};
use bestofburst::output::OutputOptions;
use bestofburst::rank::{Keep, Preset, RankSummary};
use bestofburst::scan;
use bestofburst::state::State;
use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

const HARDLINK_NOTE: &str = "\
Hard links: by default the picks are hard links. A hard link is the same file as the original.
Deleting it from `picks` never deletes the original, but an editor that saves *in place* over a
file in `picks` changes the original too. Use --copy if you will edit pixels directly.";

#[derive(Parser)]
#[command(name = "bestofburst", version, about = "Pick the best shots from a folder of photos.", after_help = HARDLINK_NOTE)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan, analyze, rank and write the picks folder (resumes where it stopped).
    Run(RunArgs),
    /// Re-rank from state.json only and rebuild the output folders.
    Rank(RankArgs),
    /// Counts from state.json.
    Status {
        folder: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Open the window (double-clicking bestofburst.exe does the same).
    Ui { folder: Option<PathBuf> },
    /// Format, decoder, metrics and timing for one file.
    Probe { file: PathBuf },
    /// Find the originals of photos someone sent back (watermarked previews, re-saved copies).
    Find(FindArgs),
    /// Remove the watermark from previews, using the originals they were made from.
    Clean {
        /// The folder with the originals.
        originals: PathBuf,
        /// The watermarked photos: files and/or folders.
        #[arg(required = true)]
        sent: Vec<PathBuf>,
        /// Where the cleaned photos go [default: a `watermark removed` folder beside the first sent photo].
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Remove a watermark without the originals: the mark is learned from the watermarked
    /// photos themselves (give at least 3 different photos carrying the same mark).
    Unmark {
        /// The watermarked photos: files and/or folders.
        #[arg(required = true)]
        sent: Vec<PathBuf>,
        /// Where the cleaned photos go [default: a `watermark removed` folder beside the first photo].
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Args)]
struct FindArgs {
    /// The shoot folder (already culled with `run`).
    folder: PathBuf,
    /// The photos that were sent: files and/or folders.
    #[arg(required = true)]
    sent: Vec<PathBuf>,
    /// Output and state folder [default: <FOLDER>\_bestofburst].
    #[arg(long)]
    out: Option<PathBuf>,
    /// Copy the originals instead of hard-linking them.
    #[arg(long)]
    copy: bool,
    /// Only list the matches; do not fill the `found originals` folder.
    #[arg(long)]
    list: bool,
    /// Also remove the watermark from the sent previews (into `clean previews`).
    #[arg(long)]
    clean: bool,
    /// Also write the originals' names to `client selection.txt` and `.csv`, with this file
    /// ending (`same` keeps each photo's own, `none` leaves it off, or e.g. `nef`).
    #[arg(long, value_name = "ENDING")]
    names: Option<String>,
}

#[derive(Args, Clone)]
struct RankOpts {
    /// Exact number of picks.
    #[arg(long, conflicts_with = "keep_percent")]
    keep: Option<usize>,
    /// Picks as a percentage of analyzed files, rounded up [default: 10].
    #[arg(long)]
    keep_percent: Option<f64>,
    /// Weight preset.
    #[arg(long, default_value = "general", value_parser = ["general", "portrait", "sports", "landscape"])]
    preset: String,
    /// Output and state folder [default: <FOLDER>\_bestofburst].
    #[arg(long)]
    out: Option<PathBuf>,
    /// Most picks allowed from one same-shot group.
    #[arg(long, default_value_t = 1)]
    max_per_group: usize,
    /// Also build the `maybe` folder.
    #[arg(long)]
    maybe: bool,
    /// Also build the `rejects` folder (one subfolder per first reason).
    #[arg(long)]
    rejects: bool,
    /// Copy files instead of hard-linking them.
    #[arg(long)]
    copy: bool,
    /// Keep original file names instead of rank prefixes.
    #[arg(long)]
    keep_names: bool,
}

#[derive(Args)]
struct RunArgs {
    folder: PathBuf,
    #[command(flatten)]
    opts: RankOpts,
    /// Analysis threads [default: logical cores - 1].
    #[arg(long)]
    threads: Option<usize>,
    /// Re-analyze files marked failed (useful after installing a codec).
    #[arg(long)]
    retry_failed: bool,
    /// Analysis hardware: auto (GPU when one works), gpu, or cpu.
    #[arg(long, default_value = "auto", value_parser = ["auto", "gpu", "cpu"])]
    engine: String,
    /// Skip the closed-eyes check (faster; for shoots without people).
    #[arg(long)]
    no_eyes: bool,
}

#[derive(Args)]
struct RankArgs {
    folder: PathBuf,
    #[command(flatten)]
    opts: RankOpts,
}

fn config(folder: &Path, o: &RankOpts) -> Result<RunConfig> {
    let input = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
    let mut cfg = RunConfig::new(input.clone());
    if let Some(out) = &o.out {
        cfg.out = std::path::absolute(out).unwrap_or_else(|_| out.clone());
    }
    cfg.keep = match (o.keep, o.keep_percent) {
        (Some(n), _) => Keep::Count(n),
        (None, Some(p)) => Keep::Percent(p),
        (None, None) => Keep::default(),
    };
    cfg.preset = Preset::parse(&o.preset).unwrap_or_default();
    cfg.max_per_group = o.max_per_group.max(1);
    cfg.output = OutputOptions { maybe: o.maybe, rejects: o.rejects, copy: o.copy, keep_names: o.keep_names };
    Ok(cfg)
}

fn main() -> ExitCode {
    if std::env::args_os().len() <= 1 {
        hide_console();
        return report(bestofburst::ui::run(None));
    }
    let cli = Cli::parse();
    let result = match cli.cmd {
        None => bestofburst::ui::run(None),
        Some(Cmd::Ui { folder }) => bestofburst::ui::run(folder),
        Some(Cmd::Run(a)) => cmd_run(a),
        Some(Cmd::Rank(a)) => cmd_rank(a),
        Some(Cmd::Status { folder, out }) => cmd_status(&folder, out),
        Some(Cmd::Probe { file }) => cmd_probe(&file),
        Some(Cmd::Find(a)) => cmd_find(a),
        Some(Cmd::Clean { originals, sent, out }) => cmd_clean(&originals, &sent, out),
        Some(Cmd::Unmark { sent, out }) => cmd_unmark(&sent, out),
    };
    report(result)
}

fn report(r: Result<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn hide_console() {
    // SAFETY: detaching from the console has no preconditions.
    unsafe {
        let _ = windows::Win32::System::Console::FreeConsole();
    }
}

pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn clock(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn elapsed(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn cmd_run(a: RunArgs) -> Result<()> {
    let mut cfg = config(&a.folder, &a.opts)?;
    if let Some(t) = a.threads {
        cfg.threads = t.max(1);
    }
    cfg.retry_failed = a.retry_failed;
    cfg.eyes = !a.no_eyes;
    cfg.engine = engine::EngineChoice::parse(&a.engine).unwrap_or_default();
    let started = Instant::now();

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }

    println!("Scanning...");
    let bar = ProgressBar::hidden();
    bar.set_style(
        ProgressStyle::with_template("[{bar:30}] {pos}/{len}  {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#-"),
    );
    let mut codec_missing = false;
    let outcome = engine::analyze_folder(&cfg, &stop, |ev| match ev {
        Event::Warning(w) => eprintln!("{w}"),
        Event::Engine(e) => println!("Engine: {e}"),
        Event::Scanned { total, new, already } => {
            println!(
                "Scanning... {} images ({} new, {} already analyzed)",
                thousands(total),
                thousands(new),
                thousands(already)
            );
            if new > 0 {
                bar.set_length(new as u64);
                bar.set_draw_target(indicatif::ProgressDrawTarget::stderr());
            }
        }
        Event::Progress { done, per_sec, eta_s, .. } => {
            bar.set_position(done as u64);
            bar.set_message(format!(
                "{:.0} img/s  ETA {}",
                per_sec,
                eta_s.map(clock).unwrap_or_else(|| "-".into())
            ));
        }
        Event::Failed { error, .. } => {
            if !codec_missing && bestofburst::decode::wic::is_missing_codec(&error) {
                codec_missing = true;
                bar.println(format!("Note: {error}"));
            }
        }
    })?;
    bar.finish_and_clear();

    if outcome.stopped {
        println!("Stopped. Run the same command to resume.");
        return Ok(());
    }

    let mut state = outcome.state;
    let (ranking, out_sum) = engine::rank_and_output(&mut state, &cfg)?;
    println!(
        "Ranking... {} burst groups, {} rejected",
        thousands(ranking.summary.groups),
        thousands(ranking.summary.rejects)
    );
    if out_sum.copied_fallback {
        println!("Hard links not supported here; copying files.");
    }
    println!("Done in {}", elapsed(started.elapsed().as_secs()));
    let st = bestofburst::stats::STATS.view();
    if st.gpu_photos + st.cpu_photos > 0 {
        println!(
            "  Engine:  GPU {} photos ({:.0} ms each), CPU {} photos ({:.0} ms each), decode {:.0} ms each{}",
            thousands(st.gpu_photos as usize),
            st.gpu_ms_avg,
            thousands(st.cpu_photos as usize),
            st.cpu_ms_avg,
            st.decode_ms_avg,
            if st.gpu_fallbacks > 0 { format!(", {} GPU fallbacks", st.gpu_fallbacks) } else { String::new() }
        );
    }
    print_summary(&ranking.summary, &out_sum.picks_dir, &cfg);
    for e in out_sum.errors.iter().take(5) {
        eprintln!("  could not place {e}");
    }
    Ok(())
}

fn print_summary(s: &RankSummary, picks_dir: &Path, cfg: &RunConfig) {
    let picks = if s.picks < s.target && cfg.keep != Keep::Count(0) {
        format!("{} of {} requested", thousands(s.picks), thousands(s.target))
    } else if s.picks > s.target {
        format!("{} ({} requested, plus one from every pose shot 3+ times)", thousands(s.picks), thousands(s.target))
    } else {
        thousands(s.picks)
    };
    println!("  Picks:   {picks}  -> {}", picks_dir.display());
    println!("  Maybe:   {}", thousands(s.maybe));
    let mut reasons: Vec<(&String, &usize)> = s.reject_reasons.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut parts: Vec<String> = reasons
        .iter()
        .take(3)
        .map(|(r, n)| format!("{} {}", r.replace('_', " "), thousands(**n)))
        .collect();
    let other: usize = reasons.iter().skip(3).map(|(_, n)| **n).sum();
    if other > 0 {
        parts.push(format!("other {}", thousands(other)));
    }
    if parts.is_empty() {
        println!("  Rejects: {}", thousands(s.rejects));
    } else {
        println!("  Rejects: {}  ({})", thousands(s.rejects), parts.join(", "));
    }
    if s.failed > 0 {
        println!("  Failed:  {}     (see report.csv)", thousands(s.failed));
    }
}

fn cmd_rank(a: RankArgs) -> Result<()> {
    let cfg = config(&a.folder, &a.opts)?;
    let started = Instant::now();
    let Some((mut state, warning)) = State::load_existing(&cfg.input, &cfg.out)? else {
        bail!("No state.json in {}. Run `bestofburst run` first.", cfg.out.display());
    };
    if let Some(w) = warning {
        eprintln!("{w}");
    }
    let (ranking, out_sum) = engine::rank_and_output(&mut state, &cfg)?;
    println!(
        "Ranking... {} burst groups, {} rejected",
        thousands(ranking.summary.groups),
        thousands(ranking.summary.rejects)
    );
    if out_sum.copied_fallback {
        println!("Hard links not supported here; copying files.");
    }
    println!("Done in {}", elapsed(started.elapsed().as_secs()));
    print_summary(&ranking.summary, &out_sum.picks_dir, &cfg);
    Ok(())
}

fn cmd_find(a: FindArgs) -> Result<()> {
    use bestofburst::find::{self, Verdict};
    let input = std::path::absolute(&a.folder).unwrap_or_else(|_| a.folder.clone());
    let out = a.out.clone().unwrap_or_else(|| scan::default_out(&input));
    let Some((state, warning)) = State::load_existing(&input, &out)? else {
        bail!("No state.json in {}. Run `bestofburst run` on the shoot first.", out.display());
    };
    if let Some(w) = warning {
        eprintln!("{w}");
    }
    let started = Instant::now();
    let queries = find::collect_queries(&a.sent);
    if queries.is_empty() {
        bail!("No photos found in what was given.");
    }
    println!("Reading the shoot...");
    let index = find::Index::build(&state, &out);
    if index.is_empty() {
        bail!("The shoot has no analyzed photos with thumbnails. Run `bestofburst run` first.");
    }
    println!("Matching {} sent photos against {} originals...", thousands(queries.len()), thousands(index.len()));
    let stop = AtomicBool::new(false);
    let report = find::find_originals(&index, &queries, None, &stop, |_, _| {});
    for (shape, wm) in &report.watermarks {
        println!(
            "Watermark spotted on the {} previews (about {:.0}% of the frame, learned from {} matches); left out of the comparison.",
            shape.name(),
            wm.coverage * 100.0,
            wm.learned_from
        );
    }
    for m in &report.matches {
        let sent = m.query.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        match m.verdict {
            Verdict::Found | Verdict::Check => {
                let b = m.best.as_ref();
                let mut line = format!(
                    "  {:<9} {sent}  ->  {}  ({:.0}% alike)",
                    if m.verdict == Verdict::Found { "found" } else { "check" },
                    m.originals().join(" + "),
                    b.map(|b| b.similarity).unwrap_or(0.0)
                );
                if m.verdict == Verdict::Check {
                    if let Some(r) = m.runner_up.as_ref().filter(|r| r.similarity >= find::CHECK_MIN) {
                        line.push_str(&format!("   or {} ({:.0}%)", r.path, r.similarity));
                    }
                }
                println!("{line}");
            }
            Verdict::NotFound => println!("  not found {sent}"),
            Verdict::Failed => println!("  failed    {sent}: {}", m.error.clone().unwrap_or_default()),
        }
    }
    println!(
        "Found {}, to check {}, not in this shoot {}, couldn't open {}  ({})",
        thousands(report.count(Verdict::Found)),
        thousands(report.count(Verdict::Check)),
        thousands(report.count(Verdict::NotFound)),
        thousands(report.count(Verdict::Failed)),
        elapsed(started.elapsed().as_secs())
    );
    if !a.list {
        let s = find::write_found(&input, &out, &report.matches, a.copy)?;
        if s.copied_fallback {
            println!("Hard links not supported here; copying files.");
        }
        println!("  Originals: {}  -> {}", thousands(s.written), s.dir.display());
        println!("  List:      {}", out.join("found.csv").display());
        for e in s.errors.iter().take(5) {
            eprintln!("  could not place {e}");
        }
    }
    if let Some(ending) = &a.names {
        let names: Vec<String> = report.matches.iter().flat_map(|m| m.originals()).map(|s| s.to_string()).collect();
        let ending = if ending.eq_ignore_ascii_case("same") { "" } else { ending.as_str() };
        let (txt, csv) = bestofburst::output::write_name_lists(&out, "client selection", &names, ending)?;
        println!("  Names:     {} and {}", txt.display(), csv.display());
    }
    if a.clean {
        let s = bestofburst::clean::clean_all(&input, &report.matches, &out.join(bestofburst::clean::CLEAN_DIR), &stop, |_, _| {})?;
        println!("  Cleaned:   {}  -> {}", thousands(s.written), s.dir.display());
        for (name, e) in s.errors.iter().take(5) {
            eprintln!("  {name}: {e}");
        }
    }
    Ok(())
}

fn cmd_clean(originals: &Path, sent: &[PathBuf], out: Option<PathBuf>) -> Result<()> {
    use bestofburst::find;
    let input = std::path::absolute(originals).unwrap_or_else(|_| originals.to_path_buf());
    if !input.is_dir() {
        bail!("{} is not a folder", input.display());
    }
    let queries = find::collect_queries(sent);
    if queries.is_empty() {
        bail!("No photos found in what was given.");
    }
    let started = Instant::now();
    let stop = AtomicBool::new(false);
    println!("Reading the originals...");
    let index = find::Index::open(&input, &stop, |_, _| {});
    if index.is_empty() {
        bail!("No photos could be opened in {}.", input.display());
    }
    println!("Matching {} photos against {} originals...", thousands(queries.len()), thousands(index.len()));
    let report = find::find_originals(&index, &queries, None, &stop, |_, _| {});
    let dir = out.unwrap_or_else(|| bestofburst::ui::remover::default_dir(&queries));
    println!("Removing watermarks...");
    let s = bestofburst::clean::clean_all(&input, &report.matches, &dir, &stop, |_, _| {})?;
    println!("Cleaned {} of {}  -> {}  ({})", thousands(s.written), thousands(queries.len()), s.dir.display(), elapsed(started.elapsed().as_secs()));
    if s.unmatched > 0 {
        println!("  {} had no sure original in that folder and were left alone.", thousands(s.unmatched));
    }
    for (name, e) in s.errors.iter().take(8) {
        eprintln!("  {name}: {e}");
    }
    Ok(())
}

fn cmd_unmark(sent: &[PathBuf], out: Option<PathBuf>) -> Result<()> {
    let queries = bestofburst::find::collect_queries(sent);
    if queries.is_empty() {
        bail!("No photos found in what was given.");
    }
    let started = Instant::now();
    let stop = AtomicBool::new(false);
    let stage = std::sync::atomic::AtomicUsize::new(0);
    let dir = out.unwrap_or_else(|| bestofburst::ui::remover::default_dir(&queries));
    println!("Learning the watermark from {} photos, then removing it...", thousands(queries.len()));
    let (mark, s) = bestofburst::unmark::unmark_all(&queries, &dir, &stop, &stage, |_, _| {})?;
    println!(
        "  Mark: covers {:.1}% of the frame, {:.0}% opaque, colour {:.0} {:.0} {:.0} (learned from {} photos)",
        mark.coverage * 100.0,
        (mark.alpha[0] + mark.alpha[1] + mark.alpha[2]) / 3.0 * 100.0,
        mark.colour[0],
        mark.colour[1],
        mark.colour[2],
        mark.learned_from
    );
    println!("Cleaned {} of {}  -> {}  ({})", thousands(s.written), thousands(queries.len()), s.dir.display(), elapsed(started.elapsed().as_secs()));
    if s.unmatched > 0 {
        println!("  {} did not carry the mark where the others do and were left alone.", thousands(s.unmatched));
    }
    for (name, e) in s.errors.iter().take(8) {
        eprintln!("  {name}: {e}");
    }
    Ok(())
}

fn cmd_status(folder: &Path, out: Option<PathBuf>) -> Result<()> {
    let input = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
    let out = out.unwrap_or_else(|| scan::default_out(&input));
    let Some((state, warning)) = State::load_existing(&input, &out)? else {
        println!("No state.json in {}.", out.display());
        return Ok(());
    };
    if let Some(w) = warning {
        eprintln!("{w}");
    }
    let (done, failed) = state.counts();
    println!("State:      {}", State::path(&out).display());
    println!("Updated:    {}", state.updated);
    println!("Files:      {}", thousands(state.files.len()));
    println!("  Analyzed: {}", thousands(done));
    println!("  Failed:   {}", thousands(failed));
    if let Some(lr) = &state.last_rank {
        let count = |s: &str| lr.results.values().filter(|r| r.status == s).count();
        println!("Last rank:  {} (preset {}, target {}, max {} per group)", lr.ranked_at, lr.preset, lr.target, lr.max_per_group);
        println!("  Picks:    {}", thousands(count("pick")));
        println!("  Maybe:    {}", thousands(count("maybe")));
        println!("  Rejects:  {}", thousands(count("reject")));
        println!("  Groups:   {}", thousands(lr.groups.len()));
        let close: std::collections::BTreeSet<u64> =
            lr.results.values().filter(|r| r.close_call).map(|r| r.group).collect();
        println!("  Close calls: {}", thousands(close.len()));
    } else {
        println!("Last rank:  none");
    }
    println!("Overrides:  {}", thousands(state.overrides.len()));
    Ok(())
}

fn cmd_probe(file: &Path) -> Result<()> {
    use bestofburst::analyze;
    use bestofburst::decode;
    let meta = std::fs::metadata(file)?;
    let Some(format) = scan::detect_file(file)? else {
        bail!("{}: not a supported image", file.display());
    };
    println!("File:      {}", file.display());
    println!("Size:      {} bytes", thousands(meta.len() as usize));
    println!("Format:    {}", format.name());
    let t0 = Instant::now();
    let decoded = match decode::decode_full(file, format) {
        Ok(d) => d,
        Err(e) => {
            println!("Decode:    FAILED after {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
            println!("Error:     {e:#}");
            return Ok(());
        }
    };
    let t_decode = t0.elapsed();
    println!("Decoder:   {}", decoded.decoder.name());
    println!("Pixels:    {} x {}", decoded.image.width, decoded.image.height);
    if decoded.image.width < decode::MIN_EDGE || decoded.image.height < decode::MIN_EDGE {
        println!("Error:     too small");
        return Ok(());
    }
    let t1 = Instant::now();
    let img = decode::resize_long_edge(decoded.image, decode::ANALYSIS_EDGE)?;
    let t_resize = t1.elapsed();
    let t2 = Instant::now();
    let (m, tiles) = analyze::analyze(&img);
    let hash = analyze::dhash(&img);
    let t_analyze = t2.elapsed();
    let cap = analyze::capture_ms(file, format, scan::mtime_ms(&meta));
    println!("Analyzed:  {} x {}", img.width, img.height);
    println!("Captured:  {}", analyze::format_capture(cap));
    println!("dHash:     {hash:016x}");
    println!("Metrics:");
    println!("  sharpness       {:.1}", m.sharpness);
    println!("  tenengrad       {:.1}", m.tenengrad);
    println!("  mean_luma       {:.3}", m.mean_luma);
    println!("  clip_shadows    {:.4}", m.clip_shadows);
    println!("  clip_highlights {:.4}", m.clip_highlights);
    println!("  contrast        {:.3}", m.contrast);
    println!("  noise           {:.2}", m.noise);
    println!("  colorfulness    {:.1}", m.colorfulness);
    println!("  exposure score  {:.3}", bestofburst::rank::exposure(&m));
    println!("Tile sharpness (8 x 8):");
    for row in tiles.chunks(8) {
        println!("  {}", row.iter().map(|v| format!("{v:>8.0}")).collect::<String>());
    }
    println!(
        "Timing:    decode {:.0} ms, resize {:.0} ms, analyze {:.0} ms (CPU)",
        t_decode.as_secs_f64() * 1000.0,
        t_resize.as_secs_f64() * 1000.0,
        t_analyze.as_secs_f64() * 1000.0
    );
    let t3 = Instant::now();
    match bestofburst::faces::find_faces(&decode::decode_full(file, format)?.image, decode::read_orientation(file, format)) {
        Err(e) => println!("Faces:     check unavailable ({e:#})"),
        Ok(faces) => {
            println!("Faces:     {}  ({:.0} ms incl. opening the file again)", faces.len(), t3.elapsed().as_secs_f64() * 1000.0);
            for f in &faces {
                println!(
                    "  at {:.2},{:.2}  size {:.0}% of the long edge  eye openness left {:.2} right {:.2}  -> {}",
                    f.x,
                    f.y,
                    f.size * 100.0,
                    f.left,
                    f.right,
                    bestofburst::rank::eye_state(f).name()
                );
            }
        }
    }
    match bestofburst::gpu::get() {
        Err(why) => println!("GPU:       not used ({why})"),
        Ok(g) => {
            let full = decode::decode_full(file, format)?;
            let _ = g.analyze(&full.image, decode::ANALYSIS_EDGE, 320); // warm-up
            let t = Instant::now();
            match g.analyze(&full.image, decode::ANALYSIS_EDGE, 320) {
                Ok(o) => {
                    let (gm, _) = analyze::finish(&o.sums);
                    let gh = analyze::dhash_from_cells(&o.cells);
                    println!("GPU:       {} ({}, {})", g.info.name, g.info.backend, g.info.device_type);
                    println!(
                        "  resize + analyze + hash + thumbnail {:.0} ms  (CPU: {:.0} ms)",
                        t.elapsed().as_secs_f64() * 1000.0,
                        (t_resize + t_analyze).as_secs_f64() * 1000.0
                    );
                    println!(
                        "  sharpness {:.1} (CPU {:.1}) · luma {:.3} · contrast {:.3} · noise {:.2} · dHash distance {}",
                        gm.sharpness,
                        m.sharpness,
                        gm.mean_luma,
                        gm.contrast,
                        gm.noise,
                        analyze::hamming(gh, hash)
                    );
                }
                Err(e) => println!("GPU:       failed on this photo ({e:#}); the CPU would be used"),
            }
        }
    }
    Ok(())
}
