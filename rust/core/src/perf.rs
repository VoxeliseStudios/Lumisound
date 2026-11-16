//! Performance telemetry: what the app costs the phone, all the time, cheaply.
//!
//! A sampler thread (1 s in the foreground, 5 s in the background) reads,
//! straight from the kernel:
//! * CPU per thread (Mach `THREAD_EXTENDED_INFO`: user+system ns and the
//!   thread's name, so the engine's decode/render threads, tokio workers and
//!   the main thread are told apart), process total, thread count, and the
//!   system-wide load (`host_statistics`);
//! * memory: physical footprint (what jetsam counts) and, on iOS, how much
//!   is left before the OS kills the app (`os_proc_available_memory`);
//! * GPU time the process submitted and its energy use (Mach
//!   `TASK_POWER_INFO_V2`: `task_gpu_utilisation` ns, `task_energy` nJ on
//!   arm64) plus interrupt wakeups — iOS has no public temperature reading,
//!   so energy (mW) is the heat proxy next to the OS thermal state.
//!
//! The app feeds what only UIKit sees: frame intervals from a display link
//! (`perf_record_frames`, batched ~1/s), thermal state changes, battery and
//! Low Power Mode, and which screen is showing.
//!
//! Everything is folded into a window (`perf_take_window`, the app takes one
//! a minute) with percentiles, hitch time ratio (Apple's ms-per-second
//! measure), dropped/frozen frames per screen, thermal time-in-state,
//! battery drain — and `interesting` + reasons, so the app uploads only
//! windows that show a problem (plus a periodic heartbeat). `perf_live` is
//! the last sample, for the in-app Performance Monitor.
//!
//! Cost: one sample is ~40 µs on a phone (≈30 threads); frames are a
//! counter bump and a histogram slot each. Linux (tests) reads /proc.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

// ---- records ----------------------------------------------------------------------------------

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq)]
pub struct ThreadCpu {
    pub name: String,
    /// Share of one core (100 = a whole core) over the window / sample.
    pub cpu_pct: f64,
    pub cpu_ms: f64,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub frames: u64,
    pub fps: f64,
    /// The display's frame budget (16.7 ms at 60 Hz).
    pub expected_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Frames the display had to repeat because ours were late.
    pub dropped: u64,
    /// Late frames (> 1.5x budget).
    pub hitches: u64,
    /// Frames of 100 ms or more (visible stutter).
    pub long_hitches: u64,
    /// Frames of 700 ms or more (the UI froze).
    pub frozen: u64,
    pub hitch_ms: f64,
    /// Hitch time per second of frames (Apple: < 5 good, 5–10 warning, > 10 critical).
    pub hitch_ratio: f64,
    pub rating: String,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq)]
pub struct ScreenPerf {
    pub screen: String,
    pub seconds: f64,
    pub hitch_ratio: f64,
    pub worst_frame_ms: f64,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq)]
pub struct PerfWindow {
    pub seconds: f64,
    pub foreground_seconds: f64,
    pub cores: u32,
    /// Process CPU, 100 = one core.
    pub cpu_avg_pct: f64,
    pub cpu_max_pct: f64,
    /// Whole device (all apps), 0–100.
    pub system_cpu_pct: f64,
    pub threads: Vec<ThreadCpu>,
    pub thread_count_max: u32,
    /// GPU time this process submitted.
    pub gpu_ms: f64,
    pub gpu_avg_pct: f64,
    pub gpu_max_pct: f64,
    /// Average/peak power of this process (arm64 Apple; 0 = unknown).
    pub energy_mw_avg: f64,
    pub energy_mw_max: f64,
    pub wakeups_per_s: f64,
    pub memory_mb_avg: f64,
    pub memory_mb_max: f64,
    /// Least headroom before the OS would kill the app (-1 = unknown).
    pub memory_available_mb_min: f64,
    pub frames: FrameStats,
    pub screens: Vec<ScreenPerf>,
    pub thermal_now: String,
    pub thermal_max: String,
    pub thermal_changes: u32,
    pub thermal_fair_s: f64,
    pub thermal_serious_s: f64,
    pub thermal_critical_s: f64,
    pub battery_pct: f64,
    /// Battery % per hour while not charging (0 = unknown / charging).
    pub battery_drain_pct_per_hour: f64,
    pub charging: bool,
    pub low_power: bool,
    pub sampler_overhead_us: f64,
    pub interesting: bool,
    pub reasons: Vec<String>,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq)]
pub struct PerfLive {
    pub cpu_pct: f64,
    pub system_cpu_pct: f64,
    pub cores: u32,
    pub threads: Vec<ThreadCpu>,
    pub thread_count: u32,
    pub gpu_pct: f64,
    pub energy_mw: f64,
    pub memory_mb: f64,
    pub memory_available_mb: f64,
    /// Last ~second of frames.
    pub fps: f64,
    pub frame_max_ms: f64,
    pub hitch_ratio: f64,
    pub thermal: String,
    pub battery_pct: f64,
    pub low_power: bool,
    pub screen: String,
    pub sampler_overhead_us: f64,
}

// ---- kernel readers ---------------------------------------------------------------------------

struct ThreadSample {
    key: u64,
    name: String,
    cpu_ns: u64,
}

#[derive(Default, Clone, Copy)]
struct PowerSample {
    gpu_ns: u64,
    energy_nj: u64,
    wakeups: u64,
}

#[cfg(target_vendor = "apple")]
mod sys {
    use super::{PowerSample, ThreadSample};

    unsafe extern "C" {
        fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> libc::kern_return_t;
        #[cfg(target_os = "ios")]
        fn os_proc_available_memory() -> usize;
    }

    fn task() -> libc::mach_port_t {
        unsafe { libc::mach_task_self() }
    }

    pub fn threads() -> Vec<ThreadSample> {
        let mut list: libc::thread_act_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        if unsafe { libc::task_threads(task(), &mut list, &mut count) } != libc::KERN_SUCCESS || list.is_null() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let th = unsafe { *list.add(i) };
            let mut ext: libc::thread_extended_info = unsafe { std::mem::zeroed() };
            let mut n = libc::THREAD_EXTENDED_INFO_COUNT;
            let ok = unsafe { libc::thread_info(th, libc::THREAD_EXTENDED_INFO as _, &mut ext as *mut _ as libc::thread_info_t, &mut n) } == libc::KERN_SUCCESS;
            let mut ident: libc::thread_identifier_info = unsafe { std::mem::zeroed() };
            let mut n2 = (std::mem::size_of::<libc::thread_identifier_info>() / 4) as libc::mach_msg_type_number_t;
            let key = if unsafe { libc::thread_info(th, libc::THREAD_IDENTIFIER_INFO as _, &mut ident as *mut _ as libc::thread_info_t, &mut n2) } == libc::KERN_SUCCESS {
                ident.thread_id
            } else {
                th as u64
            };
            if ok {
                let raw: Vec<u8> = ext.pth_name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
                out.push(ThreadSample { key, name: String::from_utf8_lossy(&raw).into_owned(), cpu_ns: ext.pth_user_time + ext.pth_system_time });
            }
            unsafe { mach_port_deallocate(task(), th) };
        }
        unsafe {
            libc::vm_deallocate(task(), list as libc::vm_address_t, count as usize * std::mem::size_of::<libc::thread_act_t>());
        }
        out
    }

    /// `task_vm_info` up to rev1 (phys_footprint). The header packs it to 4.
    #[repr(C, packed(4))]
    #[derive(Default)]
    struct TaskVmInfo {
        virtual_size: u64,
        region_count: i32,
        page_size: i32,
        resident_size: u64,
        resident_size_peak: u64,
        device: u64,
        device_peak: u64,
        internal: u64,
        internal_peak: u64,
        external: u64,
        external_peak: u64,
        reusable: u64,
        reusable_peak: u64,
        purgeable_volatile_pmap: u64,
        purgeable_volatile_resident: u64,
        purgeable_volatile_virtual: u64,
        compressed: u64,
        compressed_peak: u64,
        compressed_lifetime: u64,
        phys_footprint: u64,
    }
    const TASK_VM_INFO: u32 = 22;

    /// (footprint bytes, bytes left before jetsam if known)
    pub fn memory() -> (u64, Option<u64>) {
        let mut info = TaskVmInfo::default();
        let mut n = (std::mem::size_of::<TaskVmInfo>() / 4) as libc::mach_msg_type_number_t;
        let ok = unsafe { libc::task_info(task(), TASK_VM_INFO as _, &mut info as *mut _ as libc::task_info_t, &mut n) } == libc::KERN_SUCCESS;
        let fp = if ok { info.phys_footprint } else { 0 };
        #[cfg(target_os = "ios")]
        let avail = Some(unsafe { os_proc_available_memory() } as u64);
        #[cfg(not(target_os = "ios"))]
        let avail = None;
        (fp, avail)
    }

    #[repr(C, packed(4))]
    #[derive(Default)]
    struct TaskPowerInfoV2 {
        total_user: u64,
        total_system: u64,
        task_interrupt_wakeups: u64,
        task_platform_idle_wakeups: u64,
        task_timer_wakeups_bin_1: u64,
        task_timer_wakeups_bin_2: u64,
        task_gpu_utilisation: u64,
        task_gpu_stat_reserved0: u64,
        task_gpu_stat_reserved1: u64,
        task_gpu_stat_reserved2: u64,
        #[cfg(target_arch = "aarch64")]
        task_energy: u64,
        task_ptime: u64,
        task_pset_switches: u64,
    }
    const TASK_POWER_INFO_V2: u32 = 26;

    pub fn power() -> Option<PowerSample> {
        let mut p = TaskPowerInfoV2::default();
        let mut n = (std::mem::size_of::<TaskPowerInfoV2>() / 4) as libc::mach_msg_type_number_t;
        if unsafe { libc::task_info(task(), TASK_POWER_INFO_V2 as _, &mut p as *mut _ as libc::task_info_t, &mut n) } != libc::KERN_SUCCESS {
            return None;
        }
        #[cfg(target_arch = "aarch64")]
        let energy = p.task_energy;
        #[cfg(not(target_arch = "aarch64"))]
        let energy = 0;
        Some(PowerSample { gpu_ns: p.task_gpu_utilisation, energy_nj: energy, wakeups: p.task_interrupt_wakeups })
    }

    /// (busy ticks, total ticks) across all cores since boot.
    #[allow(deprecated)] // libc points at the mach2 crate; same call.
    pub fn system_ticks() -> Option<(u64, u64)> {
        let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
        let mut n = (std::mem::size_of::<libc::host_cpu_load_info>() / 4) as libc::mach_msg_type_number_t;
        if unsafe { libc::host_statistics(libc::mach_host_self(), libc::HOST_CPU_LOAD_INFO, &mut info as *mut _ as libc::host_info_t, &mut n) } != libc::KERN_SUCCESS {
            return None;
        }
        let t = info.cpu_ticks;
        let busy = t[libc::CPU_STATE_USER as usize] as u64 + t[libc::CPU_STATE_SYSTEM as usize] as u64 + t[libc::CPU_STATE_NICE as usize] as u64;
        Some((busy, busy + t[libc::CPU_STATE_IDLE as usize] as u64))
    }
}

#[cfg(not(target_vendor = "apple"))]
mod sys {
    use super::{PowerSample, ThreadSample};

    fn ticks_ns() -> u64 {
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
        1_000_000_000 / hz
    }

    pub fn threads() -> Vec<ThreadSample> {
        let tick = ticks_ns();
        let Ok(rd) = std::fs::read_dir("/proc/self/task") else { return Vec::new() };
        rd.flatten()
            .filter_map(|e| {
                let tid: u64 = e.file_name().to_str()?.parse().ok()?;
                let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
                // "tid (comm) state ..." — comm may hold spaces/parens.
                let close = stat.rfind(')')?;
                let name = stat[stat.find('(')? + 1..close].to_string();
                let f: Vec<&str> = stat[close + 2..].split_whitespace().collect();
                let (u, s): (u64, u64) = (f.get(11)?.parse().ok()?, f.get(12)?.parse().ok()?);
                Some(ThreadSample { key: tid, name, cpu_ns: (u + s) * tick })
            })
            .collect()
    }

    pub fn memory() -> (u64, Option<u64>) {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64;
        let rss = std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * page;
        let avail = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
            let l = s.lines().find(|l| l.starts_with("MemAvailable:"))?;
            l.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
        });
        (rss, avail)
    }

    pub fn power() -> Option<PowerSample> {
        None
    }

    pub fn system_ticks() -> Option<(u64, u64)> {
        let s = std::fs::read_to_string("/proc/stat").ok()?;
        let f: Vec<u64> = s.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        let idle = f.get(3)? + f.get(4).unwrap_or(&0);
        let total: u64 = f.iter().take(8).sum();
        Some((total - idle, total))
    }
}

// ---- state ------------------------------------------------------------------------------------

const BUCKET_MS: f64 = 0.5;
const BUCKETS: usize = 2001; // 0..1000 ms in 0.5 ms steps, last = overflow

struct Frames {
    hist: Vec<u32>,
    frames: u64,
    total_ms: f64,
    expected_ms: f64,
    max_ms: f64,
    dropped: u64,
    hitches: u64,
    long_hitches: u64,
    frozen: u64,
    hitch_ms: f64,
}

impl Default for Frames {
    fn default() -> Self {
        Frames { hist: vec![0; BUCKETS], frames: 0, total_ms: 0.0, expected_ms: 0.0, max_ms: 0.0, dropped: 0, hitches: 0, long_hitches: 0, frozen: 0, hitch_ms: 0.0 }
    }
}

impl Frames {
    fn add(&mut self, ms: f64, expected: f64) {
        if !ms.is_finite() || ms <= 0.0 {
            return;
        }
        let expected = if expected > 1.0 { expected } else { 1000.0 / 60.0 };
        self.expected_ms = expected;
        self.frames += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        self.hist[((ms / BUCKET_MS) as usize).min(BUCKETS - 1)] += 1;
        if ms > expected * 1.5 {
            self.hitches += 1;
            self.hitch_ms += ms - expected;
            self.dropped += ((ms / expected).round() as u64).saturating_sub(1);
        }
        if ms >= 100.0 {
            self.long_hitches += 1;
        }
        if ms >= 700.0 {
            self.frozen += 1;
        }
    }

    fn pct(&self, p: f64) -> f64 {
        if self.frames == 0 {
            return 0.0;
        }
        let want = (self.frames as f64 * p).ceil() as u64;
        let mut seen = 0u64;
        for (i, c) in self.hist.iter().enumerate() {
            seen += *c as u64;
            if seen >= want {
                return ((i as f64 + 1.0) * BUCKET_MS).min(self.max_ms);
            }
        }
        self.max_ms
    }

    fn hitch_ratio(&self) -> f64 {
        if self.total_ms <= 0.0 { 0.0 } else { self.hitch_ms / (self.total_ms / 1000.0) }
    }

    fn stats(&self) -> FrameStats {
        let ratio = self.hitch_ratio();
        FrameStats {
            frames: self.frames,
            fps: if self.total_ms > 0.0 { self.frames as f64 / (self.total_ms / 1000.0) } else { 0.0 },
            expected_ms: self.expected_ms,
            p50_ms: self.pct(0.50),
            p95_ms: self.pct(0.95),
            p99_ms: self.pct(0.99),
            max_ms: self.max_ms,
            dropped: self.dropped,
            hitches: self.hitches,
            long_hitches: self.long_hitches,
            frozen: self.frozen,
            hitch_ms: self.hitch_ms,
            hitch_ratio: ratio,
            rating: if self.frames == 0 { "none" } else if ratio < 5.0 { "good" } else if ratio < 10.0 { "warning" } else { "critical" }.into(),
        }
    }
}

#[derive(Default)]
struct ScreenAcc {
    frame_ms: f64,
    hitch_ms: f64,
    worst: f64,
}

struct Window {
    started: Instant,
    fg_secs: f64,
    samples: u32,
    cpu_sum: f64,
    cpu_max: f64,
    sys_sum: f64,
    sys_n: u32,
    thread_ns: HashMap<String, u64>,
    thread_count_max: u32,
    gpu_ns: u64,
    gpu_max: f64,
    energy_nj: u64,
    energy_max_mw: f64,
    wakeups: u64,
    mem_sum: f64,
    mem_max: f64,
    avail_min: f64,
    frames: Frames,
    screens: HashMap<String, ScreenAcc>,
    thermal_secs: [f64; 4],
    thermal_max: i64,
    thermal_changes: u32,
    battery_start: Option<(f64, Instant)>,
    overhead_us_sum: f64,
}

impl Window {
    fn new() -> Self {
        Window {
            started: Instant::now(),
            fg_secs: 0.0,
            samples: 0,
            cpu_sum: 0.0,
            cpu_max: 0.0,
            sys_sum: 0.0,
            sys_n: 0,
            thread_ns: HashMap::new(),
            thread_count_max: 0,
            gpu_ns: 0,
            gpu_max: 0.0,
            energy_nj: 0,
            energy_max_mw: 0.0,
            wakeups: 0,
            mem_sum: 0.0,
            mem_max: 0.0,
            avail_min: f64::MAX,
            frames: Frames::default(),
            screens: HashMap::new(),
            thermal_secs: [0.0; 4],
            thermal_max: 0,
            thermal_changes: 0,
            battery_start: None,
            overhead_us_sum: 0.0,
        }
    }
}

struct State {
    window: Window,
    live: PerfLive,
    /// Recent frames (~1 s) for the live view.
    recent: Frames,
    recent_started: Instant,
    last_threads: HashMap<u64, (String, u64)>,
    last_power: Option<PowerSample>,
    last_sys: Option<(u64, u64)>,
    last_at: Option<Instant>,
    foreground: bool,
    screen: String,
    thermal: i64,
    thermal_since: Instant,
    battery: f64,
    charging: bool,
    low_power: bool,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        window: Window::new(),
        live: PerfLive::default(),
        recent: Frames::default(),
        recent_started: Instant::now(),
        last_threads: HashMap::new(),
        last_power: None,
        last_sys: None,
        last_at: None,
        foreground: true,
        screen: String::new(),
        thermal: 0,
        thermal_since: Instant::now(),
        battery: -1.0,
        charging: false,
        low_power: false,
    })
});
static STARTED: AtomicBool = AtomicBool::new(false);

fn cores() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)
}

pub fn thermal_name(raw: i64) -> &'static str {
    match raw {
        0 => "nominal",
        1 => "fair",
        2 => "serious",
        3 => "critical",
        _ => "unknown",
    }
}

/// Collapse per-instance names ("tokio-runtime-worker", numbered queues)
/// and give unnamed threads a stable label.
fn thread_group(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() {
        return "(unnamed)".into();
    }
    let n = n.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-' || c == '#' || c == ' ' || c == '.');
    if n.is_empty() { "(unnamed)".into() } else { n.to_string() }
}

/// One kernel sample folded into the window and the live view.
fn sample_now() {
    let t0 = Instant::now();
    let threads = sys::threads();
    let (mem, avail) = sys::memory();
    let power = sys::power();
    let systicks = sys::system_ticks();
    let read_us = t0.elapsed().as_secs_f64() * 1e6;

    let mut st = STATE.lock().unwrap();
    let now = Instant::now();
    let dt = st.last_at.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
    st.last_at = Some(now);
    let thermal = st.thermal;
    if dt > 0.0 && (0..4).contains(&thermal) {
        st.window.thermal_secs[thermal as usize] += dt;
    }
    if st.foreground {
        st.window.fg_secs += dt;
    }

    // CPU per thread (delta since last sample; new threads count from 0).
    let mut cur: HashMap<u64, (String, u64)> = HashMap::with_capacity(threads.len());
    let mut total_delta = 0u64;
    let mut per_group: HashMap<String, u64> = HashMap::new();
    for t in &threads {
        let prev = st.last_threads.get(&t.key).map(|p| p.1).unwrap_or(t.cpu_ns);
        let d = t.cpu_ns.saturating_sub(prev);
        total_delta += d;
        if d > 0 {
            *per_group.entry(thread_group(&t.name)).or_default() += d;
        }
        cur.insert(t.key, (t.name.clone(), t.cpu_ns));
    }
    let first = st.last_threads.is_empty();
    st.last_threads = cur;
    let cpu_pct = if dt > 0.0 && !first { total_delta as f64 / (dt * 1e9) * 100.0 } else { 0.0 };

    let (gpu_pct, energy_mw, wake) = match (power, st.last_power) {
        (Some(p), Some(l)) if dt > 0.0 => {
            let g = p.gpu_ns.saturating_sub(l.gpu_ns);
            let e = p.energy_nj.saturating_sub(l.energy_nj);
            st.window.gpu_ns += g;
            st.window.energy_nj += e;
            st.window.wakeups += p.wakeups.saturating_sub(l.wakeups);
            (g as f64 / (dt * 1e9) * 100.0, e as f64 / 1e6 / dt, p.wakeups.saturating_sub(l.wakeups))
        }
        _ => (0.0, 0.0, 0),
    };
    let _ = wake;
    st.last_power = power;
    let sys_pct = match (systicks, st.last_sys) {
        (Some((b, t)), Some((lb, lt))) if t > lt => (b.saturating_sub(lb)) as f64 / (t - lt) as f64 * 100.0,
        _ => -1.0,
    };
    st.last_sys = systicks;

    let mem_mb = mem as f64 / 1_048_576.0;
    let avail_mb = avail.map(|a| a as f64 / 1_048_576.0).unwrap_or(-1.0);
    let w = &mut st.window;
    if !first && dt > 0.0 {
        w.samples += 1;
        w.cpu_sum += cpu_pct;
        w.cpu_max = w.cpu_max.max(cpu_pct);
        for (k, v) in &per_group {
            *w.thread_ns.entry(k.clone()).or_default() += v;
        }
        w.gpu_max = w.gpu_max.max(gpu_pct);
        w.energy_max_mw = w.energy_max_mw.max(energy_mw);
        if sys_pct >= 0.0 {
            w.sys_sum += sys_pct;
            w.sys_n += 1;
        }
    }
    w.thread_count_max = w.thread_count_max.max(threads.len() as u32);
    w.mem_sum += mem_mb;
    w.mem_max = w.mem_max.max(mem_mb);
    if avail_mb >= 0.0 {
        w.avail_min = w.avail_min.min(avail_mb);
    }
    let overhead = t0.elapsed().as_secs_f64() * 1e6;
    w.overhead_us_sum += overhead;

    let mut top: Vec<ThreadCpu> = per_group
        .into_iter()
        .map(|(name, ns)| ThreadCpu { name, cpu_pct: if dt > 0.0 { ns as f64 / (dt * 1e9) * 100.0 } else { 0.0 }, cpu_ms: ns as f64 / 1e6 })
        .collect();
    top.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
    top.truncate(10);
    let recent = st.recent.stats();
    let live = PerfLive {
        cpu_pct,
        system_cpu_pct: sys_pct.max(0.0),
        cores: cores(),
        threads: top,
        thread_count: threads.len() as u32,
        gpu_pct,
        energy_mw,
        memory_mb: mem_mb,
        memory_available_mb: avail_mb,
        fps: recent.fps,
        frame_max_ms: recent.max_ms,
        hitch_ratio: recent.hitch_ratio,
        thermal: thermal_name(st.thermal).into(),
        battery_pct: st.battery,
        low_power: st.low_power,
        screen: st.screen.clone(),
        sampler_overhead_us: read_us,
    };
    st.live = live;
    if now.duration_since(st.recent_started) >= Duration::from_secs(2) {
        st.recent = Frames::default();
        st.recent_started = now;
    }
}

// ---- API --------------------------------------------------------------------------------------

/// Start the sampler (idempotent).
#[uniffi::export]
pub fn perf_start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("lumisound-perf".into()).spawn(|| {
        sample_now();
        loop {
            let fg = STATE.lock().unwrap().foreground;
            std::thread::sleep(Duration::from_secs(if fg { 1 } else { 5 }));
            sample_now();
        }
    });
}

/// Foreground/background and the screen being shown.
#[uniffi::export]
pub fn perf_set_state(foreground: bool, screen: String) {
    let mut st = STATE.lock().unwrap();
    st.foreground = foreground;
    if !screen.is_empty() {
        st.screen = screen;
    }
}

/// Display-link frame intervals (ms) since the last call, and the frame
/// budget (ms). Attributed to the current screen.
#[uniffi::export]
pub fn perf_record_frames(intervals_ms: Vec<f32>, expected_ms: f32) {
    let mut st = STATE.lock().unwrap();
    let screen = st.screen.clone();
    let e = expected_ms as f64;
    let mut acc = ScreenAcc::default();
    for &ms in &intervals_ms {
        let ms = ms as f64;
        st.window.frames.add(ms, e);
        st.recent.add(ms, e);
        if ms.is_finite() && ms > 0.0 {
            acc.frame_ms += ms;
            let budget = if e > 1.0 { e } else { 1000.0 / 60.0 };
            if ms > budget * 1.5 {
                acc.hitch_ms += ms - budget;
            }
            acc.worst = acc.worst.max(ms);
        }
    }
    let s = st.window.screens.entry(if screen.is_empty() { "app".into() } else { screen }).or_default();
    s.frame_ms += acc.frame_ms;
    s.hitch_ms += acc.hitch_ms;
    s.worst = s.worst.max(acc.worst);
}

/// ProcessInfo.ThermalState raw value (0 nominal … 3 critical).
#[uniffi::export]
pub fn perf_record_thermal(raw: i64) {
    let mut st = STATE.lock().unwrap();
    if raw != st.thermal {
        st.window.thermal_changes += 1;
    }
    st.thermal = raw;
    st.thermal_since = Instant::now();
    st.window.thermal_max = st.window.thermal_max.max(raw);
}

/// Battery level 0–1 (negative = unknown), charging, Low Power Mode.
#[uniffi::export]
pub fn perf_record_power(battery_level: f32, charging: bool, low_power: bool) {
    let mut st = STATE.lock().unwrap();
    let pct = if battery_level >= 0.0 { battery_level as f64 * 100.0 } else { -1.0 };
    if charging != st.charging || st.window.battery_start.is_none() {
        st.window.battery_start = (pct >= 0.0).then(|| (pct, Instant::now()));
    }
    st.battery = pct;
    st.charging = charging;
    st.low_power = low_power;
}

/// The latest sample (live view).
#[uniffi::export]
pub fn perf_live() -> PerfLive {
    STATE.lock().unwrap().live.clone()
}

/// Close the current window and start a new one.
#[uniffi::export]
pub fn perf_take_window() -> PerfWindow {
    let mut st = STATE.lock().unwrap();
    let w = std::mem::replace(&mut st.window, Window::new());
    st.window.thermal_max = st.thermal;
    st.window.battery_start = (st.battery >= 0.0 && !st.charging).then(|| (st.battery, Instant::now()));
    let secs = w.started.elapsed().as_secs_f64();
    let n = w.samples.max(1) as f64;
    let mut threads: Vec<ThreadCpu> = w
        .thread_ns
        .iter()
        .map(|(name, ns)| ThreadCpu { name: name.clone(), cpu_pct: if secs > 0.0 { *ns as f64 / (secs * 1e9) * 100.0 } else { 0.0 }, cpu_ms: *ns as f64 / 1e6 })
        .collect();
    threads.sort_by(|a, b| b.cpu_ms.total_cmp(&a.cpu_ms));
    threads.truncate(8);
    let mut screens: Vec<ScreenPerf> = w
        .screens
        .iter()
        .map(|(k, v)| ScreenPerf { screen: k.clone(), seconds: v.frame_ms / 1000.0, hitch_ratio: if v.frame_ms > 0.0 { v.hitch_ms / (v.frame_ms / 1000.0) } else { 0.0 }, worst_frame_ms: v.worst })
        .collect();
    screens.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    screens.truncate(6);
    let drain = match w.battery_start {
        Some((p0, t0)) if !st.charging && st.battery >= 0.0 && t0.elapsed().as_secs_f64() > 120.0 => ((p0 - st.battery) / (t0.elapsed().as_secs_f64() / 3600.0)).max(0.0),
        _ => 0.0,
    };
    let frames = w.frames.stats();
    let mut out = PerfWindow {
        seconds: secs,
        foreground_seconds: w.fg_secs,
        cores: cores(),
        cpu_avg_pct: w.cpu_sum / n,
        cpu_max_pct: w.cpu_max,
        system_cpu_pct: if w.sys_n > 0 { w.sys_sum / w.sys_n as f64 } else { -1.0 },
        threads,
        thread_count_max: w.thread_count_max,
        gpu_ms: w.gpu_ns as f64 / 1e6,
        gpu_avg_pct: if secs > 0.0 { w.gpu_ns as f64 / (secs * 1e9) * 100.0 } else { 0.0 },
        gpu_max_pct: w.gpu_max,
        energy_mw_avg: if secs > 0.0 { w.energy_nj as f64 / 1e6 / secs } else { 0.0 },
        energy_mw_max: w.energy_max_mw,
        wakeups_per_s: if secs > 0.0 { w.wakeups as f64 / secs } else { 0.0 },
        memory_mb_avg: w.mem_sum / (w.samples.max(1) as f64 + 1.0),
        memory_mb_max: w.mem_max,
        memory_available_mb_min: if w.avail_min == f64::MAX { -1.0 } else { w.avail_min },
        frames,
        screens,
        thermal_now: thermal_name(st.thermal).into(),
        thermal_max: thermal_name(w.thermal_max).into(),
        thermal_changes: w.thermal_changes,
        thermal_fair_s: w.thermal_secs[1],
        thermal_serious_s: w.thermal_secs[2],
        thermal_critical_s: w.thermal_secs[3],
        battery_pct: st.battery,
        battery_drain_pct_per_hour: drain,
        charging: st.charging,
        low_power: st.low_power,
        sampler_overhead_us: w.overhead_us_sum / (w.samples.max(1) as f64 + 1.0),
        interesting: false,
        reasons: Vec::new(),
    };
    judge(&mut out);
    out
}

/// Flag windows worth uploading, with why.
pub fn judge(w: &mut PerfWindow) {
    let mut r = Vec::new();
    let f = &w.frames;
    if f.frames >= 30 && f.hitch_ratio >= 5.0 {
        r.push(format!("hitch ratio {:.1} ms/s ({})", f.hitch_ratio, f.rating));
    }
    if f.frozen > 0 {
        r.push(format!("{} frozen frame(s), worst {:.0} ms", f.frozen, f.max_ms));
    } else if f.long_hitches >= 3 {
        r.push(format!("{} hitches ≥100 ms", f.long_hitches));
    }
    if w.cpu_avg_pct >= 60.0 {
        r.push(format!("CPU avg {:.0}%", w.cpu_avg_pct));
    } else if w.cpu_max_pct >= 180.0 {
        r.push(format!("CPU peak {:.0}%", w.cpu_max_pct));
    }
    if w.gpu_avg_pct >= 25.0 {
        r.push(format!("GPU avg {:.0}%", w.gpu_avg_pct));
    }
    if w.energy_mw_avg >= 1500.0 {
        r.push(format!("power {:.0} mW", w.energy_mw_avg));
    }
    if w.thermal_max == "serious" || w.thermal_max == "critical" {
        r.push(format!("thermal {}", w.thermal_max));
    }
    if w.memory_mb_max >= 900.0 {
        r.push(format!("memory {:.0} MB", w.memory_mb_max));
    }
    if w.memory_available_mb_min >= 0.0 && w.memory_available_mb_min < 150.0 {
        r.push(format!("only {:.0} MB left before jetsam", w.memory_available_mb_min));
    }
    if w.battery_drain_pct_per_hour >= 25.0 {
        r.push(format!("battery {:.0}%/h", w.battery_drain_pct_per_hour));
    }
    w.interesting = !r.is_empty();
    w.reasons = r;
}

/// JSON for the bridge event log.
#[uniffi::export]
pub fn perf_window_json(w: PerfWindow) -> String {
    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    serde_json::json!({
        "seconds": r1(w.seconds), "foregroundSeconds": r1(w.foreground_seconds), "cores": w.cores,
        "cpu": {"avgPct": r1(w.cpu_avg_pct), "maxPct": r1(w.cpu_max_pct), "systemPct": r1(w.system_cpu_pct), "threadsMax": w.thread_count_max,
                "threads": w.threads.iter().map(|t| serde_json::json!({"name": t.name, "pct": r1(t.cpu_pct), "ms": r1(t.cpu_ms)})).collect::<Vec<_>>()},
        "gpu": {"ms": r1(w.gpu_ms), "avgPct": r1(w.gpu_avg_pct), "maxPct": r1(w.gpu_max_pct)},
        "power": {"avgMw": r1(w.energy_mw_avg), "maxMw": r1(w.energy_mw_max), "wakeupsPerS": r1(w.wakeups_per_s),
                  "batteryPct": r1(w.battery_pct), "drainPctPerHour": r1(w.battery_drain_pct_per_hour), "charging": w.charging, "lowPower": w.low_power},
        "memory": {"avgMb": r1(w.memory_mb_avg), "maxMb": r1(w.memory_mb_max), "availableMinMb": r1(w.memory_available_mb_min)},
        "frames": {"count": w.frames.frames, "fps": r1(w.frames.fps), "budgetMs": r1(w.frames.expected_ms), "p50Ms": r1(w.frames.p50_ms), "p95Ms": r1(w.frames.p95_ms),
                   "p99Ms": r1(w.frames.p99_ms), "maxMs": r1(w.frames.max_ms), "dropped": w.frames.dropped, "hitches": w.frames.hitches,
                   "longHitches": w.frames.long_hitches, "frozen": w.frames.frozen, "hitchRatio": r1(w.frames.hitch_ratio), "rating": w.frames.rating},
        "screens": w.screens.iter().map(|s| serde_json::json!({"screen": s.screen, "seconds": r1(s.seconds), "hitchRatio": r1(s.hitch_ratio), "worstMs": r1(s.worst_frame_ms)})).collect::<Vec<_>>(),
        "thermal": {"now": w.thermal_now, "max": w.thermal_max, "changes": w.thermal_changes, "fairS": r1(w.thermal_fair_s), "seriousS": r1(w.thermal_serious_s), "criticalS": r1(w.thermal_critical_s)},
        "samplerOverheadUs": r1(w.sampler_overhead_us),
        "reasons": w.reasons,
    })
    .to_string()
}

/// One-line summary for logs.
#[uniffi::export]
pub fn perf_window_summary(w: PerfWindow) -> String {
    format!(
        "{:.0}s: CPU {:.0}% (peak {:.0}%), GPU {:.1}%, {:.0} mW, mem {:.0} MB, {:.0} fps hitch {:.1} ms/s ({}), thermal {}{}",
        w.seconds,
        w.cpu_avg_pct,
        w.cpu_max_pct,
        w.gpu_avg_pct,
        w.energy_mw_avg,
        w.memory_mb_max,
        w.frames.fps,
        w.frames.hitch_ratio,
        w.frames.rating,
        w.thermal_max,
        if w.reasons.is_empty() { String::new() } else { format!(" — {}", w.reasons.join("; ")) }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_stats_percentiles_and_hitches() {
        let mut f = Frames::default();
        for _ in 0..590 {
            f.add(16.7, 16.7);
        }
        for _ in 0..8 {
            f.add(33.4, 16.7);
        }
        f.add(120.0, 16.7);
        f.add(800.0, 16.7);
        let s = f.stats();
        assert_eq!(s.frames, 600);
        assert!((s.p50_ms - 17.0).abs() <= 0.5, "{}", s.p50_ms);
        assert!(s.p99_ms >= 33.0 && s.p99_ms <= 34.0, "{}", s.p99_ms);
        assert_eq!(s.max_ms, 800.0);
        assert_eq!(s.hitches, 10);
        assert_eq!(s.long_hitches, 2);
        assert_eq!(s.frozen, 1);
        assert_eq!(s.dropped, 8 + 6 + 47);
        assert!(s.hitch_ratio > 10.0 && s.rating == "critical", "{}", s.hitch_ratio);
    }

    #[test]
    fn sampler_measures_a_busy_thread() {
        perf_start();
        perf_set_state(true, "Test".into());
        perf_record_thermal(0);
        let _ = perf_take_window();
        // Burn one core on a named thread for ~2.5 s.
        let h = std::thread::Builder::new()
            .name("perf-burner".into())
            .spawn(|| {
                let t = Instant::now();
                let mut x = 0u64;
                while t.elapsed() < Duration::from_millis(2500) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                }
                x
            })
            .unwrap();
        perf_record_frames(vec![16.7; 120], 16.7);
        perf_record_thermal(2);
        h.join().unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let live = perf_live();
        assert!(live.memory_mb > 1.0 && live.thread_count >= 2);
        let w = perf_take_window();
        let burner = w.threads.iter().find(|t| t.name.starts_with("perf-burner")).map(|t| t.cpu_pct).unwrap_or(0.0);
        assert!(burner > 40.0, "burner thread {burner}% of {:?}", w.threads);
        assert!(w.cpu_max_pct > 50.0, "{}", w.cpu_max_pct);
        assert_eq!(w.thermal_max, "serious");
        assert!(w.interesting && w.reasons.iter().any(|r| r.contains("thermal")));
        assert_eq!(w.frames.frames, 120);
        assert_eq!(w.screens[0].screen, "Test");
        assert!(w.sampler_overhead_us < 20_000.0, "sampler too slow: {} µs", w.sampler_overhead_us);
        let j: serde_json::Value = serde_json::from_str(&perf_window_json(w)).unwrap();
        assert_eq!(j["thermal"]["max"], "serious");
        perf_record_thermal(0);
    }

    #[test]
    fn thread_groups() {
        assert_eq!(thread_group("tokio-runtime-worker"), "tokio-runtime-worker");
        assert_eq!(thread_group("com.apple.audio.IOThread.client"), "com.apple.audio.IOThread.client");
        assert_eq!(thread_group("lumisound-deck-12"), "lumisound-deck");
        assert_eq!(thread_group(""), "(unnamed)");
    }
}
