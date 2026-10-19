//! `picks`, `maybe` and `rejects` folders (hard links or copies) and `report.csv`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::analyze::format_capture;
use crate::rank::{better, Entry, Status};
use crate::state::State;

/// The only folders BestOfBurst ever empties.
pub const OWNED_DIRS: [&str; 3] = ["picks", "maybe", "rejects"];

#[derive(Clone, Copy, Debug, Default)]
pub struct OutputOptions {
    pub maybe: bool,
    pub rejects: bool,
    pub copy: bool,
    pub keep_names: bool,
}

#[derive(Clone, Debug, Default)]
pub struct OutputSummary {
    pub picks_dir: PathBuf,
    pub written: usize,
    pub copied_fallback: bool,
    pub errors: Vec<String>,
}

/// Refuse `--out` equal to the input folder.
pub fn check_out(input: &Path, out: &Path) -> Result<()> {
    let same = match (crate::scan::dunce_canonical(input), crate::scan::dunce_canonical(out)) {
        (Some(a), Some(b)) => a == b,
        _ => input == out,
    };
    if same {
        bail!("--out must not be the input folder itself");
    }
    Ok(())
}

fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `name`, then `name_1`, `name_2`, ... until unused in this folder (case-insensitive).
fn unique_name(used: &mut HashSet<String>, name: &str) -> String {
    let (stem, ext) = split_name(name);
    let mut candidate = name.to_string();
    let mut k = 1;
    while used.contains(&candidate.to_lowercase()) {
        candidate = format!("{stem}_{k}{ext}");
        k += 1;
    }
    used.insert(candidate.to_lowercase());
    candidate
}

fn file_name(rel: &str) -> String {
    rel.rsplit(['\\', '/']).next().unwrap_or(rel).to_string()
}

struct Linker {
    copy: bool,
    fell_back: bool,
}

impl Linker {
    fn place(&mut self, src: &Path, dst: &Path) -> std::io::Result<()> {
        if !self.copy && !self.fell_back {
            match std::fs::hard_link(src, dst) {
                Ok(()) => return Ok(()),
                Err(_) => self.fell_back = true,
            }
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// Empty the owned folders and rebuild them from the effective statuses.
pub fn write_folders(input: &Path, out: &Path, entries: &[Entry], opts: OutputOptions) -> Result<OutputSummary> {
    check_out(input, out)?;
    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;
    for d in OWNED_DIRS {
        let p = out.join(d);
        if p.exists() {
            // These folders hold only links or copies BestOfBurst made; removing a hard link
            // never touches the original.
            std::fs::remove_dir_all(&p).with_context(|| format!("cannot empty {}", p.display()))?;
        }
    }
    let picks_dir = out.join("picks");
    std::fs::create_dir_all(&picks_dir)?;

    let mut linker = Linker { copy: opts.copy, fell_back: false };
    let mut summary = OutputSummary { picks_dir: picks_dir.clone(), ..Default::default() };

    let mut picks: Vec<&Entry> = entries.iter().filter(|e| e.effective() == Status::Pick).collect();
    picks.sort_by_key(|e| e.rank.unwrap_or(usize::MAX));
    let mut used = HashSet::new();
    for e in picks {
        let name = file_name(&e.path);
        let dest_name = if opts.keep_names {
            unique_name(&mut used, &name)
        } else {
            unique_name(&mut used, &format!("{:04}_{}", e.rank.unwrap_or(0), name))
        };
        match linker.place(&input.join(&e.path), &picks_dir.join(&dest_name)) {
            Ok(()) => summary.written += 1,
            Err(err) => summary.errors.push(format!("{}: {err}", e.path)),
        }
    }

    if opts.maybe {
        let dir = out.join("maybe");
        std::fs::create_dir_all(&dir)?;
        let mut used = HashSet::new();
        let mut maybe: Vec<&Entry> = entries.iter().filter(|e| e.effective() == Status::Maybe).collect();
        maybe.sort_by(|a, b| better(a, b));
        for e in maybe {
            let dest = dir.join(unique_name(&mut used, &file_name(&e.path)));
            match linker.place(&input.join(&e.path), &dest) {
                Ok(()) => summary.written += 1,
                Err(err) => summary.errors.push(format!("{}: {err}", e.path)),
            }
        }
    }

    if opts.rejects {
        let dir = out.join("rejects");
        let mut used: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        let mut rejects: Vec<&Entry> = entries
            .iter()
            .filter(|e| matches!(e.effective(), Status::Reject | Status::Failed))
            .collect();
        rejects.sort_by(|a, b| better(a, b));
        for e in rejects {
            let reason = e.reasons.first().cloned().unwrap_or_else(|| "manual".into());
            let sub = dir.join(&reason);
            std::fs::create_dir_all(&sub)?;
            let set = used.entry(reason).or_default();
            let dest = sub.join(unique_name(set, &file_name(&e.path)));
            match linker.place(&input.join(&e.path), &dest) {
                Ok(()) => summary.written += 1,
                Err(err) => summary.errors.push(format!("{}: {err}", e.path)),
            }
        }
    }

    summary.copied_fallback = linker.fell_back;
    Ok(summary)
}

fn r3(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

/// One row per file: picks by rank, then maybe by score, then rejects by score, then failed.
pub fn write_report(out: &Path, state: &State, entries: &[Entry]) -> Result<()> {
    let path = out.join("report.csv");
    let tmp = out.join("report.csv.tmp");
    {
        let mut w = csv::Writer::from_path(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        w.write_record([
            "rank", "status", "score", "file", "reasons", "group", "group_size", "sharpness", "tenengrad",
            "mean_luma", "clip_highlights", "clip_shadows", "contrast", "noise", "colorfulness", "capture_time",
            "format", "decoder", "error",
        ])?;
        let bucket = |e: &Entry| match e.effective() {
            Status::Pick => 0,
            Status::Maybe => 1,
            Status::Reject => 2,
            Status::Failed => 3,
        };
        let mut order: Vec<&Entry> = entries.iter().collect();
        order.sort_by(|a, b| {
            bucket(a).cmp(&bucket(b)).then_with(|| match bucket(a) {
                0 => a.rank.cmp(&b.rank),
                3 => a.path.cmp(&b.path),
                _ => better(a, b),
            })
        });
        for e in order {
            let f = state.files.get(&e.path);
            let m = &e.metrics;
            let ok = e.ok;
            let num = |v: f32| if ok { r3(v) } else { String::new() };
            w.write_record([
                e.rank.map(|r| r.to_string()).unwrap_or_default(),
                e.effective().name().to_string(),
                if ok { format!("{:.1}", e.score) } else { String::new() },
                e.path.clone(),
                e.reasons.join(";"),
                e.group.to_string(),
                e.group_size.to_string(),
                if ok { format!("{:.1}", m.sharpness) } else { String::new() },
                if ok { format!("{:.1}", m.tenengrad) } else { String::new() },
                num(m.mean_luma),
                num(m.clip_highlights),
                num(m.clip_shadows),
                num(m.contrast),
                if ok { format!("{:.2}", m.noise) } else { String::new() },
                if ok { format!("{:.1}", m.colorfulness) } else { String::new() },
                f.and_then(|f| f.capture_ms).map(format_capture).unwrap_or_default(),
                f.map(|f| f.format.clone()).unwrap_or_default(),
                f.and_then(|f| f.decoder.clone()).unwrap_or_default(),
                f.and_then(|f| f.error.clone()).unwrap_or_default(),
            ])?;
        }
        w.flush()?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}
