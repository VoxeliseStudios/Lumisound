import Darwin
import Foundation
import MetricKit

// MARK: - PerformanceMonitorService

/// Always-on CPU/GPU resource-usage logging — no settings toggle, starts
/// automatically at launch. Every `AppLogger` entry already auto-attaches
/// `DeviceInfo.modelIdentifier`/`.osVersion` (see `AppLogger.LogEntry`), so
/// logging through `appLog(..., category: "performance")` gets "categorized
/// by device and iOS number" for free — no extra plumbing needed here.
///
/// Two independent sources, since no single public iOS API covers both CPU
/// and GPU in real time:
///  1. Live sampling moved to the Rust core (`PerformanceTelemetryService`,
///     rust/core/src/perf.rs): per-thread CPU, GPU time, power, memory, frames.
///  2. `MetricSubscriber` — Apple's own MetricKit daily aggregate, which is
///     the ONLY public API surface exposing GPU usage on iOS at all; there
///     is no real-time per-frame GPU-utilization query available to apps.
enum PerformanceMonitorService {
    /// Call once from `LumisoundApp.init()`, alongside the other `.register()`
    /// calls — must be set up before/at launch so MetricKit doesn't miss the
    /// registration window and so CPU sampling covers the whole session.
    static func start() {
        // Live CPU/GPU/memory/frames now come from the Rust sampler
        // (PerformanceTelemetryService); the 60 s system-load logger it
        // replaced is gone.
        MXMetricManager.shared.add(MetricSubscriber.shared)
    }
}

// Internal, not fileprivate: `MainThreadHangMonitor` reports the thermal state
// alongside each stall too, and a second copy of this switch would be one more
// place for the strings to drift apart — they are compared across events.
extension ProcessInfo.ThermalState {
    var lumisoundDescription: String {
        switch self {
        case .nominal: return "nominal"
        case .fair: return "fair"
        case .serious: return "serious"
        case .critical: return "critical"
        @unknown default: return "unknown"
        }
    }
}

// MARK: - MetricSubscriber (Apple's own daily CPU/GPU aggregate)

/// Receives `MXMetricPayload`s — delivered by the OS roughly once per day,
/// each summarizing the previous ~24h window's cumulative CPU time, GPU
/// time, and thermal-state exposure for this app specifically. This is
/// Apple's intended mechanism for exactly this kind of field performance
/// monitoring; there's no faster or more granular public API for GPU usage
/// on iOS, so a daily rollup (rather than a live percentage) is the honest
/// ceiling of what's queryable here.
private final class MetricSubscriber: NSObject, MXMetricManagerSubscriber {
    static let shared = MetricSubscriber()

    func didReceive(_ payloads: [MXMetricPayload]) {
        for payload in payloads {
            var extra: [String: String] = [
                "periodStart": Self.iso(payload.timeStampBegin),
                "periodEnd": Self.iso(payload.timeStampEnd),
            ]
            if let cpu = payload.cpuMetrics?.cumulativeCPUTime {
                extra["cumulativeCPUTimeSeconds"] = String(format: "%.1f", cpu.converted(to: .seconds).value)
            }
            if let gpu = payload.gpuMetrics?.cumulativeGPUTime {
                extra["cumulativeGPUTimeSeconds"] = String(format: "%.1f", gpu.converted(to: .seconds).value)
            }
            appLog("MetricKit daily CPU/GPU report", category: "performance", extra: extra)
            reportDailyMetrics(payload)

            // WHY the app stopped running, which nothing here reported before.
            //
            // Field logs show real unclean shutdowns — the app's own breadcrumb
            // trail records 15 on the current build across 6 accounts, always
            // ending mid-playback or just after opening the Library — but the
            // breadcrumbs only say what happened BEFORE, never the reason. That
            // left "a few crashes" unanswerable: memory peaks top out near 400MB
            // with no correlation to which accounts die most (one account peaked
            // at 396MB with a single shutdown, another at 362MB with dozens), so
            // guessing from resource usage was not going to settle it either.
            //
            // `applicationExitMetrics` is the authoritative answer and costs
            // nothing extra: the OS already computes it and it distinguishes the
            // possibilities that need completely different fixes — a memory kill,
            // a watchdog timeout, a bad memory access, a CPU limit while
            // backgrounded, or an ordinary exit that was never a crash at all.
            reportExitMetrics(payload)
        }
    }

    /// Apple's own daily view of the same things the live sampler measures —
    /// the reference to check it against: CPU/GPU time, scroll hitch rate,
    /// peak memory, launch and resume times, hang time, disk writes, display.
    private func reportDailyMetrics(_ p: MXMetricPayload) {
        var d: [String: Any] = ["periodStart": Self.iso(p.timeStampBegin), "periodEnd": Self.iso(p.timeStampEnd), "appVersion": p.latestApplicationVersion]
        if let v = p.cpuMetrics?.cumulativeCPUTime { d["cpuSeconds"] = v.converted(to: .seconds).value }
        if let v = p.cpuMetrics?.cumulativeCPUInstructions { d["cpuInstructions"] = v.value }
        if let v = p.gpuMetrics?.cumulativeGPUTime { d["gpuSeconds"] = v.converted(to: .seconds).value }
        if let v = p.animationMetrics?.scrollHitchTimeRatio { d["scrollHitchMsPerS"] = v.value }
        if let v = p.memoryMetrics?.peakMemoryUsage { d["peakMemoryMb"] = v.converted(to: .megabytes).value }
        if let v = p.memoryMetrics?.averageSuspendedMemory.averageMeasurement { d["suspendedMemoryMb"] = v.converted(to: .megabytes).value }
        if let v = p.diskIOMetrics?.cumulativeLogicalWrites { d["diskWritesMb"] = v.converted(to: .megabytes).value }
        if let v = p.displayMetrics?.averagePixelLuminance?.averageMeasurement { d["avgPixelLuminanceApl"] = v.value }
        if let v = p.cellularConditionMetrics?.histogrammedCellularConditionTime { d["cellularBuckets"] = v.totalBucketCount }
        if let h = p.applicationResponsivenessMetrics?.histogrammedApplicationHangTime { d["hangBuckets"] = Self.histogram(h) }
        if let h = p.applicationLaunchMetrics?.histogrammedTimeToFirstDraw { d["launchFirstDraw"] = Self.histogram(h) }
        if let h = p.applicationLaunchMetrics?.histogrammedApplicationResumeTime { d["resume"] = Self.histogram(h) }
        if let t = p.applicationTimeMetrics {
            d["foregroundMin"] = t.cumulativeForegroundTime.converted(to: .minutes).value
            d["backgroundAudioMin"] = t.cumulativeBackgroundAudioTime.converted(to: .minutes).value
        }
        RemoteLogger.log(category: "performance", event: "metrickit_daily", level: "info",
                         message: "MetricKit daily performance", detail: d)
    }

    /// "<start>-<end> <unit>: count" for each non-empty bucket.
    private static func histogram<U: Unit>(_ h: MXHistogram<U>) -> [String] {
        var out: [String] = []
        let e = h.bucketEnumerator
        while let b = e.nextObject() as? MXHistogramBucket<U> {
            guard b.bucketCount > 0 else { continue }
            out.append("\(Int(b.bucketStart.value))-\(Int(b.bucketEnd.value)) \(b.bucketStart.unit.symbol): \(b.bucketCount)")
        }
        return out
    }

    /// Reports the OS's own breakdown of how this app's recent sessions ended.
    ///
    /// Foreground and background are kept apart deliberately: a background memory
    /// kill is routine for an audio app and means one thing, while the same kill in
    /// the foreground means the app is genuinely using too much. Zero counts are
    /// dropped so the report carries only what actually happened.
    private func reportExitMetrics(_ payload: MXMetricPayload) {
        guard let exits = payload.applicationExitMetrics else { return }

        var detail: [String: Any] = [
            "periodStart": Self.iso(payload.timeStampBegin),
            "periodEnd": Self.iso(payload.timeStampEnd),
        ]
        func add(_ prefix: String, _ counts: [(String, Int)]) {
            for (name, value) in counts where value > 0 {
                detail["\(prefix)_\(name)"] = value
            }
        }
        let fg = exits.foregroundExitData
        add("fg", [
            ("normal", fg.cumulativeNormalAppExitCount),
            ("memoryLimit", fg.cumulativeMemoryResourceLimitExitCount),
            ("badAccess", fg.cumulativeBadAccessExitCount),
            ("abnormal", fg.cumulativeAbnormalExitCount),
            ("illegalInstruction", fg.cumulativeIllegalInstructionExitCount),
            ("watchdog", fg.cumulativeAppWatchdogExitCount),
        ])
        let bg = exits.backgroundExitData
        add("bg", [
            ("normal", bg.cumulativeNormalAppExitCount),
            ("memoryLimit", bg.cumulativeMemoryResourceLimitExitCount),
            ("cpuLimit", bg.cumulativeCPUResourceLimitExitCount),
            ("badAccess", bg.cumulativeBadAccessExitCount),
            ("abnormal", bg.cumulativeAbnormalExitCount),
            ("illegalInstruction", bg.cumulativeIllegalInstructionExitCount),
            ("watchdog", bg.cumulativeAppWatchdogExitCount),
            ("suspendedWithLockedFile", bg.cumulativeSuspendedWithLockedFileExitCount),
            ("backgroundTaskTimeout", bg.cumulativeBackgroundTaskAssertionTimeoutExitCount),
        ])

        // An abnormal count of zero everywhere is the good case and still worth
        // recording: it is what distinguishes "no crashes happened" from "nothing
        // reported", which the unclean-shutdown breadcrumbs cannot tell apart.
        let crashy = detail.keys.contains { key in
            key.hasSuffix("memoryLimit") || key.hasSuffix("badAccess") || key.hasSuffix("abnormal")
                || key.hasSuffix("illegalInstruction") || key.hasSuffix("watchdog")
                || key.hasSuffix("cpuLimit") || key.hasSuffix("backgroundTaskTimeout")
        }
        RemoteLogger.log(
            category: "diagnostics", event: "app_exit_metrics",
            level: crashy ? "warning" : "info",
            message: crashy ? "abnormal exits reported" : "no abnormal exits",
            detail: detail
        )
    }

    // MARK: Crash / hang diagnostics

    /// Delivered by the OS after a crash, an unresponsive stretch, or a
    /// disk-write exception — usually on the next launch.
    ///
    /// This callback simply was not implemented, which is why crashes had no
    /// recorded cause at all. `MXMetricPayload` above covers CPU and GPU; the
    /// diagnostic payload is the separate one carrying the exception type, the
    /// signal, the OS's termination reason, and the call stack that produced it.
    /// Without it the only evidence of a crash was the app noticing, on its next
    /// launch, that the previous session had not shut down cleanly — which says
    /// nothing whatsoever about why.
    func didReceive(_ payloads: [MXDiagnosticPayload]) {
        for payload in payloads {
            for crash in payload.crashDiagnostics ?? [] {
                var detail: [String: Any] = [
                    "appVersion": crash.metaData.applicationBuildVersion,
                    "osVersion": crash.metaData.osVersion,
                    "deviceType": crash.metaData.deviceType,
                ]
                if let type = crash.exceptionType { detail["exceptionType"] = type.intValue }
                if let code = crash.exceptionCode { detail["exceptionCode"] = code.intValue }
                if let signal = crash.signal { detail["signal"] = signal.intValue }
                if let reason = crash.terminationReason { detail["terminationReason"] = reason }
                if let vmRegion = crash.virtualMemoryRegionInfo { detail["vmRegion"] = String(vmRegion.prefix(200)) }
                // The full tree is large and mostly addresses; the leading frames
                // are what identify a crash site, and keeping this bounded means a
                // crash report can never be too big to actually get uploaded.
                if let json = try? crash.callStackTree.jsonRepresentation() {
                    if let frames = Self.heaviestPathFrames(json) {
                        detail["leafFrames"] = frames
                    }
                    if let text = String(data: json, encoding: .utf8) {
                        detail["callStackPrefix"] = String(text.prefix(1800))
                    }
                }
                RemoteLogger.logError(
                    category: "diagnostics", event: "crash_diagnostic",
                    message: crash.terminationReason ?? "crash reported by MetricKit",
                    detail: detail
                )
            }

            // A hang is not a crash, but it is what the OS kills an app FOR — a
            // watchdog exit in the metrics above and a hang here are two views of
            // the same event, so both are needed to tell them apart.
            for hang in payload.hangDiagnostics ?? [] {
                var detail: [String: Any] = [
                    "hangSeconds": hang.hangDuration.converted(to: .seconds).value,
                    "appVersion": hang.metaData.applicationBuildVersion,
                    "osVersion": hang.metaData.osVersion,
                ]
                if let json = try? hang.callStackTree.jsonRepresentation() {
                    if let frames = Self.heaviestPathFrames(json) {
                        detail["leafFrames"] = frames
                    }
                    let appPaths = Self.heaviestAppPaths(json)
                    if !appPaths.isEmpty { detail["appPaths"] = appPaths }
                }
                RemoteLogger.log(
                    category: "diagnostics", event: "hang_diagnostic", level: "warning",
                    message: "main thread unresponsive for \(String(format: "%.1f", hang.hangDuration.converted(to: .seconds).value))s",
                    detail: detail
                )
            }

            // Which code did the writing is the whole question with these, and
            // only the stack answers it: one account was reported writing 4 GB
            // and 17 GB in single days with nothing to say what wrote it.
            for exception in payload.diskWriteExceptionDiagnostics ?? [] {
                var detail: [String: Any] = [
                    "writesCausedMB": exception.totalWritesCaused.converted(to: .megabytes).value,
                ]
                if let json = try? exception.callStackTree.jsonRepresentation() {
                    if let frames = Self.heaviestPathFrames(json) {
                        detail["leafFrames"] = frames
                    }
                    let appPaths = Self.heaviestAppPaths(json)
                    if !appPaths.isEmpty { detail["appPaths"] = appPaths }
                }
                RemoteLogger.log(
                    category: "diagnostics", event: "disk_write_exception", level: "warning",
                    message: "excessive disk writes",
                    detail: detail
                )
            }
        }
    }

    /// The frames of the most-sampled path through a MetricKit call-stack
    /// tree, innermost first, as `binary+0xoffset` (symbolicate with the dSYMs
    /// attached to each release).
    ///
    /// The tree is stored root-first, so the old `callStackPrefix` (the first
    /// 1,800 characters of its JSON) held only the outermost frames: thread
    /// start, the run loop, `main`. Every watchdog report sent so far ended
    /// there, before reaching the frame that was actually busy. Following the
    /// heaviest child at each level reaches it; for a crash tree, which has one
    /// sample, that is simply the crashing stack.
    static func heaviestPathFrames(_ json: Data, limit: Int = 30) -> [String]? {
        guard let root = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
              let stacks = root["callStacks"] as? [[String: Any]],
              let stack = stacks.first(where: { ($0["threadAttributed"] as? Bool) == true }) ?? stacks.first,
              var level = stack["callStackRootFrames"] as? [[String: Any]]
        else { return nil }
        var path: [String] = []
        while let heaviest = level.max(by: { ($0["sampleCount"] as? Int ?? 0) < ($1["sampleCount"] as? Int ?? 0) }) {
            let binary = heaviest["binaryName"] as? String
                ?? String((heaviest["binaryUUID"] as? String ?? "?").prefix(8))
            let offset = heaviest["offsetIntoBinaryTextSegment"] as? Int ?? 0
            path.append("\(binary)+0x\(String(offset, radix: 16))")
            level = heaviest["subFrames"] as? [[String: Any]] ?? []
        }
        guard !path.isEmpty else { return nil }
        return Array(path.reversed().prefix(limit))
    }

    /// The `limit` most-sampled stacks that pass through this app's own
    /// binary, each as "<samples>x: frame <- frame <- ...", innermost first.
    ///
    /// `heaviestPathFrames` follows the single heaviest branch, and for disk
    /// writes that branch can be pure system code: the first report with a
    /// stack ended in a dispatch worker calling `write`, with no app frame on
    /// it, because the bytes were written by a system queue on the app's
    /// behalf (a download landing on disk, say). The app code that started the
    /// work sits on lighter branches. This walks every leaf and keeps the
    /// heaviest paths that contain an app frame.
    static func heaviestAppPaths(_ json: Data, limit: Int = 3, framesPerPath: Int = 14) -> [String] {
        guard let appBinary = Bundle.main.executableURL?.lastPathComponent,
              let root = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
              let stacks = root["callStacks"] as? [[String: Any]]
        else { return [] }
        var found: [(samples: Int, frames: [String])] = []
        func walk(_ level: [[String: Any]], _ path: [(name: String, frame: String)]) {
            for node in level {
                let binary = node["binaryName"] as? String
                    ?? String((node["binaryUUID"] as? String ?? "?").prefix(8))
                let offset = node["offsetIntoBinaryTextSegment"] as? Int ?? 0
                let next = path + [(binary, "\(binary)+0x\(String(offset, radix: 16))")]
                let children = node["subFrames"] as? [[String: Any]] ?? []
                if children.isEmpty {
                    let innermostFirst = Array(next.reversed())
                    if let firstApp = innermostFirst.firstIndex(where: { $0.name == appBinary }) {
                        // Always reach the innermost app frame, plus a few
                        // callers above it for context.
                        let keep = max(framesPerPath, firstApp + 4)
                        found.append((node["sampleCount"] as? Int ?? 0,
                                      innermostFirst.prefix(keep).map(\.frame)))
                    }
                } else {
                    walk(children, next)
                }
            }
        }
        for stack in stacks {
            walk(stack["callStackRootFrames"] as? [[String: Any]] ?? [], [])
        }
        return found
            .sorted { $0.samples > $1.samples }
            .prefix(limit)
            .map { "\($0.samples)x: " + $0.frames.joined(separator: " <- ") }
    }

    private static func iso(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }
}
