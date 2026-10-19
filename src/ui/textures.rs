//! Off-UI-thread image loading: a 2-thread pool for thumbnails (LRU of 800 textures) and one
//! worker for 1,600 px previews (last 20) and 100% subject crops (last 8).

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
}

#[derive(Clone)]
enum Job {
    Thumb { key: String, path: PathBuf },
    Preview { key: String, path: PathBuf, format: Format },
    Crop { key: String, path: PathBuf, format: Format, center: [f32; 2] },
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
            Job::Thumb { path, .. } => {
                let data = std::fs::read(path)?;
                let img = decode::decode_jpeg_bytes(&data)?;
                Ok(to_color_image(&img))
            }
            Job::Preview { path, format, .. } => {
                let d = decode::decode_full(path, *format)?;
                let small = decode::resize_long_edge(d.image, PREVIEW_EDGE)?;
                let oriented = decode::apply_orientation(small, decode::read_orientation(path, *format));
                Ok(to_color_image(&oriented))
            }
            Job::Crop { path, format, center, .. } => {
                let d = decode::decode_full(path, *format)?;
                let crop = decode::crop_centered(&d.image, center[0], center[1], CROP_W, CROP_H);
                let oriented = decode::apply_orientation(crop, decode::read_orientation(path, *format));
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
    thumb_q: Arc<Queue>,
    big_q: Arc<Queue>,
    rx: Receiver<Done>,
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
            thumb_q,
            big_q,
            rx,
        }
    }

    /// Forget everything (another folder was opened).
    pub fn clear(&mut self) {
        self.thumbs.clear();
        self.previews.clear();
        self.crops.clear();
    }

    /// Upload finished decodes.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.tick += 1;
        let mut uploaded = 0;
        while uploaded < 48 {
            let Ok(done) = self.rx.try_recv() else { break };
            uploaded += 1;
            let cache = match done.kind {
                Kind::Thumb => &mut self.thumbs,
                Kind::Preview => &mut self.previews,
                Kind::Crop => &mut self.crops,
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

    pub fn thumb(&mut self, path: &Path) -> Loaded {
        let key = path.to_string_lossy().to_string();
        if let Some(l) = Self::lookup(&mut self.thumbs, &key, self.tick) {
            return l;
        }
        self.thumbs.pending.insert(key.clone(), Instant::now());
        self.thumb_q.push(Job::Thumb { key, path: path.to_path_buf() });
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
