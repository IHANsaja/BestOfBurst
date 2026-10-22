//! Off-UI-thread image loading: a 2-thread pool for thumbnails (LRU of 800 textures) and one
//! worker for 1,600 px previews (last 20), 100% subject crops (last 8) and full-size images for
//! the full-screen viewer (last 2). Everything is turned upright by its EXIF orientation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions, Vec2};

use crate::decode::{self, RgbImage};
use crate::scan::Format;

const THUMB_CAP: usize = 800;
const PREVIEW_CAP: usize = 20;
const CROP_CAP: usize = 8;
const FULL_CAP: usize = 2;
const PREVIEW_EDGE: u32 = 1600;
const CROP_W: u32 = 1200;
const CROP_H: u32 = 800;
const QUEUE_CAP: usize = 600;
const RETRY_AFTER: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Thumb,
    Preview,
    Crop,
    Full,
}

#[derive(Clone)]
enum Job {
    /// `src` is the original photo, read for its orientation (cached thumbnails are not rotated).
    Thumb { key: String, path: PathBuf, src: Option<(PathBuf, Format)> },
    Preview { key: String, path: PathBuf, format: Format },
    Crop { key: String, path: PathBuf, format: Format, center: [f32; 2] },
    Full { key: String, path: PathBuf, format: Format, max_side: u32 },
}

struct Done {
    kind: Kind,
    key: String,
    result: Result<ColorImage, String>,
}

/// LIFO job stack: the most recently requested images (what is on screen now) load first.
struct Queue {
    jobs: Mutex<Vec<Job>>,
    cv: Condvar,
}

impl Queue {
    fn new() -> Arc<Self> {
        Arc::new(Queue { jobs: Mutex::new(Vec::new()), cv: Condvar::new() })
    }
    fn push(&self, job: Job) {
        if let Ok(mut j) = self.jobs.lock() {
            j.push(job);
            if j.len() > QUEUE_CAP {
                j.remove(0);
            }
        }
        self.cv.notify_one();
    }
    fn pop(&self) -> Option<Job> {
        let mut j = self.jobs.lock().ok()?;
        loop {
            if let Some(job) = j.pop() {
                return Some(job);
            }
            j = self.cv.wait(j).ok()?;
        }
    }
}

fn to_color_image(img: &RgbImage) -> ColorImage {
    ColorImage::from_rgb([img.width as usize, img.height as usize], &img.rgb)
}

fn run_job(job: &Job) -> Result<ColorImage, String> {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<ColorImage> {
        match job {
            Job::Thumb { path, src, .. } => {
                let data = std::fs::read(path)?;
                let img = decode::decode_jpeg_bytes(&data)?;
                let o = src.as_ref().map(|(p, f)| decode::orientation_cached(p, *f)).unwrap_or(1);
                Ok(to_color_image(&decode::apply_orientation(img, o)))
            }
            Job::Preview { path, format, .. } => {
                let d = decode::decode_full(path, *format)?;
                let small = decode::resize_long_edge(d.image, PREVIEW_EDGE)?;
                let oriented = decode::apply_orientation(small, decode::orientation_cached(path, *format));
                Ok(to_color_image(&oriented))
            }
            Job::Crop { path, format, center, .. } => {
                let d = decode::decode_full(path, *format)?;
                let crop = decode::crop_centered(&d.image, center[0], center[1], CROP_W, CROP_H);
                let oriented = decode::apply_orientation(crop, decode::orientation_cached(path, *format));
                Ok(to_color_image(&oriented))
            }
            Job::Full { path, format, max_side, .. } => {
                let d = decode::decode_full(path, *format)?;
                let img = decode::resize_long_edge(d.image, *max_side)?;
                let oriented = decode::apply_orientation(img, decode::orientation_cached(path, *format));
                Ok(to_color_image(&oriented))
            }
        }
    }));
    match r {
        Ok(Ok(img)) => Ok(img),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("decoder crashed".into()),
    }
}

fn spawn_worker(queue: Arc<Queue>, tx: Sender<Done>, ctx: egui::Context, name: String) {
    let _ = std::thread::Builder::new().name(name).spawn(move || {
        while let Some(job) = queue.pop() {
            let (kind, key) = match &job {
                Job::Thumb { key, .. } => (Kind::Thumb, key.clone()),
                Job::Preview { key, .. } => (Kind::Preview, key.clone()),
                Job::Crop { key, .. } => (Kind::Crop, key.clone()),
                Job::Full { key, .. } => (Kind::Full, key.clone()),
            };
            let result = run_job(&job);
            if tx.send(Done { kind, key, result }).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
}

struct Cache {
    cap: usize,
    map: HashMap<String, (TextureHandle, u64)>,
    pending: HashMap<String, Instant>,
    failed: HashMap<String, String>,
}

impl Cache {
    fn new(cap: usize) -> Self {
        Cache { cap, map: HashMap::new(), pending: HashMap::new(), failed: HashMap::new() }
    }
    fn insert(&mut self, key: String, tex: TextureHandle, tick: u64) {
        self.pending.remove(&key);
        self.map.insert(key, (tex, tick));
        while self.map.len() > self.cap {
            let Some(oldest) = self.map.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()) else {
                break;
            };
            self.map.remove(&oldest);
        }
    }
    fn clear(&mut self) {
        self.map.clear();
        self.pending.clear();
        self.failed.clear();
    }
}

pub enum Loaded {
    Ready(TextureId, Vec2),
    Loading,
    Failed(String),
}

pub struct Textures {
    tick: u64,
    thumbs: Cache,
    previews: Cache,
    crops: Cache,
    fulls: Cache,
    thumb_q: Arc<Queue>,
    big_q: Arc<Queue>,
    rx: Receiver<Done>,
    max_side: u32,
}

impl Textures {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let thumb_q = Queue::new();
        let big_q = Queue::new();
        for i in 0..2 {
            spawn_worker(thumb_q.clone(), tx.clone(), ctx.clone(), format!("bob-thumb-{i}"));
        }
        spawn_worker(big_q.clone(), tx, ctx.clone(), "bob-preview".into());
        Textures {
            tick: 0,
            thumbs: Cache::new(THUMB_CAP),
            previews: Cache::new(PREVIEW_CAP),
            crops: Cache::new(CROP_CAP),
            fulls: Cache::new(FULL_CAP),
            thumb_q,
            big_q,
            rx,
            max_side: 8192,
        }
    }

    /// Forget everything (another folder was opened).
    pub fn clear(&mut self) {
        self.thumbs.clear();
        self.previews.clear();
        self.crops.clear();
        self.fulls.clear();
    }

    /// Upload finished decodes.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.tick += 1;
        self.max_side = ctx.input(|i| i.max_texture_side).clamp(2048, 16384) as u32;
        let mut uploaded = 0;
        while uploaded < 48 {
            let Ok(done) = self.rx.try_recv() else { break };
            uploaded += 1;
            let cache = match done.kind {
                Kind::Thumb => &mut self.thumbs,
                Kind::Preview => &mut self.previews,
                Kind::Crop => &mut self.crops,
                Kind::Full => &mut self.fulls,
            };
            match done.result {
                Ok(img) => {
                    let opts = TextureOptions::LINEAR;
                    let tex = ctx.load_texture(&done.key, img, opts);
                    cache.insert(done.key, tex, self.tick);
                }
                Err(e) => {
                    cache.pending.remove(&done.key);
                    cache.failed.insert(done.key, e);
                }
            }
        }
        if uploaded == 48 {
            ctx.request_repaint();
        }
    }

    fn lookup(cache: &mut Cache, key: &str, tick: u64) -> Option<Loaded> {
        if let Some((tex, t)) = cache.map.get_mut(key) {
            *t = tick;
            return Some(Loaded::Ready(tex.id(), tex.size_vec2()));
        }
        if let Some(e) = cache.failed.get(key) {
            return Some(Loaded::Failed(e.clone()));
        }
        if let Some(at) = cache.pending.get(key) {
            if at.elapsed() < RETRY_AFTER {
                return Some(Loaded::Loading);
            }
        }
        None
    }

    /// A cached analysis thumbnail; `src` is its original photo, used to turn it upright.
    pub fn thumb(&mut self, path: &Path, src: Option<(&Path, Format)>) -> Loaded {
        // Upright and as-stored versions are cached apart: the contact sheet shows the stored one.
        let key = format!("{}{}", path.to_string_lossy(), if src.is_some() { "|up" } else { "" });
        if let Some(l) = Self::lookup(&mut self.thumbs, &key, self.tick) {
            return l;
        }
        self.thumbs.pending.insert(key.clone(), Instant::now());
        let src = src.map(|(p, f)| (p.to_path_buf(), f));
        self.thumb_q.push(Job::Thumb { key, path: path.to_path_buf(), src });
        Loaded::Loading
    }

    /// The preview if it is already loaded, without asking for it.
    pub fn preview_if_ready(&mut self, path: &Path) -> Option<(TextureId, Vec2)> {
        let key = path.to_string_lossy().to_string();
        match Self::lookup(&mut self.previews, &key, self.tick) {
            Some(Loaded::Ready(id, size)) => Some((id, size)),
            _ => None,
        }
    }

    /// The photo at full size (up to the GPU's largest texture), for the full-screen viewer.
    pub fn full(&mut self, path: &Path, format: Format) -> Loaded {
        let key = format!("{}|full", path.to_string_lossy());
        if let Some(l) = Self::lookup(&mut self.fulls, &key, self.tick) {
            return l;
        }
        self.fulls.pending.insert(key.clone(), Instant::now() + Duration::from_secs(60));
        self.big_q.push(Job::Full { key, path: path.to_path_buf(), format, max_side: self.max_side });
        Loaded::Loading
    }

    pub fn preview(&mut self, path: &Path, format: Format) -> Loaded {
        let key = path.to_string_lossy().to_string();
        if let Some(l) = Self::lookup(&mut self.previews, &key, self.tick) {
            return l;
        }
        self.previews.pending.insert(key.clone(), Instant::now() + Duration::from_secs(60));
        self.big_q.push(Job::Preview { key, path: path.to_path_buf(), format });
        Loaded::Loading
    }

    pub fn crop(&mut self, path: &Path, format: Format, center: [f32; 2]) -> Loaded {
        let key = format!("{}|crop|{:.3},{:.3}", path.to_string_lossy(), center[0], center[1]);
        if let Some(l) = Self::lookup(&mut self.crops, &key, self.tick) {
            return l;
        }
        self.crops.pending.insert(key.clone(), Instant::now() + Duration::from_secs(60));
        self.big_q.push(Job::Crop { key, path: path.to_path_buf(), format, center });
        Loaded::Loading
    }
}
