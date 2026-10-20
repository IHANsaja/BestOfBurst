//! Live engine counters shared by the workers, the CLI and the window.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

#[derive(Default)]
pub struct EngineStats {
    /// Workers currently working on a photo.
    pub busy: AtomicUsize,
    /// Workers currently decoding (CPU).
    pub decoding: AtomicUsize,
    /// Workers waiting for or using the GPU.
    pub on_gpu: AtomicUsize,
    pub gpu_photos: AtomicU64,
    pub cpu_photos: AtomicU64,
    pub gpu_fallbacks: AtomicU64,
    pub decode_us: AtomicU64,
    pub gpu_us: AtomicU64,
    pub cpu_analyze_us: AtomicU64,
    /// Worker threads in the current run.
    pub threads: AtomicUsize,
    /// 0 idle, 1 CPU only, 2 GPU + CPU.
    pub engine: AtomicUsize,
}

pub static STATS: EngineStats = EngineStats {
    busy: AtomicUsize::new(0),
    decoding: AtomicUsize::new(0),
    on_gpu: AtomicUsize::new(0),
    gpu_photos: AtomicU64::new(0),
    cpu_photos: AtomicU64::new(0),
    gpu_fallbacks: AtomicU64::new(0),
    decode_us: AtomicU64::new(0),
    gpu_us: AtomicU64::new(0),
    cpu_analyze_us: AtomicU64::new(0),
    threads: AtomicUsize::new(0),
    engine: AtomicUsize::new(0),
};

/// A plain copy of the counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatsView {
    pub busy: usize,
    pub decoding: usize,
    pub on_gpu: usize,
    pub gpu_photos: u64,
    pub cpu_photos: u64,
    pub gpu_fallbacks: u64,
    pub decode_ms_avg: f32,
    pub gpu_ms_avg: f32,
    pub cpu_ms_avg: f32,
    pub threads: usize,
    pub engine: usize,
}

impl EngineStats {
    pub fn reset(&self, threads: usize, gpu: bool) {
        for a in [&self.busy, &self.decoding, &self.on_gpu] {
            a.store(0, Relaxed);
        }
        for a in [&self.gpu_photos, &self.cpu_photos, &self.gpu_fallbacks, &self.decode_us, &self.gpu_us, &self.cpu_analyze_us] {
            a.store(0, Relaxed);
        }
        self.threads.store(threads, Relaxed);
        self.engine.store(if gpu { 2 } else { 1 }, Relaxed);
    }

    pub fn view(&self) -> StatsView {
        let gp = self.gpu_photos.load(Relaxed);
        let cp = self.cpu_photos.load(Relaxed);
        let all = (gp + cp).max(1);
        StatsView {
            busy: self.busy.load(Relaxed),
            decoding: self.decoding.load(Relaxed),
            on_gpu: self.on_gpu.load(Relaxed),
            gpu_photos: gp,
            cpu_photos: cp,
            gpu_fallbacks: self.gpu_fallbacks.load(Relaxed),
            decode_ms_avg: self.decode_us.load(Relaxed) as f32 / all as f32 / 1000.0,
            gpu_ms_avg: self.gpu_us.load(Relaxed) as f32 / gp.max(1) as f32 / 1000.0,
            cpu_ms_avg: self.cpu_analyze_us.load(Relaxed) as f32 / cp.max(1) as f32 / 1000.0,
            threads: self.threads.load(Relaxed),
            engine: self.engine.load(Relaxed),
        }
    }
}

/// Decrements a counter when dropped (so a panicking decoder never leaves it raised).
pub struct Busy(&'static AtomicUsize);

impl Busy {
    pub fn new(c: &'static AtomicUsize) -> Self {
        c.fetch_add(1, Relaxed);
        Busy(c)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}
