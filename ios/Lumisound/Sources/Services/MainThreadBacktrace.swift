import Darwin
import Foundation

/// The main thread's call stack, sampled from another thread while the main
/// thread is stuck.
///
/// `MainThreadHangMonitor` could say that the main thread stalled and for how
/// long, never where: `Thread.callStackSymbols` only describes the calling
/// thread. This suspends the main thread for a few microseconds, reads its
/// registers and walks the frame-pointer chain, then resumes it.
///
/// Rules while the main thread is suspended: no allocation, no locks, no
/// Objective-C or Swift runtime calls — the main thread may hold the malloc or
/// dyld lock, and waiting on it here would deadlock the whole app. The frame
/// buffer is allocated up front, memory is read with `vm_read_overwrite` (a
/// Mach trap, and safe on a bad pointer), and symbol lookup (`dladdr`) happens
/// only after `thread_resume`.
enum MainThreadBacktrace {
    private static var mainThread: thread_t = 0
    private static let capacity = 64
    private static let frames = UnsafeMutablePointer<UInt>.allocate(capacity: capacity)

    /// Call once, on the main thread.
    static func registerMainThread() {
        if mainThread == 0 { mainThread = mach_thread_self() }
    }

    /// CPU time the main thread has used so far, in seconds (0 if unknown).
    ///
    /// Read before and after a stall, it says what kind of stall it was: a
    /// main thread that burned about as much CPU as the stall lasted was
    /// computing; one that used almost none was waiting on something (a
    /// lock, a synchronous call, disk).
    static func mainThreadCPUSeconds() -> Double {
        let thread = mainThread
        guard thread != 0 else { return 0 }
        var info = thread_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<thread_basic_info>.size / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                thread_info(thread, thread_flavor_t(THREAD_BASIC_INFO), $0, &count)
            }
        }
        guard result == KERN_SUCCESS else { return 0 }
        return Double(info.user_time.seconds) + Double(info.user_time.microseconds) / 1_000_000
            + Double(info.system_time.seconds) + Double(info.system_time.microseconds) / 1_000_000
    }

    /// Frames as "Image 0xoffset symbol", innermost first. Empty when the
    /// stack can't be read (not registered, or not an arm64 device).
    static func capture() -> [String] {
        #if arch(arm64)
        let thread = mainThread
        guard thread != 0, thread != mach_thread_self() else { return [] }
        let count = walk(thread)
        return (0..<count).map { describe(frames[$0]) }
        #else
        return []
        #endif
    }

    /// The other threads' stacks, for the ones that could be what the main
    /// thread is waiting on: any thread inside the app's own code or inside
    /// the drawing / media frameworks. Idle pool threads and run loops parked
    /// in the kernel are left out. Each entry is "name: frame | frame | …".
    ///
    /// A main thread stuck in Core Animation's commit is waiting for someone
    /// else — another thread in the middle of its own commit, or one holding
    /// a lock the commit needs. The main thread's stack alone cannot say who.
    static func captureOthers(limit: Int = 10) -> [String] {
        #if arch(arm64)
        var list: thread_act_array_t?
        var total: mach_msg_type_number_t = 0
        guard task_threads(mach_task_self_, &list, &total) == KERN_SUCCESS, let list else { return [] }
        defer {
            vm_deallocate(mach_task_self_, vm_address_t(UInt(bitPattern: list)),
                          vm_size_t(Int(total) * MemoryLayout<thread_act_t>.size))
        }
        let me = mach_thread_self()
        var out: [String] = []
        for i in 0..<Int(total) {
            let thread = list[i]
            if thread == me || thread == mainThread { continue }
            let count = walk(thread)
            guard count > 2 else { continue }
            // Everything below runs with the thread resumed.
            let described = (0..<min(count, 28)).map { describe(frames[$0]) }
            let interesting = described.contains { f in
                f.hasPrefix("Lumisound ") || f.hasPrefix("QuartzCore ") || f.hasPrefix("UIKitCore ")
                    || f.hasPrefix("SwiftUI ") || f.hasPrefix("AVFCore ") || f.hasPrefix("AVFoundation ")
                    || f.hasPrefix("CoreMedia ") || f.hasPrefix("MediaToolbox ") || f.hasPrefix("CoreUI ")
                    || f.hasPrefix("ImageIO ") || f.hasPrefix("AttributeGraph ")
            }
            guard interesting else { continue }
            var name = [CChar](repeating: 0, count: 64)
            if let p = pthread_from_mach_thread_np(thread) { pthread_getname_np(p, &name, name.count) }
            let label = String(cString: name)
            out.append((label.isEmpty ? "thread \(i)" : label) + ": " + described.joined(separator: " | "))
            if out.count >= limit { break }
        }
        return out
        #else
        return []
        #endif
    }

    #if arch(arm64)
    /// Suspends `thread`, walks its frame-pointer chain into `frames`, resumes
    /// it. Returns the number of frames. See the rules at the top of the file.
    private static func walk(_ thread: thread_t) -> Int {
        var count = 0
        guard thread_suspend(thread) == KERN_SUCCESS else { return 0 }
        var state = arm_thread_state64_t()
        var stateCount = mach_msg_type_number_t(MemoryLayout<arm_thread_state64_t>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &state) { ptr in
            ptr.withMemoryRebound(to: natural_t.self, capacity: Int(stateCount)) {
                thread_get_state(thread, ARM_THREAD_STATE64, $0, &stateCount)
            }
        }
        if kr == KERN_SUCCESS {
            frames[0] = UInt(state.__pc)
            frames[1] = UInt(state.__lr)
            count = 2
            var fp = vm_address_t(state.__fp)
            // Each frame record is { previous fp, return address }.
            var record: (UInt, UInt) = (0, 0)
            while fp != 0, fp & 0x7 == 0, count < capacity {
                var read: vm_size_t = 0
                let ok = withUnsafeMutablePointer(to: &record) { rec in
                    vm_read_overwrite(mach_task_self_, fp, vm_size_t(MemoryLayout<(UInt, UInt)>.size),
                                      vm_address_t(UInt(bitPattern: rec)), &read)
                }
                guard ok == KERN_SUCCESS, read == vm_size_t(MemoryLayout<(UInt, UInt)>.size), record.1 != 0 else { break }
                frames[count] = record.1
                count += 1
                // The chain only ever moves up the stack; anything else is garbage.
                guard record.0 > UInt(fp) else { break }
                fp = vm_address_t(record.0)
            }
        }
        thread_resume(thread)
        return count
    }
    #endif

    /// "Lumisound 0x1a2b3c symbol+12" (offset from the image's load address,
    /// which is what a crash symbolicator needs with the build's dSYM).
    private static func describe(_ raw: UInt) -> String {
        // Strip pointer-authentication bits from return addresses.
        let address = raw & 0x0000_000f_ffff_ffff
        var info = Dl_info()
        guard dladdr(UnsafeRawPointer(bitPattern: address), &info) != 0 else {
            return String(format: "? 0x%lx", address)
        }
        let image = info.dli_fname.map { (String(cString: $0) as NSString).lastPathComponent } ?? "?"
        let offset = address - UInt(bitPattern: info.dli_fbase)
        guard let sname = info.dli_sname else { return String(format: "%@ 0x%lx", image, offset) }
        let symbol = String(String(cString: sname).prefix(120))
        let delta = address - UInt(bitPattern: info.dli_saddr)
        return String(format: "%@ 0x%lx %@+%lu", image, offset, symbol, delta)
    }
}
