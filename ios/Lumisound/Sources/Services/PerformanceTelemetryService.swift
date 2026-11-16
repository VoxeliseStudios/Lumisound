import Foundation
import QuartzCore
import UIKit

// MARK: - PerformanceTelemetryService
//
// In-depth, always-on performance telemetry. The measuring and judging is in
// the Rust core (rust/core/src/perf.rs): a sampler thread reads per-thread CPU
// (with thread names), physical memory footprint and the headroom left before
// the OS kills the app, the GPU time this process submits, its energy use
// (mW, the heat proxy — iOS exposes no temperature reading) and wakeups,
// and folds everything into one-minute windows with frame percentiles, hitch
// time ratio, dropped/frozen frames per screen, thermal time-in-state and
// battery drain.
//
// This file feeds the core what only UIKit knows and ships the results:
// * frames — a CADisplayLink (60 Hz cap on iPhone: Info.plist keeps
//   CADisableMinimumFrameDurationOnPhone off) records each frame interval;
//   batched to the core once a second, paused in the background;
// * thermal state, battery level/charging, Low Power Mode, foreground state
//   and the screen showing;
// * every minute the window is taken: uploaded as `perf_window` (warning)
//   when the core flags it (hitches, frozen frames, CPU/GPU/power, thermal,
//   memory pressure, battery drain), else a `perf_heartbeat` (info) every
//   15 min in the foreground; `perf_thermal` right away when the device
//   reaches serious/critical.
//
// Overhead: one closure per frame appending a Float, one FFI call a second,
// one a minute; the core's sampler costs ~40 µs per second.
@MainActor
final class PerformanceTelemetryService: ObservableObject {
    static let shared = PerformanceTelemetryService()

    @Published private(set) var lastWindow: PerfWindow?
    @Published private(set) var lastWindowSummary: String?

    private var frameProbe: FrameProbe?
    private var reportTimer: Timer?
    private var lastHeartbeat = Date()
    private var screen = "Library"
    private var started = false
    private var lastThermal = ProcessInfo.processInfo.thermalState

    private init() {}

    func start() {
        guard !started else { return }
        started = true
        perfStart()
        let probe = FrameProbe()
        probe.start()
        frameProbe = probe
        UIDevice.current.isBatteryMonitoringEnabled = true
        pushThermal()
        pushPower()
        perfSetState(foreground: UIApplication.shared.applicationState != .background, screen: screen)

        let nc = NotificationCenter.default
        nc.addObserver(forName: ProcessInfo.thermalStateDidChangeNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { PerformanceTelemetryService.shared.thermalChanged() }
        }
        for name in [UIDevice.batteryLevelDidChangeNotification, UIDevice.batteryStateDidChangeNotification, Notification.Name.NSProcessInfoPowerStateDidChange] {
            nc.addObserver(forName: name, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { PerformanceTelemetryService.shared.pushPower() }
            }
        }
        nc.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { PerformanceTelemetryService.shared.setForeground(false) }
        }
        nc.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { PerformanceTelemetryService.shared.setForeground(true) }
        }

        let t = Timer(timeInterval: 60, repeats: true) { _ in
            MainActor.assumeIsolated { PerformanceTelemetryService.shared.report() }
        }
        t.tolerance = 5
        RunLoop.main.add(t, forMode: .common)
        reportTimer = t
    }

    /// The screen frames and hitches are attributed to.
    func setScreen(_ name: String) {
        guard name != screen else { return }
        frameProbe?.flush()
        screen = name
        perfSetState(foreground: UIApplication.shared.applicationState != .background, screen: name)
    }

    private func setForeground(_ fg: Bool) {
        frameProbe?.setPaused(!fg)
        perfSetState(foreground: fg, screen: screen)
        if !fg { report() }
    }

    private func pushThermal() {
        perfRecordThermal(raw: Int64(ProcessInfo.processInfo.thermalState.rawValue))
    }

    private func pushPower() {
        let d = UIDevice.current
        perfRecordPower(batteryLevel: d.batteryLevel,
                        charging: d.batteryState == .charging || d.batteryState == .full,
                        lowPower: ProcessInfo.processInfo.isLowPowerModeEnabled)
    }

    private func thermalChanged() {
        pushThermal()
        let now = ProcessInfo.processInfo.thermalState
        defer { lastThermal = now }
        guard now.rawValue >= ProcessInfo.ThermalState.serious.rawValue, now.rawValue > lastThermal.rawValue else { return }
        let live = perfLive()
        var detail = Self.liveDetail(live)
        detail["from"] = lastThermal.lumisoundDescription
        detail["to"] = now.lumisoundDescription
        detail["context"] = context()
        RemoteLogger.log(category: "performance", event: "perf_thermal", level: "warning",
                         message: String(format: "thermal %@ → %@ (CPU %.0f%%, GPU %.0f%%, %.0f mW)",
                                         lastThermal.lumisoundDescription, now.lumisoundDescription, live.cpuPct, live.gpuPct, live.energyMw),
                         detail: detail)
    }

    /// Take the window and upload it when it matters (or as a heartbeat).
    private func report() {
        frameProbe?.flush()
        let w = perfTakeWindow()
        guard w.seconds >= 5 else { return }
        lastWindow = w
        let summary = perfWindowSummary(w: w)
        lastWindowSummary = summary
        let heartbeatDue = w.foregroundSeconds > 0 && Date().timeIntervalSince(lastHeartbeat) >= 15 * 60
        guard w.interesting || heartbeatDue else { return }
        var detail = (perfWindowJson(w: w).data(using: .utf8))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
        detail["context"] = context()
        detail["engine"] = AudioPlayerManager.engineHealthSnapshot()
        if heartbeatDue { lastHeartbeat = Date() }
        RemoteLogger.log(category: "performance", event: w.interesting ? "perf_window" : "perf_heartbeat",
                         level: w.interesting ? "warning" : "info", message: summary, detail: detail)
        if w.interesting { appWarn("Performance: \(summary)", category: "performance") }
    }

    private func context() -> [String: Any] {
        let player = AudioPlayerManager.shared
        return [
            "screen": screen,
            "isPlaying": player?.isPlaying ?? false,
            "background": UIApplication.shared.applicationState == .background,
            "lowPower": ProcessInfo.processInfo.isLowPowerModeEnabled,
            "reduceMotion": UIAccessibility.isReduceMotionEnabled,
            "physicalMemoryMb": Int(ProcessInfo.processInfo.physicalMemory / 1_048_576),
            "uptimeMin": Int(ProcessInfo.processInfo.systemUptime / 60),
        ]
    }

    static func liveDetail(_ l: PerfLive) -> [String: Any] {
        [
            "cpuPct": l.cpuPct.rounded(), "systemCpuPct": l.systemCpuPct.rounded(), "gpuPct": (l.gpuPct * 10).rounded() / 10,
            "energyMw": l.energyMw.rounded(), "memoryMb": l.memoryMb.rounded(), "memoryAvailableMb": l.memoryAvailableMb.rounded(),
            "fps": l.fps.rounded(), "hitchRatio": (l.hitchRatio * 10).rounded() / 10, "threads": l.threadCount,
            "topThreads": l.threads.prefix(5).map { ["name": $0.name, "pct": $0.cpuPct.rounded()] },
        ]
    }
}

// MARK: - FrameProbe

/// Records display-link frame intervals and hands them to the core in
/// batches. A late callback means the main thread missed a frame: exactly
/// what the user sees as a hitch.
private final class FrameProbe: NSObject {
    private var link: CADisplayLink?
    private var last: CFTimeInterval = 0
    private var budgetMs: Float = 1000 / 60
    private var buffer: [Float] = []
    private var lastFlush: CFTimeInterval = 0

    func start() {
        buffer.reserveCapacity(128)
        let l = CADisplayLink(target: self, selector: #selector(tick(_:)))
        l.add(to: .main, forMode: .common)
        link = l
    }

    func setPaused(_ paused: Bool) {
        if paused { flush() }
        link?.isPaused = paused
        last = 0
    }

    @objc private func tick(_ l: CADisplayLink) {
        let now = l.timestamp
        let frame = Float((l.targetTimestamp - l.timestamp) * 1000)
        if frame > 1 { budgetMs = frame }
        if last > 0 {
            buffer.append(Float((now - last) * 1000))
        }
        last = now
        if now - lastFlush >= 1 { flush(at: now) }
    }

    func flush(at now: CFTimeInterval = CACurrentMediaTime()) {
        lastFlush = now
        guard !buffer.isEmpty else { return }
        perfRecordFrames(intervalsMs: buffer, expectedMs: budgetMs)
        buffer.removeAll(keepingCapacity: true)
    }
}
