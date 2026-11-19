//! Thread stacks read from another thread (Apple arm64), replacing the app's
//! Swift `MainThreadBacktrace`:
//! - the main thread's stack while it is stuck (the hang monitor),
//! - the other threads a stuck main thread may be waiting on,
//! - a continuous sample of the main thread while it is running, folded into
//!   each perf window, so main-thread CPU (2.0.0.65: 23–28 % on an iPhone 13)
//!   can be placed in code,
//! - where this process really runs from (LiveContainer runs guest apps inside
//!   its own process, so `Bundle.main` alone can't tell).
//!
//! Rules while a thread is suspended: no allocation, no locks, no runtime
//! calls — the suspended thread may hold the malloc or dyld lock. Frames go
//! into a caller-owned fixed buffer, memory is read with `vm_read_overwrite`
//! (a Mach trap, safe on a bad pointer), and `dladdr` runs only after
//! `thread_resume`.

/// Frames kept per walk.
pub(crate) const MAX_FRAMES: usize = 64;

#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct HostProcess {
    /// Path of the process's main executable.
    pub executable: String,
    /// True when the executable isn't inside the app's own bundle (the app is
    /// a guest of another process, e.g. LiveContainer).
    pub hosted: bool,
}

/// Frames of the main thread as "Image 0xoffset symbol+delta", innermost first
/// (offset from the image's load address — what llvm-symbolizer needs with the
/// build's dSYM). Empty when it can't be read.
#[uniffi::export]
pub fn main_thread_stack() -> Vec<String> {
    let mut buf = [0u64; MAX_FRAMES];
    let n = imp::walk_main(&mut buf);
    buf[..n].iter().map(|a| describe(*a)).collect()
}

/// Stacks of the threads a stuck main thread may be waiting on: any thread in
/// the app's own code or in the drawing / media frameworks ("name: frame | …").
#[uniffi::export]
pub fn other_thread_stacks(limit: u32) -> Vec<String> {
    imp::other_stacks(limit as usize)
}

/// CPU time the main thread has used so far, in seconds (0 if unknown).
#[uniffi::export]
pub fn main_thread_cpu_seconds() -> f64 {
    imp::main_cpu_seconds()
}

#[uniffi::export]
pub fn host_process_info(bundle_path: String) -> HostProcess {
    let executable = imp::executable_path();
    HostProcess { hosted: is_hosted(&executable, &bundle_path), executable }
}

fn is_hosted(executable: &str, bundle_path: &str) -> bool {
    let bundle = bundle_path.trim_end_matches('/');
    !executable.is_empty() && !bundle.is_empty() && !executable.starts_with(&format!("{bundle}/"))
}

/// Strips pointer-authentication bits from a return address.
pub(crate) fn strip(a: u64) -> u64 {
    a & 0x0000_000f_ffff_ffff
}

/// "Lumisound 0x1a2b3c symbol+12".
pub(crate) fn describe(raw: u64) -> String {
    imp::describe(strip(raw))
}

pub(crate) use imp::{main_running, walk_main};

#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
mod imp {
    use super::MAX_FRAMES;
    use libc::{c_int, kern_return_t, mach_port_t};
    use std::ffi::CStr;

    const ARM_THREAD_STATE64: c_int = 6;
    const KERN_SUCCESS: kern_return_t = 0;

    #[repr(C)]
    #[derive(Default)]
    struct ArmThreadState64 {
        x: [u64; 29],
        fp: u64,
        lr: u64,
        sp: u64,
        pc: u64,
        cpsr: u32,
        pad: u32,
    }

    unsafe extern "C" {
        fn thread_suspend(thread: mach_port_t) -> kern_return_t;
        fn thread_resume(thread: mach_port_t) -> kern_return_t;
        fn thread_get_state(thread: mach_port_t, flavor: c_int, state: *mut u32, count: *mut u32) -> kern_return_t;
        fn vm_read_overwrite(task: mach_port_t, address: u64, size: u64, data: u64, out_size: *mut u64) -> kern_return_t;
        fn pthread_main_thread_np() -> libc::pthread_t;
        fn mach_port_deallocate(task: mach_port_t, name: mach_port_t) -> kern_return_t;
    }

    fn task() -> mach_port_t {
        unsafe { libc::mach_task_self() }
    }

    /// The main thread's port (no extra reference is taken).
    fn main_port() -> mach_port_t {
        unsafe { libc::pthread_mach_thread_np(pthread_main_thread_np()) }
    }

    fn self_port() -> mach_port_t {
        unsafe { libc::pthread_mach_thread_np(libc::pthread_self()) }
    }

    fn basic_info(thread: mach_port_t) -> Option<libc::thread_basic_info> {
        let mut info: libc::thread_basic_info = unsafe { std::mem::zeroed() };
        let mut n = (std::mem::size_of::<libc::thread_basic_info>() / 4) as libc::mach_msg_type_number_t;
        let kr = unsafe { libc::thread_info(thread, libc::THREAD_BASIC_INFO as _, &mut info as *mut _ as libc::thread_info_t, &mut n) };
        (kr == KERN_SUCCESS).then_some(info)
    }

    /// The main thread is on a CPU right now (not parked in the run loop).
    pub(crate) fn main_running() -> bool {
        basic_info(main_port()).is_some_and(|i| i.run_state == libc::TH_STATE_RUNNING as i32)
    }

    pub(crate) fn main_cpu_seconds() -> f64 {
        basic_info(main_port()).map_or(0.0, |i| {
            i.user_time.seconds as f64 + i.user_time.microseconds as f64 / 1e6 + i.system_time.seconds as f64 + i.system_time.microseconds as f64 / 1e6
        })
    }

    pub(crate) fn walk_main(buf: &mut [u64; MAX_FRAMES]) -> usize {
        let main = main_port();
        if main == self_port() {
            return 0;
        }
        walk(main, buf)
    }

    /// Suspends `thread`, walks its frame-pointer chain into `buf`, resumes it.
    /// Nothing between suspend and resume allocates or locks.
    fn walk(thread: mach_port_t, buf: &mut [u64; MAX_FRAMES]) -> usize {
        if unsafe { thread_suspend(thread) } != KERN_SUCCESS {
            return 0;
        }
        let mut count = 0usize;
        let mut st = ArmThreadState64::default();
        let mut n = (std::mem::size_of::<ArmThreadState64>() / 4) as u32;
        if unsafe { thread_get_state(thread, ARM_THREAD_STATE64, &mut st as *mut _ as *mut u32, &mut n) } == KERN_SUCCESS {
            buf[0] = st.pc;
            buf[1] = st.lr;
            count = 2;
            let mut fp = st.fp;
            // Each frame record is { previous fp, return address }.
            let mut rec = [0u64; 2];
            while fp != 0 && fp & 0x7 == 0 && count < MAX_FRAMES {
                let mut got = 0u64;
                let kr = unsafe { vm_read_overwrite(task(), fp, 16, rec.as_mut_ptr() as u64, &mut got) };
                if kr != KERN_SUCCESS || got != 16 || rec[1] == 0 {
                    break;
                }
                buf[count] = rec[1];
                count += 1;
                // The chain only moves up the stack; anything else is garbage.
                if rec[0] <= fp {
                    break;
                }
                fp = rec[0];
            }
        }
        unsafe { thread_resume(thread) };
        count
    }

    pub(crate) fn describe(address: u64) -> String {
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        if unsafe { libc::dladdr(address as *const libc::c_void, &mut info) } == 0 {
            return format!("? 0x{address:x}");
        }
        let image = if info.dli_fname.is_null() {
            "?".to_string()
        } else {
            let p = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy();
            p.rsplit('/').next().unwrap_or(&p).to_string()
        };
        let offset = address.wrapping_sub(info.dli_fbase as u64);
        if info.dli_sname.is_null() {
            return format!("{image} 0x{offset:x}");
        }
        let sym: String = unsafe { CStr::from_ptr(info.dli_sname) }.to_string_lossy().chars().take(120).collect();
        format!("{image} 0x{offset:x} {sym}+{}", address.wrapping_sub(info.dli_saddr as u64))
    }

    const INTERESTING: [&str; 11] = [
        "Lumisound ", "QuartzCore ", "UIKitCore ", "SwiftUI ", "AVFCore ", "AVFoundation ", "CoreMedia ", "MediaToolbox ", "CoreUI ", "ImageIO ",
        "AttributeGraph ",
    ];

    pub(crate) fn other_stacks(limit: usize) -> Vec<String> {
        let mut list: libc::thread_act_array_t = std::ptr::null_mut();
        let mut total: libc::mach_msg_type_number_t = 0;
        if unsafe { libc::task_threads(task(), &mut list, &mut total) } != KERN_SUCCESS || list.is_null() {
            return Vec::new();
        }
        let ports: Vec<mach_port_t> = (0..total as usize).map(|i| unsafe { *list.add(i) }).collect();
        unsafe { libc::vm_deallocate(task(), list as libc::vm_address_t, total as usize * std::mem::size_of::<libc::thread_act_t>()) };
        let (me, main) = (self_port(), main_port());
        let mut out = Vec::new();
        let mut buf = [0u64; MAX_FRAMES];
        for (i, &th) in ports.iter().enumerate() {
            if out.len() < limit && th != me && th != main {
                let n = walk(th, &mut buf);
                if n > 2 {
                    // Everything below runs with the thread resumed.
                    let frames: Vec<String> = buf[..n.min(28)].iter().map(|a| describe(super::strip(*a))).collect();
                    if frames.iter().any(|f| INTERESTING.iter().any(|p| f.starts_with(p))) {
                        let mut name = [0 as libc::c_char; 64];
                        let p = unsafe { libc::pthread_from_mach_thread_np(th) };
                        if p != 0 as libc::pthread_t {
                            unsafe { libc::pthread_getname_np(p, name.as_mut_ptr(), name.len()) };
                        }
                        let label = unsafe { CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned();
                        let label = if label.is_empty() { format!("thread {i}") } else { label };
                        out.push(format!("{label}: {}", frames.join(" | ")));
                    }
                }
            }
            unsafe { mach_port_deallocate(task(), th) };
        }
        out
    }

    pub(crate) fn executable_path() -> String {
        let p = unsafe { libc::_dyld_get_image_name(0) };
        if p.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    }
}

#[cfg(not(all(target_vendor = "apple", target_arch = "aarch64")))]
mod imp {
    use super::MAX_FRAMES;
    pub(crate) fn main_running() -> bool {
        false
    }
    pub(crate) fn main_cpu_seconds() -> f64 {
        0.0
    }
    pub(crate) fn walk_main(_buf: &mut [u64; MAX_FRAMES]) -> usize {
        0
    }
    pub(crate) fn describe(address: u64) -> String {
        format!("? 0x{address:x}")
    }
    pub(crate) fn other_stacks(_limit: usize) -> Vec<String> {
        Vec::new()
    }
    pub(crate) fn executable_path() -> String {
        std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_detection() {
        let b = "/private/var/containers/Bundle/Application/X/Lumisound.app";
        assert!(!is_hosted(&format!("{b}/Lumisound"), b));
        assert!(!is_hosted(&format!("{b}/Lumisound"), &format!("{b}/")));
        // LiveContainer: the process is LiveContainer's, the guest bundle lives in its Documents.
        assert!(is_hosted("/private/var/containers/Bundle/Application/Y/LiveContainer.app/LiveContainer", "/var/mobile/Containers/Data/Application/Z/Documents/Applications/Lumisound.app"));
        assert!(!is_hosted("", b));
        assert_eq!(strip(0x8012_3456_0001_2345), 0x0000_0006_0001_2345);
    }
}
