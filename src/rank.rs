//! Burst grouping, hard rejects, scores, best frame per group, and selection of the best N.
//! Reads only what `state.json` stores, so re-ranking takes seconds.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::analyze::{hamming, Metrics};
use crate::state::{GroupInfo, LastRank, RankResult, State};

// Grouping constants (written into `last_rank`).
pub const BURST_GAP_MS: i64 = 2000;
pub const BURST_HASH: u32 = 10;
pub const DUPLICATE_HASH: u32 = 4;
pub const LOOKBACK_S: i64 = 120;
pub const LOOKBACK_HASH: u32 = 6;
pub const LOOKBACK_GROUPS: usize = 8;
pub const MAX_GROUP_SIZE: usize = 60;

// Hard-reject thresholds: starting points, adjust after looking at a real run.
pub const BLURRY_ABS: f32 = 10.0;
pub const BLURRY_REL: f32 = 0.25;
pub const OVEREXPOSED_CLIP: f32 = 0.25;
pub const UNDEREXPOSED_CLIP: f32 = 0.50;
pub const TOO_DARK_LUMA: f32 = 0.05;
pub const TOO_BRIGHT_LUMA: f32 = 0.95;
pub const LOW_CONTRAST: f32 = 0.03;

/// Top two group scores closer than this flag a close call.
pub const CLOSE_CALL_POINTS: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Status {
    Pick,
    Maybe,
    Reject,
    Failed,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Pick => "pick",
            Status::Maybe => "maybe",
            Status::Reject => "reject",
            Status::Failed => "failed",
        }
    }
    pub fn from_name(s: &str) -> Option<Status> {
        Some(match s {
            "pick" => Status::Pick,
            "maybe" => Status::Maybe,
            "reject" => Status::Reject,
            "failed" => Status::Failed,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preset {
    #[default]
    General,
    Portrait,
    Sports,
    Landscape,
}

pub const PRESETS: [Preset; 4] = [Preset::General, Preset::Portrait, Preset::Sports, Preset::Landscape];

impl Preset {
    /// Weights for sharpness, exposure, contrast, noise, color.
    pub fn weights(self) -> [f32; 5] {
        match self {
            Preset::General => [0.40, 0.25, 0.15, 0.10, 0.10],
            Preset::Portrait => [0.35, 0.35, 0.10, 0.15, 0.05],
            Preset::Sports => [0.55, 0.20, 0.10, 0.10, 0.05],
            Preset::Landscape => [0.30, 0.25, 0.25, 0.05, 0.15],
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Preset::General => "general",
            Preset::Portrait => "portrait",
            Preset::Sports => "sports",
            Preset::Landscape => "landscape",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Preset::General => "General",
            Preset::Portrait => "Portrait",
            Preset::Sports => "Sports",
            Preset::Landscape => "Landscape",
        }
    }
    pub fn parse(s: &str) -> Option<Preset> {
        PRESETS.iter().copied().find(|p| p.name().eq_ignore_ascii_case(s))
    }
}

/// How many picks to make.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keep {
    Count(usize),
    Percent(f64),
}

impl Default for Keep {
    fn default() -> Self {
        Keep::Percent(10.0)
    }
}

impl Keep {
    /// `--keep`, or ceil(--keep-percent % of analyzed files).
    pub fn target(self, analyzed: usize) -> usize {
        match self {
            Keep::Count(n) => n,
            Keep::Percent(p) => ((p.max(0.0) / 100.0) * analyzed as f64 - 1e-9).ceil().max(0.0) as usize,
        }
    }
    /// "1000" or "10%".
    pub fn parse(s: &str) -> Option<Keep> {
        let s = s.trim();
        if let Some(p) = s.strip_suffix('%') {
            let v: f64 = p.trim().parse().ok()?;
            (v >= 0.0).then_some(Keep::Percent(v))
        } else {
            s.parse::<usize>().ok().map(Keep::Count)
        }
    }
    pub fn display(self) -> String {
        match self {
            Keep::Count(n) => n.to_string(),
            Keep::Percent(p) => format!("{}%", trim_float(p)),
        }
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    /// Analysis succeeded.
    pub ok: bool,
    pub metrics: Metrics,
    pub tiles: [f32; 64],
    pub dhash: u64,
    pub capture_ms: i64,
    pub group: u64,
    pub group_size: usize,
    pub reasons: Vec<String>,
    /// sharpness, exposure, contrast, noise, color, each 0..1.
    pub components: [f32; 5],
    pub score: f32,
    pub group_score: f32,
    pub close_call: bool,
    /// Status from ranking alone.
    pub status: Status,
    /// Status set by the user, if any.
    pub override_status: Option<Status>,
    /// Rank among the effective picks (1..N).
    pub rank: Option<usize>,
}

impl Entry {
    pub fn new(path: &str, metrics: Metrics, tiles: [f32; 64], dhash: u64, capture_ms: i64) -> Self {
        Entry {
            path: path.to_string(),
            ok: true,
            metrics,
            tiles,
            dhash,
            capture_ms,
            group: 0,
            group_size: 1,
            reasons: Vec::new(),
            components: [0.0; 5],
            score: 0.0,
            group_score: 0.0,
            close_call: false,
            status: Status::Maybe,
            override_status: None,
            rank: None,
        }
    }

    pub fn failed(path: &str) -> Self {
        let mut e = Entry::new(path, Metrics::default(), [0.0; 64], 0, i64::MAX);
        e.ok = false;
        e.status = Status::Failed;
        e
    }

    /// The override if present, else the ranked status.
    pub fn effective(&self) -> Status {
        self.override_status.unwrap_or(self.status)
    }

    pub fn eligible(&self) -> bool {
        self.ok && self.reasons.is_empty()
    }
}

pub fn entries_from_state(state: &State) -> Vec<Entry> {
    state
        .files
        .iter()
        .map(|(path, f)| match (&f.metrics, f.is_done()) {
            (Some(m), true) => {
                let mut tiles = [0f32; 64];
                if let Some(t) = &f.tiles {
                    for (k, v) in t.iter().take(64).enumerate() {
                        tiles[k] = *v;
                    }
                }
                Entry::new(
                    path,
                    m.clone(),
                    tiles,
                    f.dhash_u64().unwrap_or(0),
                    f.capture_ms.unwrap_or(f.mtime_ms),
                )
            }
            _ => Entry::failed(path),
        })
        .collect()
}

/// Step 1: same-shot groups (sequential pass plus look-back merge). Failed files get their own.
pub fn group_entries(e: &mut [Entry]) {
    struct G {
        first: usize,
        start_ms: i64,
        size: usize,
    }
    let mut order: Vec<usize> = (0..e.len()).filter(|&i| e[i].ok).collect();
    order.sort_by(|&a, &b| e[a].capture_ms.cmp(&e[b].capture_ms).then_with(|| e[a].path.cmp(&e[b].path)));

    let mut groups: Vec<G> = Vec::new();
    let mut prev: Option<usize> = None;
    for &i in &order {
        let mut assigned: Option<usize> = None;
        if let Some(p) = prev {
            let gi = e[p].group as usize;
            let d = hamming(e[i].dhash, e[p].dhash);
            let gap = e[i].capture_ms.saturating_sub(e[p].capture_ms);
            if ((gap <= BURST_GAP_MS && d <= BURST_HASH) || d <= DUPLICATE_HASH) && groups[gi].size < MAX_GROUP_SIZE {
                assigned = Some(gi);
            }
        }
        if assigned.is_none() {
            // Look-back: "shot A, two other frames, shot A again".
            let mut best: Option<(u32, usize)> = None;
            for gi in (0..groups.len()).rev().take(LOOKBACK_GROUPS) {
                let g = &groups[gi];
                if e[i].capture_ms.saturating_sub(g.start_ms) > LOOKBACK_S * 1000 || g.size >= MAX_GROUP_SIZE {
                    continue;
                }
                let d = hamming(e[i].dhash, e[g.first].dhash);
                if d <= LOOKBACK_HASH && best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, gi));
                }
            }
            assigned = best.map(|(_, g)| g);
        }
        let gi = match assigned {
            Some(g) => {
                groups[g].size += 1;
                g
            }
            None => {
                groups.push(G { first: i, start_ms: e[i].capture_ms, size: 1 });
                groups.len() - 1
            }
        };
        e[i].group = gi as u64;
        prev = Some(i);
    }
    let mut next = groups.len() as u64;
    for x in e.iter_mut().filter(|x| !x.ok) {
        x.group = next;
        next += 1;
    }
    let mut sizes: BTreeMap<u64, usize> = BTreeMap::new();
    for x in e.iter() {
        *sizes.entry(x.group).or_default() += 1;
    }
    for x in e.iter_mut() {
        x.group_size = sizes.get(&x.group).copied().unwrap_or(1);
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Step 2: hard rejects. Each fired rule adds its reason.
pub fn apply_rejects(e: &mut [Entry]) {
    let med = median(e.iter().filter(|x| x.ok).map(|x| x.metrics.sharpness).collect());
    for x in e.iter_mut() {
        x.reasons.clear();
        if !x.ok {
            x.reasons.push("unreadable".into());
            continue;
        }
        let m = &x.metrics;
        if m.sharpness < BLURRY_ABS || m.sharpness < BLURRY_REL * med {
            x.reasons.push("blurry".into());
        }
        if m.clip_highlights > OVEREXPOSED_CLIP {
            x.reasons.push("overexposed".into());
        }
        if m.clip_shadows > UNDEREXPOSED_CLIP {
            x.reasons.push("underexposed".into());
        }
        if m.mean_luma < TOO_DARK_LUMA {
            x.reasons.push("too_dark".into());
        }
        if m.mean_luma > TOO_BRIGHT_LUMA {
            x.reasons.push("too_bright".into());
        }
        if m.contrast < LOW_CONTRAST {
            x.reasons.push("low_contrast".into());
        }
    }
}

pub fn exposure(m: &Metrics) -> f32 {
    let centre = 1.0 - ((m.mean_luma - 0.45).abs() / 0.35).min(1.0);
    centre * (1.0 - (4.0 * m.clip_highlights).min(1.0)) * (1.0 - (2.0 * m.clip_shadows).min(1.0))
}

/// Mid-rank percentiles among the eligible values (ties share their average rank; a single
/// file gets 1.0). Rejected files are placed into the eligible distribution without counting.
// SPEC-GAP: the spec leaves rejected files' percentiles open; they are interpolated against
// the eligible set so their report scores stay comparable.
fn percentiles(values: &[f32], eligible: &[bool]) -> Vec<f32> {
    let mut sorted: Vec<f32> = values
        .iter()
        .zip(eligible)
        .filter(|(_, &ok)| ok)
        .map(|(v, _)| *v)
        .collect();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    values
        .iter()
        .zip(eligible)
        .map(|(&v, &is_el)| {
            if n == 0 {
                return 0.5;
            }
            let less = sorted.partition_point(|&s| s < v);
            let le = sorted.partition_point(|&s| s <= v);
            if n == 1 {
                return if is_el || v >= sorted[0] { 1.0 } else { 0.0 };
            }
            let pos = if is_el {
                less as f32 + (le - less).saturating_sub(1) as f32 / 2.0
            } else {
                (less as f32 + (le - less) as f32 / 2.0 - 0.5).max(0.0)
            };
            (pos / (n - 1) as f32).clamp(0.0, 1.0)
        })
        .collect()
}

pub fn weighted(c: &[f32; 5], w: &[f32; 5]) -> f32 {
    let sw: f32 = w.iter().sum();
    100.0 * c.iter().zip(w).map(|(c, w)| c * w).sum::<f32>() / sw
}

/// Step 3: components and the 0..100 score.
pub fn score_entries(e: &mut [Entry], weights: &[f32; 5]) {
    let el: Vec<bool> = e.iter().map(|x| x.eligible()).collect();
    let p_sharp = percentiles(&e.iter().map(|x| x.metrics.sharpness).collect::<Vec<_>>(), &el);
    let p_con = percentiles(&e.iter().map(|x| x.metrics.contrast).collect::<Vec<_>>(), &el);
    let p_noise = percentiles(&e.iter().map(|x| x.metrics.noise).collect::<Vec<_>>(), &el);
    let p_col = percentiles(&e.iter().map(|x| x.metrics.colorfulness).collect::<Vec<_>>(), &el);
    for (i, x) in e.iter_mut().enumerate() {
        if !x.ok {
            x.components = [0.0; 5];
            x.score = 0.0;
            x.group_score = 0.0;
            continue;
        }
        x.components = [p_sharp[i], exposure(&x.metrics), p_con[i], 1.0 - p_noise[i], p_col[i]];
        x.score = weighted(&x.components, weights);
        x.group_score = x.score;
    }
}

/// Relative sharpness of each frame on the group's subject tiles, 0..1.
pub fn subject_sharpness(group: &[&Entry]) -> Vec<f32> {
    let max_t: Vec<f32> = (0..64).map(|t| group.iter().map(|f| f.tiles[t]).fold(0.0, f32::max)).collect();
    let peak = max_t.iter().cloned().fold(0.0, f32::max);
    let subject: Vec<usize> = (0..64).filter(|&t| peak > 0.0 && max_t[t] >= 0.5 * peak).collect();
    group
        .iter()
        .map(|f| subject.iter().map(|&t| f.tiles[t] / max_t[t]).sum::<f32>() / subject.len().max(1) as f32)
        .collect()
}

/// Centre of the subject tiles as 0..1 fractions.
pub fn subject_center(group: &[&Entry]) -> [f32; 2] {
    let max_t: Vec<f32> = (0..64).map(|t| group.iter().map(|f| f.tiles[t]).fold(0.0, f32::max)).collect();
    let peak = max_t.iter().cloned().fold(0.0, f32::max);
    if peak <= 0.0 {
        return [0.5, 0.5];
    }
    let (mut sx, mut sy, mut n) = (0.0f32, 0.0f32, 0.0f32);
    for t in 0..64 {
        if max_t[t] >= 0.5 * peak {
            sx += (t % 8) as f32 + 0.5;
            sy += (t / 8) as f32 + 0.5;
            n += 1.0;
        }
    }
    [sx / n / 8.0, sy / n / 8.0]
}

/// Inside a group: group score desc, then score, then path.
pub fn better_in_group(a: &Entry, b: &Entry) -> Ordering {
    b.group_score
        .total_cmp(&a.group_score)
        .then_with(|| b.score.total_cmp(&a.score))
        .then_with(|| a.path.cmp(&b.path))
}

/// Across groups: score desc, then sharpness desc, then path asc.
pub fn better(a: &Entry, b: &Entry) -> Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| b.metrics.sharpness.total_cmp(&a.metrics.sharpness))
        .then_with(|| a.path.cmp(&b.path))
}

/// Step 3b: group scores, close calls and subject centres.
pub fn group_scores(e: &mut [Entry], weights: &[f32; 5]) -> BTreeMap<u64, [f32; 2]> {
    let mut by_group: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, x) in e.iter().enumerate() {
        if x.ok {
            by_group.entry(x.group).or_default().push(i);
        }
    }
    let mut centers = BTreeMap::new();
    for (g, members) in by_group {
        let eligible: Vec<usize> = members.iter().copied().filter(|&i| e[i].eligible()).collect();
        let basis: &Vec<usize> = if eligible.is_empty() { &members } else { &eligible };
        let refs: Vec<&Entry> = basis.iter().map(|&i| &e[i]).collect();
        centers.insert(g, subject_center(&refs));

        for &i in &members {
            e[i].close_call = false;
            e[i].group_score = e[i].score;
        }
        if eligible.len() >= 2 {
            let refs: Vec<&Entry> = eligible.iter().map(|&i| &e[i]).collect();
            let ss = subject_sharpness(&refs);
            for (k, &i) in eligible.iter().enumerate() {
                let mut c = e[i].components;
                c[0] = ss[k];
                e[i].group_score = weighted(&c, weights);
            }
            let mut gs: Vec<f32> = eligible.iter().map(|&i| e[i].group_score).collect();
            gs.sort_by(|a, b| b.total_cmp(a));
            let close = gs[0] - gs[1] < CLOSE_CALL_POINTS;
            for &i in &members {
                e[i].close_call = close;
            }
        }
    }
    centers
}

/// Step 4: the best N, at most `max_per_group` from one same-shot group.
pub fn select(e: &mut [Entry], target: usize, max_per_group: usize) {
    let mut groups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, f) in e.iter().enumerate() {
        if f.eligible() {
            groups.entry(f.group).or_default().push(i);
        }
    }
    for g in groups.values_mut() {
        g.sort_by(|&a, &b| better_in_group(&e[a], &e[b]));
    }
    let mut picks: Vec<usize> = Vec::new();
    for round in 0..max_per_group.max(1) {
        if picks.len() >= target {
            break;
        }
        let mut tier: Vec<usize> = groups.values().filter_map(|g| g.get(round).copied()).collect();
        tier.sort_by(|&a, &b| better(&e[a], &e[b]));
        for i in tier {
            if picks.len() >= target {
                break;
            }
            picks.push(i);
        }
    }
    for x in e.iter_mut() {
        x.status = if !x.ok {
            Status::Failed
        } else if !x.reasons.is_empty() {
            Status::Reject
        } else {
            Status::Maybe
        };
        x.rank = None;
    }
    for &i in &picks {
        e[i].status = Status::Pick;
    }
}

/// Apply user overrides and number the effective picks by score.
pub fn apply_overrides(e: &mut [Entry], overrides: &BTreeMap<String, String>) {
    for x in e.iter_mut() {
        x.override_status = overrides.get(&x.path).and_then(|s| Status::from_name(s)).filter(|s| *s != Status::Failed);
    }
    renumber(e);
}

pub fn renumber(e: &mut [Entry]) {
    let mut picks: Vec<usize> = (0..e.len()).filter(|&i| e[i].effective() == Status::Pick).collect();
    picks.sort_by(|&a, &b| better(&e[a], &e[b]));
    for x in e.iter_mut() {
        x.rank = None;
    }
    for (r, &i) in picks.iter().enumerate() {
        e[i].rank = Some(r + 1);
    }
}

#[derive(Clone, Debug, Default)]
pub struct RankSummary {
    pub analyzed: usize,
    pub failed: usize,
    pub groups: usize,
    pub target: usize,
    pub picks: usize,
    pub maybe: usize,
    pub rejects: usize,
    /// First reason -> count, effective rejects only.
    pub reject_reasons: BTreeMap<String, usize>,
    pub close_call_groups: usize,
}

pub struct Ranking {
    pub entries: Vec<Entry>,
    pub centers: BTreeMap<u64, [f32; 2]>,
    pub summary: RankSummary,
    pub preset: Preset,
    pub target: usize,
    pub max_per_group: usize,
}

pub fn summarize(e: &[Entry], target: usize) -> RankSummary {
    let mut s = RankSummary { target, ..Default::default() };
    let mut groups = std::collections::BTreeSet::new();
    let mut close = std::collections::BTreeSet::new();
    for x in e {
        if x.ok {
            s.analyzed += 1;
            groups.insert(x.group);
            if x.close_call {
                close.insert(x.group);
            }
        } else {
            s.failed += 1;
        }
        match x.effective() {
            Status::Pick => s.picks += 1,
            Status::Maybe => s.maybe += 1,
            Status::Reject => {
                s.rejects += 1;
                let r = x.reasons.first().cloned().unwrap_or_else(|| "manual".into());
                *s.reject_reasons.entry(r).or_default() += 1;
            }
            Status::Failed => {}
        }
    }
    s.groups = groups.len();
    s.close_call_groups = close.len();
    s
}

/// Run every ranking step over the analyzed files in `state`, with overrides applied.
pub fn rank(state: &State, preset: Preset, keep: Keep, max_per_group: usize) -> Ranking {
    let mut e = entries_from_state(state);
    let analyzed = e.iter().filter(|x| x.ok).count();
    let target = keep.target(analyzed);
    let w = preset.weights();
    group_entries(&mut e);
    apply_rejects(&mut e);
    score_entries(&mut e, &w);
    let centers = group_scores(&mut e, &w);
    select(&mut e, target, max_per_group);
    apply_overrides(&mut e, &state.overrides);
    let summary = summarize(&e, target);
    Ranking { entries: e, centers, summary, preset, target, max_per_group }
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// Write the ranking into `state.last_rank` (effective statuses and ranks).
pub fn store(state: &mut State, r: &Ranking) {
    let mut results = BTreeMap::new();
    for x in &r.entries {
        results.insert(
            x.path.clone(),
            RankResult {
                status: x.effective().name().to_string(),
                rank: x.rank,
                score: round2(x.score),
                group: x.group,
                reasons: x.reasons.clone(),
                group_score: round2(x.group_score),
                close_call: x.close_call,
            },
        );
    }
    let groups = r
        .centers
        .iter()
        .map(|(g, c)| (*g, GroupInfo { subject_center: [round2(c[0]), round2(c[1])] }))
        .collect();
    state.last_rank = Some(LastRank {
        ranked_at: crate::state::now_string(),
        preset: r.preset.name().to_string(),
        target: r.target,
        max_per_group: r.max_per_group,
        burst_gap_ms: BURST_GAP_MS,
        burst_hash: BURST_HASH,
        duplicate_hash: DUPLICATE_HASH,
        lookback_s: LOOKBACK_S,
        lookback_hash: LOOKBACK_HASH,
        results,
        groups,
    });
}
