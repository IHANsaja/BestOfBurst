//! `state.json`: one entry per file plus the last ranking and the user's overrides.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::analyze::Metrics;

pub const STATE_FILE: &str = "state.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub size: u64,
    pub mtime_ms: i64,
    /// `done` or `failed`.
    pub status: String,
    pub format: String,
    #[serde(default)]
    pub decoder: Option<String>,
    #[serde(default)]
    pub capture_ms: Option<i64>,
    #[serde(default)]
    pub dhash: Option<String>,
    #[serde(default)]
    pub metrics: Option<Metrics>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles: Option<Vec<f32>>,
}

impl FileEntry {
    pub fn is_done(&self) -> bool {
        self.status == "done"
    }
    pub fn dhash_u64(&self) -> Option<u64> {
        self.dhash.as_deref().and_then(|h| u64::from_str_radix(h, 16).ok())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RankResult {
    /// `pick`, `maybe`, `reject` or `failed`.
    pub status: String,
    pub rank: Option<usize>,
    pub score: f32,
    pub group: u64,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub group_score: f32,
    #[serde(default)]
    pub close_call: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GroupInfo {
    pub subject_center: [f32; 2],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LastRank {
    pub ranked_at: String,
    pub preset: String,
    pub target: usize,
    pub max_per_group: usize,
    /// The five grouping constants from rank.rs, recorded for reference.
    #[serde(default)]
    pub burst_gap_ms: i64,
    #[serde(default)]
    pub burst_hash: u32,
    #[serde(default)]
    pub duplicate_hash: u32,
    #[serde(default)]
    pub lookback_s: i64,
    #[serde(default)]
    pub lookback_hash: u32,
    pub results: BTreeMap<String, RankResult>,
    #[serde(default)]
    pub groups: BTreeMap<u64, GroupInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub version: u32,
    pub input: String,
    pub created: String,
    pub updated: String,
    pub files: BTreeMap<String, FileEntry>,
    #[serde(default)]
    pub last_rank: Option<LastRank>,
    /// relative path -> "pick" | "maybe" | "reject"
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
    /// relative path -> quarter turns clockwise (1..=3) the user added on top of EXIF.
    /// Display only: delivered files are never rotated.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rotations: BTreeMap<String, u8>,
}

pub fn now_string() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

impl State {
    pub fn new(input: &Path) -> Self {
        let now = now_string();
        State {
            version: crate::ANALYSIS_VERSION,
            input: input.to_string_lossy().to_string(),
            created: now.clone(),
            updated: now,
            files: BTreeMap::new(),
            last_rank: None,
            overrides: BTreeMap::new(),
            rotations: BTreeMap::new(),
        }
    }

    pub fn path(out: &Path) -> PathBuf {
        out.join(STATE_FILE)
    }

    /// Load `<out>\state.json`. Returns a fresh state when it does not exist. A file that
    /// cannot be parsed is renamed to `state.corrupt-<timestamp>.json`, and the warning is
    /// returned alongside a fresh state.
    pub fn load(input: &Path, out: &Path) -> Result<(State, Option<String>)> {
        let path = Self::path(out);
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((State::new(input), None)),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        match serde_json::from_slice::<State>(&data) {
            Ok(mut s) => {
                let mut warning = None;
                if s.version != crate::ANALYSIS_VERSION {
                    warning = Some(format!(
                        "state.json is from analysis version {}; re-analyzing every file.",
                        s.version
                    ));
                    s.files.clear();
                    s.last_rank = None;
                    s.version = crate::ANALYSIS_VERSION;
                }
                s.input = input.to_string_lossy().to_string();
                Ok((s, warning))
            }
            Err(e) => {
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                let bad = out.join(format!("state.corrupt-{stamp}.json"));
                std::fs::rename(&path, &bad).ok();
                Ok((
                    State::new(input),
                    Some(format!(
                        "Warning: state.json could not be read ({e}); renamed to {} and starting fresh.",
                        bad.display()
                    )),
                ))
            }
        }
    }

    /// Load without creating anything; `None` when there is no state file.
    pub fn load_existing(input: &Path, out: &Path) -> Result<Option<(State, Option<String>)>> {
        if !Self::path(out).exists() {
            return Ok(None);
        }
        Self::load(input, out).map(Some)
    }

    /// Atomic save: write `state.json.tmp`, sync, then rename over `state.json`.
    pub fn save(&mut self, out: &Path) -> Result<()> {
        self.updated = now_string();
        self.save_as_is(out)
    }

    pub fn save_as_is(&self, out: &Path) -> Result<()> {
        std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;
        let path = Self::path(out);
        let tmp = out.join("state.json.tmp");
        let data = serde_json::to_vec(self)?;
        {
            let mut f = std::fs::File::create(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
            f.write_all(&data)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &path).with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    /// True when the file is unchanged and already has a `done` or `failed` entry.
    pub fn is_current(&self, rel: &str, size: u64, mtime_ms: i64) -> bool {
        self.files
            .get(rel)
            .map(|e| e.size == size && e.mtime_ms == mtime_ms && (e.status == "done" || e.status == "failed"))
            .unwrap_or(false)
    }

    pub fn counts(&self) -> (usize, usize) {
        let done = self.files.values().filter(|e| e.is_done()).count();
        (done, self.files.len() - done)
    }
}
