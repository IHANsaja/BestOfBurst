//! Live hardware readings for the window: CPU cores and usage, GPU usage (Windows performance
//! counters, any vendor) and, on NVIDIA cards, core count and memory via NVML.
//! Sampled on a background thread; the UI only reads the latest snapshot.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Logical processors (what Windows schedules threads on).
    pub logical: usize,
    /// Physical cores.
    pub physical: usize,
    /// Whole-machine CPU use, 0..100.
    pub cpu_total: f32,
    /// BestOfBurst's own CPU use as a share of the whole machine, 0..100.
    pub cpu_app: f32,
    /// GPU busy, whole card (busiest engine), 0..100. None when the counters are unavailable.
    pub gpu_total: Option<f32>,
    /// GPU busy from BestOfBurst alone.
    pub gpu_app: Option<f32>,
    pub gpu_name: Option<String>,
    /// Shader cores (NVIDIA only; other drivers do not report it).
    pub gpu_cores: Option<u32>,
    pub gpu_mem_used_mb: Option<u64>,
    pub gpu_mem_total_mb: Option<u64>,
    pub samples: u64,
}

pub struct Monitor {
    snap: Arc<Mutex<Snapshot>>,
}

impl Monitor {
    /// Start sampling every 700 ms.
    pub fn start() -> Self {
        let snap = Arc::new(Mutex::new(Snapshot { logical: logical(), physical: physical_cores(), ..Default::default() }));
        let s2 = snap.clone();
        let _ = std::thread::Builder::new().name("bob-sysmon".into()).spawn(move || {
            let mut cpu = CpuSampler::new();
            let mut pdh = Pdh::open();
            let nvml = Nvml::load();
            loop {
                let (total, app) = cpu.sample();
                let (gt, ga) = pdh.as_mut().map(|p| p.sample()).unwrap_or((None, None));
                let nv = nvml.as_ref().and_then(|n| n.read());
                if let Ok(mut s) = s2.lock() {
                    s.cpu_total = total;
                    s.cpu_app = app;
                    s.gpu_total = gt.or(nv.as_ref().and_then(|n| n.util));
                    s.gpu_app = ga;
                    if let Some(n) = &nv {
                        s.gpu_name = Some(n.name.clone());
                        s.gpu_cores = n.cores;
                        s.gpu_mem_used_mb = Some(n.mem_used / (1024 * 1024));
                        s.gpu_mem_total_mb = Some(n.mem_total / (1024 * 1024));
                    }
                    s.samples += 1;
                }
                std::thread::sleep(Duration::from_millis(700));
            }
        });
        Monitor { snap }
    }

    pub fn get(&self) -> Snapshot {
        self.snap.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

pub fn logical() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Physical cores from GetLogicalProcessorInformation.
pub fn physical_cores() -> usize {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
    };
    let mut len = 0u32;
    // SAFETY: first call only reports the needed size; the second fills a buffer of that size.
    unsafe {
        let _ = GetLogicalProcessorInformation(None, &mut len);
        let n = len as usize / std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
        if n == 0 {
            return logical();
        }
        let mut buf = vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default(); n];
        if GetLogicalProcessorInformation(Some(buf.as_mut_ptr()), &mut len).is_err() {
            return logical();
        }
        buf.iter().filter(|i| i.Relationship == RelationProcessorCore).count().max(1)
    }
}

fn ft(f: windows::Win32::Foundation::FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

struct CpuSampler {
    last: Option<(u64, u64, u64)>,
}

impl CpuSampler {
    fn new() -> Self {
        CpuSampler { last: None }
    }

    /// (whole machine %, this process % of the whole machine).
    fn sample(&mut self) -> (f32, f32) {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetSystemTimes};
        let (mut idle, mut kern, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        let (mut c, mut e, mut pk, mut pu) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: plain Win32 queries writing into stack FILETIMEs.
        let ok = unsafe {
            GetSystemTimes(Some(&mut idle), Some(&mut kern), Some(&mut user)).is_ok()
                && GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut pk, &mut pu).is_ok()
        };
        if !ok {
            return (0.0, 0.0);
        }
        // Kernel time includes idle time.
        let busy_total = ft(kern) + ft(user) - ft(idle);
        let all = ft(kern) + ft(user);
        let proc_t = ft(pk) + ft(pu);
        let cur = (busy_total, all, proc_t);
        let r = match self.last {
            Some((b0, a0, p0)) => {
                let da = (cur.1 - a0).max(1) as f32;
                (((cur.0 - b0) as f32 / da) * 100.0, ((cur.2 - p0) as f32 / da) * 100.0)
            }
            None => (0.0, 0.0),
        };
        self.last = Some(cur);
        (r.0.clamp(0.0, 100.0), r.1.clamp(0.0, 100.0))
    }
}

/// "\GPU Engine(*)\Utilization Percentage", the counters Task Manager uses.
struct Pdh {
    query: windows::Win32::System::Performance::PDH_HQUERY,
    counter: windows::Win32::System::Performance::PDH_HCOUNTER,
    pid: u32,
}

impl Pdh {
    fn open() -> Option<Self> {
        use windows::core::w;
        use windows::Win32::System::Performance::{PdhAddEnglishCounterW, PdhCollectQueryData, PdhOpenQueryW};
        let mut q = windows::Win32::System::Performance::PDH_HQUERY::default();
        let mut c = windows::Win32::System::Performance::PDH_HCOUNTER::default();
        // SAFETY: PDH handles are owned by this struct for the life of the sampler thread.
        unsafe {
            if PdhOpenQueryW(None, 0, &mut q) != 0 {
                return None;
            }
            if PdhAddEnglishCounterW(q, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut c) != 0 {
                return None;
            }
            let _ = PdhCollectQueryData(q);
        }
        Some(Pdh { query: q, counter: c, pid: std::process::id() })
    }

    /// (whole card %, this process %): the busiest engine type, summed over its instances.
    fn sample(&mut self) -> (Option<f32>, Option<f32>) {
        use windows::Win32::System::Performance::{
            PdhCollectQueryData, PdhGetFormattedCounterArrayW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        };
        use std::collections::HashMap;
        // SAFETY: the buffer is sized by the first call and only read within its item count.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return (None, None);
            }
            let mut size = 0u32;
            let mut count = 0u32;
            let _ = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if size == 0 {
                return (Some(0.0), Some(0.0));
            }
            let mut buf = vec![0u8; size as usize];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                return (None, None);
            }
            let tag = format!("pid_{}_", self.pid);
            let mut all: HashMap<String, f64> = HashMap::new();
            let mut mine: HashMap<String, f64> = HashMap::new();
            for k in 0..count as usize {
                let it = &*items.add(k);
                let name = it.szName.to_string().unwrap_or_default();
                let v = it.FmtValue.Anonymous.doubleValue;
                let eng = name.rsplit("engtype_").next().unwrap_or("").to_string();
                *all.entry(eng.clone()).or_default() += v;
                if name.contains(&tag) {
                    *mine.entry(eng).or_default() += v;
                }
            }
            let peak = |m: &HashMap<String, f64>| m.values().cloned().fold(0.0, f64::max).min(100.0) as f32;
            (Some(peak(&all)), Some(peak(&mine)))
        }
    }
}

// NVML, loaded at run time so machines without an NVIDIA driver are unaffected.
type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlHandle = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
type NvmlName = unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> i32;
type NvmlCores = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;
#[repr(C)]
struct NvUtil {
    gpu: u32,
    memory: u32,
}
type NvmlUtil = unsafe extern "C" fn(*mut c_void, *mut NvUtil) -> i32;
#[repr(C)]
struct NvMem {
    total: u64,
    free: u64,
    used: u64,
}
type NvmlMem = unsafe extern "C" fn(*mut c_void, *mut NvMem) -> i32;

struct Nvml {
    dev: *mut c_void,
    name: String,
    cores: Option<u32>,
    util: Option<NvmlUtil>,
    mem: Option<NvmlMem>,
}

// SAFETY: the NVML device handle is only used from the sampler thread that created it.
unsafe impl Send for Nvml {}

struct NvRead {
    name: String,
    cores: Option<u32>,
    util: Option<f32>,
    mem_used: u64,
    mem_total: u64,
}

impl Nvml {
    fn load() -> Option<Self> {
        use windows::core::{s, w};
        use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        // SAFETY: symbols are looked up by their documented NVML names and called with the
        // documented signatures; a missing symbol leaves that reading empty.
        unsafe {
            let lib = LoadLibraryW(w!("nvml.dll")).ok()?;
            let init: NvmlInit = std::mem::transmute(GetProcAddress(lib, s!("nvmlInit_v2"))?);
            if init() != 0 {
                return None;
            }
            let handle: NvmlHandle = std::mem::transmute(GetProcAddress(lib, s!("nvmlDeviceGetHandleByIndex_v2"))?);
            let mut dev: *mut c_void = std::ptr::null_mut();
            if handle(0, &mut dev) != 0 {
                return None;
            }
            let mut name = String::new();
            if let Some(f) = GetProcAddress(lib, s!("nvmlDeviceGetName")) {
                let f: NvmlName = std::mem::transmute(f);
                let mut buf = [0u8; 96];
                if f(dev, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    name = String::from_utf8_lossy(&buf[..end]).to_string();
                }
            }
            let cores = GetProcAddress(lib, s!("nvmlDeviceGetNumGpuCores")).and_then(|f| {
                let f: NvmlCores = std::mem::transmute(f);
                let mut n = 0u32;
                (f(dev, &mut n) == 0 && n > 0).then_some(n)
            });
            let util = GetProcAddress(lib, s!("nvmlDeviceGetUtilizationRates")).map(|f| std::mem::transmute::<_, NvmlUtil>(f));
            let mem = GetProcAddress(lib, s!("nvmlDeviceGetMemoryInfo")).map(|f| std::mem::transmute::<_, NvmlMem>(f));
            Some(Nvml { dev, name, cores, util, mem })
        }
    }

    fn read(&self) -> Option<NvRead> {
        // SAFETY: see load().
        unsafe {
            let util = self.util.and_then(|f| {
                let mut u = NvUtil { gpu: 0, memory: 0 };
                (f(self.dev, &mut u) == 0).then_some(u.gpu as f32)
            });
            let (mut used, mut total) = (0, 0);
            if let Some(f) = self.mem {
                let mut m = NvMem { total: 0, free: 0, used: 0 };
                if f(self.dev, &mut m) == 0 {
                    used = m.used;
                    total = m.total;
                }
            }
            Some(NvRead { name: self.name.clone(), cores: self.cores, util, mem_used: used, mem_total: total })
        }
    }
}

/// Known CUDA core counts for cards whose drivers predate nvmlDeviceGetNumGpuCores.
pub fn known_cores(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    let table: &[(&str, u32)] = &[
        ("gtx 650 ti boost", 768),
        ("gtx 650 ti", 768),
        ("gtx 650", 384),
        ("gtx 660 ti", 1344),
        ("gtx 660", 960),
        ("gtx 670", 1344),
        ("gtx 680", 1536),
        ("gtx 750 ti", 640),
        ("gtx 750", 512),
        ("gtx 760", 1152),
        ("gtx 770", 1536),
        ("gtx 780", 2304),
        ("gtx 950", 768),
        ("gtx 960", 1024),
        ("gtx 970", 1664),
        ("gtx 980", 2048),
        ("gtx 1050 ti", 768),
        ("gtx 1050", 640),
        ("gtx 1060", 1280),
        ("gtx 1070", 1920),
        ("gtx 1080", 2560),
        ("gtx 1650", 896),
        ("gtx 1660", 1408),
    ];
    table.iter().find(|(k, _)| n.contains(k)).map(|(_, c)| *c)
}
