//! GPU analysis with wgpu compute shaders (DirectX 12 or Vulkan).
//!
//! Decoding stays on the CPU. The GPU takes the decoded photo and does everything after it:
//! the Catmull-Rom resize to 2,048 px, the per-tile sums behind the eight metrics, the dHash
//! cells and the 320 px thumbnail. Only a few kilobytes come back. The finishing math is the
//! same function the CPU path uses ([`crate::analyze::finish`]).
//!
//! Any problem (no usable adapter, a validation error, a lost device, a photo too large for one
//! buffer) returns an error and the caller falls back to the CPU.

// SPEC-GAP: the prototype spec kept GPU out of scope; this module adds it as an optional,
// automatically falling-back engine. GPU sums use f32, so metrics match the CPU path to about
// four significant digits rather than bit for bit.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::{anyhow, bail, Result};
use eframe::wgpu;

use crate::analyze::{HashCells, TileSums};
use crate::decode::{self, RgbImage};

const SHADER: &str = r#"
struct Params {
    sw: u32, sh: u32, dw: u32, dh: u32,
    src_rgba: u32, p0: u32, p1: u32, p2: u32,
    scale: f32, p3: f32, p4: f32, p5: f32,
};
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read_write> outf: array<f32>;

fn byte_at(i: u32) -> f32 {
    return f32((src[i >> 2u] >> ((i & 3u) * 8u)) & 255u);
}

fn px(idx: u32) -> vec3<f32> {
    if (P.src_rgba == 1u) {
        let w = src[idx];
        return vec3<f32>(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u));
    }
    let b = idx * 3u;
    return vec3<f32>(byte_at(b), byte_at(b + 1u), byte_at(b + 2u));
}

fn pack(c: vec3<f32>) -> u32 {
    let q = clamp(round(c), vec3<f32>(0.0), vec3<f32>(255.0));
    return u32(q.x) | (u32(q.y) << 8u) | (u32(q.z) << 16u) | (255u << 24u);
}

// Catmull-Rom (cubic, B = 0, C = 0.5), support 2.
fn cubic(x: f32) -> f32 {
    let a = abs(x);
    if (a < 1.0) { return (1.5 * a - 2.5) * a * a + 1.0; }
    if (a < 2.0) { return ((-0.5 * a + 2.5) * a - 4.0) * a + 2.0; }
    return 0.0;
}

// Horizontal pass: sw x sh -> dw x sh.
@compute @workgroup_size(16, 16)
fn hresize(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    if (x >= P.dw || y >= P.sh) { return; }
    let fs = max(P.scale, 1.0);
    let center = (f32(x) + 0.5) * P.scale;
    let left = u32(max(floor(center - 2.0 * fs), 0.0));
    let right = u32(min(ceil(center + 2.0 * fs), f32(P.sw)));
    var acc = vec3<f32>(0.0);
    var ws = 0.0;
    for (var i = left; i < right; i = i + 1u) {
        let w = cubic((f32(i) + 0.5 - center) / fs);
        acc = acc + w * px(y * P.sw + i);
        ws = ws + w;
    }
    dst[y * P.dw + x] = pack(acc / ws);
}

// Vertical pass over RGBA: dw x sh -> dw x dh.
@compute @workgroup_size(16, 16)
fn vresize(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    if (x >= P.dw || y >= P.dh) { return; }
    let fs = max(P.scale, 1.0);
    let center = (f32(y) + 0.5) * P.scale;
    let top = u32(max(floor(center - 2.0 * fs), 0.0));
    let bottom = u32(min(ceil(center + 2.0 * fs), f32(P.sh)));
    var acc = vec3<f32>(0.0);
    var ws = 0.0;
    for (var i = top; i < bottom; i = i + 1u) {
        let w = cubic((f32(i) + 0.5 - center) / fs);
        acc = acc + w * px(i * P.dw + x);
        ws = ws + w;
    }
    dst[y * P.dw + x] = pack(acc / ws);
}

fn luma_at(x: u32, y: u32) -> f32 {
    return dot(px(y * P.dw + x), vec3<f32>(0.2126, 0.7152, 0.0722));
}

var<workgroup> red: array<array<f32, 13>, 256>;

// One workgroup per tile of the 8 x 8 grid: 11 sums over interior pixels, plus clipping counts.
@compute @workgroup_size(256)
fn tiles(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let W = P.dw;
    let H = P.dh;
    let x0 = (wid.x * W + 7u) / 8u;
    let x1 = ((wid.x + 1u) * W + 7u) / 8u;
    let y0 = (wid.y * H + 7u) / 8u;
    let y1 = ((wid.y + 1u) * H + 7u) / 8u;
    let tw = x1 - x0;
    let count = tw * (y1 - y0);
    var s: array<f32, 13>;
    for (var j = 0u; j < 13u; j = j + 1u) { s[j] = 0.0; }
    for (var k = lid; k < count; k = k + 256u) {
        let x = x0 + k % tw;
        let y = y0 + k / tw;
        let p = px(y * W + x);
        let c = dot(p, vec3<f32>(0.2126, 0.7152, 0.0722));
        if (c < 4.5) { s[11] = s[11] + 1.0; }
        if (max(max(p.x, p.y), p.z) >= 251.0) { s[12] = s[12] + 1.0; }
        if (x >= 1u && x + 1u < W && y >= 1u && y + 1u < H) {
            let n = luma_at(x, y - 1u);
            let so = luma_at(x, y + 1u);
            let wv = luma_at(x - 1u, y);
            let e = luma_at(x + 1u, y);
            let nw = luma_at(x - 1u, y - 1u);
            let ne = luma_at(x + 1u, y - 1u);
            let sw = luma_at(x - 1u, y + 1u);
            let se = luma_at(x + 1u, y + 1u);
            let lap = n + so + wv + e - 4.0 * c;
            let gx = (ne + 2.0 * e + se) - (nw + 2.0 * wv + sw);
            let gy = (sw + 2.0 * so + se) - (nw + 2.0 * n + ne);
            let nz = 4.0 * c - 2.0 * (n + so + wv + e) + (nw + ne + sw + se);
            let rg = p.x - p.y;
            let yb = 0.5 * (p.x + p.y) - p.z;
            let lc = c - 128.0;
            s[0] = s[0] + 1.0;
            s[1] = s[1] + lc;
            s[2] = s[2] + lc * lc;
            s[3] = s[3] + lap;
            s[4] = s[4] + lap * lap;
            s[5] = s[5] + gx * gx + gy * gy;
            s[6] = s[6] + abs(nz);
            s[7] = s[7] + rg;
            s[8] = s[8] + rg * rg;
            s[9] = s[9] + yb;
            s[10] = s[10] + yb * yb;
        }
    }
    red[lid] = s;
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        if (lid < stride) {
            for (var j = 0u; j < 13u; j = j + 1u) {
                red[lid][j] = red[lid][j] + red[lid + stride][j];
            }
        }
        workgroupBarrier();
    }
    if (lid == 0u) {
        let base = (wid.y * 8u + wid.x) * 13u;
        for (var j = 0u; j < 13u; j = j + 1u) { outf[base + j] = red[0][j]; }
    }
}

var<workgroup> red2: array<vec2<f32>, 256>;

// One workgroup per dHash cell of the 9 x 8 grid: summed luma and pixel count.
@compute @workgroup_size(256)
fn cells(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let W = P.dw;
    let H = P.dh;
    let x0 = (wid.x * W + 8u) / 9u;
    let x1 = ((wid.x + 1u) * W + 8u) / 9u;
    let y0 = (wid.y * H + 7u) / 8u;
    let y1 = ((wid.y + 1u) * H + 7u) / 8u;
    let tw = x1 - x0;
    let count = tw * (y1 - y0);
    var acc = vec2<f32>(0.0);
    for (var k = lid; k < count; k = k + 256u) {
        acc = acc + vec2<f32>(luma_at(x0 + k % tw, y0 + k / tw), 1.0);
    }
    red2[lid] = acc;
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        if (lid < stride) { red2[lid] = red2[lid] + red2[lid + stride]; }
        workgroupBarrier();
    }
    if (lid == 0u) {
        let base = (wid.y * 9u + wid.x) * 2u;
        outf[base] = red2[0].x;
        outf[base + 1u] = red2[0].y;
    }
}
"#;

/// Threads per workgroup in the tile and dHash passes.
pub const THREADS_PER_GROUP: u32 = 256;

#[derive(Clone, Debug, Default)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub max_invocations_per_group: u32,
    pub max_buffer_mb: u64,
}

pub struct GpuOut {
    pub sums: TileSums,
    pub cells: HashCells,
    /// The 320 px thumbnail, made on the GPU from the analysis image.
    pub thumb: RgbImage,
    pub width: u32,
    pub height: u32,
    pub gpu_ms: f32,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    hresize: wgpu::ComputePipeline,
    vresize: wgpu::ComputePipeline,
    tiles: wgpu::ComputePipeline,
    cells: wgpu::ComputePipeline,
    max_binding: u64,
    pub info: GpuInfo,
    /// One photo on the GPU at a time keeps memory use predictable on small cards.
    lock: Mutex<()>,
}

static GPU: OnceLock<Result<Arc<Gpu>, String>> = OnceLock::new();

/// The shared GPU, created on first use. Err carries a plain reason ("no compatible GPU").
pub fn get() -> Result<Arc<Gpu>, String> {
    GPU.get_or_init(|| {
        match std::panic::catch_unwind(init) {
            Ok(r) => r.map(Arc::new),
            Err(_) => Err("the graphics driver crashed while starting".into()),
        }
    })
    .clone()
}

/// Status without forcing initialization.
pub fn status() -> Option<Result<GpuInfo, String>> {
    GPU.get().map(|r| r.as_ref().map(|g| g.info.clone()).map_err(|e| e.clone()))
}

fn init() -> Result<Gpu, String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|_| "no compatible graphics card (DirectX 12 or Vulkan) was found".to_string())?;
    let ai = adapter.get_info();
    if ai.device_type == wgpu::DeviceType::Cpu {
        return Err(format!("only a software renderer is available ({})", ai.name));
    }
    let al = adapter.limits();
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: al.max_storage_buffer_binding_size,
        max_buffer_size: al.max_buffer_size,
        ..wgpu::Limits::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bestofburst"),
        required_limits: limits,
        ..Default::default()
    }))
    .map_err(|e| format!("the graphics card refused to start: {e}"))?;
    device.on_uncaptured_error(Arc::new(|_e| {}));

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bestofburst-analysis"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipe = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let gpu = Gpu {
        hresize: pipe("hresize"),
        vresize: pipe("vresize"),
        tiles: pipe("tiles"),
        cells: pipe("cells"),
        max_binding: al.max_storage_buffer_binding_size as u64,
        info: GpuInfo {
            name: ai.name.clone(),
            backend: format!("{:?}", ai.backend),
            device_type: match ai.device_type {
                wgpu::DeviceType::DiscreteGpu => "dedicated graphics card".into(),
                wgpu::DeviceType::IntegratedGpu => "built-in graphics".into(),
                wgpu::DeviceType::VirtualGpu => "virtual graphics".into(),
                _ => "graphics".into(),
            },
            driver: if ai.driver_info.is_empty() { ai.driver.clone() } else { ai.driver_info.clone() },
            max_invocations_per_group: al.max_compute_invocations_per_workgroup,
            max_buffer_mb: al.max_storage_buffer_binding_size as u64 / (1024 * 1024),
        },
        device,
        queue,
        lock: Mutex::new(()),
    };
    // Prove the whole path once on a tiny image before trusting it.
    let probe = RgbImage { width: 96, height: 72, rgb: (0..96 * 72 * 3).map(|i| (i * 7 % 251) as u8).collect() };
    gpu.analyze(&probe, decode::ANALYSIS_EDGE, 320).map_err(|e| format!("self-test failed: {e:#}"))?;
    Ok(gpu)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
    src_rgba: u32,
    pad: [u32; 3],
    scale: f32,
    pad2: [f32; 3],
}

impl Params {
    fn bytes(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        let words: [u32; 12] = [
            self.sw,
            self.sh,
            self.dw,
            self.dh,
            self.src_rgba,
            0,
            0,
            0,
            self.scale.to_bits(),
            0,
            0,
            0,
        ];
        for (k, w) in words.iter().enumerate() {
            b[k * 4..k * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        b
    }
}

fn div_up(a: u32, b: u32) -> u32 {
    a.div_ceil(b)
}

impl Gpu {
    fn storage(&self, size: u64, extra: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size.max(4).next_multiple_of(4),
            usage: wgpu::BufferUsages::STORAGE | extra,
            mapped_at_creation: false,
        })
    }

    fn uniform(&self, p: Params) -> wgpu::Buffer {
        let b = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&b, 0, &p.bytes());
        b
    }

    fn bind(&self, pipe: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]) -> wgpu::BindGroup {
        let layout = pipe.get_bind_group_layout(0);
        let e: Vec<wgpu::BindGroupEntry> = entries
            .iter()
            .map(|(b, buf)| wgpu::BindGroupEntry { binding: *b, resource: buf.as_entire_binding() })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &e })
    }

    /// Resize, measure, hash and thumbnail one decoded photo.
    pub fn analyze(&self, img: &RgbImage, edge: u32, thumb_edge: u32) -> Result<GpuOut> {
        let t0 = Instant::now();
        let (sw, sh) = (img.width, img.height);
        let src_len = sw as u64 * sh as u64 * 3;
        if src_len.next_multiple_of(4) > self.max_binding {
            bail!("photo is larger than one GPU buffer");
        }
        let (dw, dh) = decode::target_dims(sw, sh, edge);
        let (tw, th) = decode::target_dims(dw, dh, thumb_edge);
        let _guard = self.lock.lock().map_err(|_| anyhow!("GPU lock poisoned"))?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let src = self.storage(src_len, wgpu::BufferUsages::COPY_DST);
        let aligned = (src_len as usize) / 4 * 4;
        self.queue.write_buffer(&src, 0, &img.rgb[..aligned]);
        if aligned < src_len as usize {
            let mut tail = [0u8; 4];
            tail[..src_len as usize - aligned].copy_from_slice(&img.rgb[aligned..src_len as usize]);
            self.queue.write_buffer(&src, aligned as u64, &tail);
        }
        let mid = self.storage(dw as u64 * sh as u64 * 4, wgpu::BufferUsages::empty());
        let out = self.storage(dw as u64 * dh as u64 * 4, wgpu::BufferUsages::empty());
        let tmid = self.storage(tw as u64 * dh as u64 * 4, wgpu::BufferUsages::empty());
        let thumb = self.storage(tw as u64 * th as u64 * 4, wgpu::BufferUsages::COPY_SRC);
        let tiles = self.storage(64 * 13 * 4, wgpu::BufferUsages::COPY_SRC);
        let cells = self.storage(72 * 2 * 4, wgpu::BufferUsages::COPY_SRC);
        let dummy_f = self.storage(4, wgpu::BufferUsages::empty());
        let dummy_u = self.storage(4, wgpu::BufferUsages::empty());

        let p_h = self.uniform(Params { sw, sh, dw, dh, src_rgba: 0, pad: [0; 3], scale: sw as f32 / dw as f32, pad2: [0.0; 3] });
        let p_v = self.uniform(Params { sw: dw, sh, dw, dh, src_rgba: 1, pad: [0; 3], scale: sh as f32 / dh as f32, pad2: [0.0; 3] });
        let p_a = self.uniform(Params { sw: dw, sh: dh, dw, dh, src_rgba: 1, pad: [0; 3], scale: 1.0, pad2: [0.0; 3] });
        let p_th = self.uniform(Params { sw: dw, sh: dh, dw: tw, dh, src_rgba: 1, pad: [0; 3], scale: dw as f32 / tw as f32, pad2: [0.0; 3] });
        let p_tv = self.uniform(Params { sw: tw, sh: dh, dw: tw, dh: th, src_rgba: 1, pad: [0; 3], scale: dh as f32 / th as f32, pad2: [0.0; 3] });

        let bg_h = self.bind(&self.hresize, &[(0, &p_h), (1, &src), (2, &mid)]);
        let bg_v = self.bind(&self.vresize, &[(0, &p_v), (1, &mid), (2, &out)]);
        let bg_t = self.bind(&self.tiles, &[(0, &p_a), (1, &out), (3, &tiles)]);
        let bg_c = self.bind(&self.cells, &[(0, &p_a), (1, &out), (3, &cells)]);
        let bg_th = self.bind(&self.hresize, &[(0, &p_th), (1, &out), (2, &tmid)]);
        let bg_tv = self.bind(&self.vresize, &[(0, &p_tv), (1, &tmid), (2, &thumb)]);
        let _ = (&dummy_f, &dummy_u);

        let rb_tiles = self.readback(64 * 13 * 4);
        let rb_cells = self.readback(72 * 2 * 4);
        let rb_thumb = self.readback(tw as u64 * th as u64 * 4);

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(&self.hresize);
            pass.set_bind_group(0, &bg_h, &[]);
            pass.dispatch_workgroups(div_up(dw, 16), div_up(sh, 16), 1);
            pass.set_pipeline(&self.vresize);
            pass.set_bind_group(0, &bg_v, &[]);
            pass.dispatch_workgroups(div_up(dw, 16), div_up(dh, 16), 1);
            pass.set_pipeline(&self.tiles);
            pass.set_bind_group(0, &bg_t, &[]);
            pass.dispatch_workgroups(8, 8, 1);
            pass.set_pipeline(&self.cells);
            pass.set_bind_group(0, &bg_c, &[]);
            pass.dispatch_workgroups(9, 8, 1);
            pass.set_pipeline(&self.hresize);
            pass.set_bind_group(0, &bg_th, &[]);
            pass.dispatch_workgroups(div_up(tw, 16), div_up(dh, 16), 1);
            pass.set_pipeline(&self.vresize);
            pass.set_bind_group(0, &bg_tv, &[]);
            pass.dispatch_workgroups(div_up(tw, 16), div_up(th, 16), 1);
        }
        enc.copy_buffer_to_buffer(&tiles, 0, &rb_tiles, 0, 64 * 13 * 4);
        enc.copy_buffer_to_buffer(&cells, 0, &rb_cells, 0, 72 * 2 * 4);
        enc.copy_buffer_to_buffer(&thumb, 0, &rb_thumb, 0, tw as u64 * th as u64 * 4);
        self.queue.submit(Some(enc.finish()));

        let tile_f = self.read_f32(&rb_tiles)?;
        let cell_f = self.read_f32(&rb_cells)?;
        let thumb_raw = self.read_bytes(&rb_thumb)?;
        if let Some(err) = pollster::block_on(scope.pop()) {
            bail!("GPU validation error: {err}");
        }

        let mut sums = TileSums { tiles: [[0f64; 11]; 64], lo: 0, hi: 0, n_all: dw as f64 * dh as f64 };
        let (mut lo, mut hi) = (0f64, 0f64);
        for t in 0..64 {
            for k in 0..11 {
                sums.tiles[t][k] = tile_f[t * 13 + k] as f64;
            }
            lo += tile_f[t * 13 + 11] as f64;
            hi += tile_f[t * 13 + 12] as f64;
        }
        sums.lo = lo.round() as u64;
        sums.hi = hi.round() as u64;
        let mut hc = HashCells { sums: [[0f64; 9]; 8], counts: [[0f64; 9]; 8] };
        for r in 0..8 {
            for c in 0..9 {
                hc.sums[r][c] = cell_f[(r * 9 + c) * 2] as f64;
                hc.counts[r][c] = cell_f[(r * 9 + c) * 2 + 1] as f64;
            }
        }
        let rgb: Vec<u8> = thumb_raw.chunks_exact(4).take((tw * th) as usize).flat_map(|p| [p[0], p[1], p[2]]).collect();
        Ok(GpuOut {
            sums,
            cells: hc,
            thumb: RgbImage::new(tw, th, rgb)?,
            width: dw,
            height: dh,
            gpu_ms: t0.elapsed().as_secs_f32() * 1000.0,
        })
    }

    fn readback(&self, size: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size.next_multiple_of(4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn read_bytes(&self, buf: &wgpu::Buffer) -> Result<Vec<u8>> {
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow!("GPU did not finish: {e}"))?;
        rx.recv().map_err(|_| anyhow!("GPU readback lost"))?.map_err(|e| anyhow!("GPU readback failed: {e}"))?;
        let data = slice.get_mapped_range().map_err(|e| anyhow!("GPU readback failed: {e:?}"))?.to_vec();
        buf.unmap();
        Ok(data)
    }

    fn read_f32(&self, buf: &wgpu::Buffer) -> Result<Vec<f32>> {
        Ok(self.read_bytes(buf)?.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
    }
}
