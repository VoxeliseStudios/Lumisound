import SwiftUI

// MARK: - PerformanceMonitorView
//
// Live view of the core's performance sampler (rust/core/src/perf.rs):
// CPU (process, per thread, whole device), GPU time, power, memory and
// headroom, frame rate and hitches, thermal state, battery. Refreshes once a
// second while open; reading the live sample costs nothing (the sampler
// runs anyway). The last minute's window is the one telemetry judged.

struct PerformanceMonitorView: View {
    @ObservedObject private var telemetry = PerformanceTelemetryService.shared
    @State private var live = perfLive()
    private let tick = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        LKPage(title: "Performance", subtitle: "What Lumisound costs your phone, live.", icon: "gauge.with.dots.needle.67percent", tint: .teal) {
            LKGrid(columns: 2) {
                LKStat(value: String(format: "%.0f%%", live.cpuPct), label: "CPU (\(live.cores) cores = \(live.cores * 100)%)", icon: "cpu", tint: level(live.cpuPct, 40, 80))
                LKStat(value: String(format: "%.1f%%", live.gpuPct), label: "GPU (this app)", icon: "square.stack.3d.up", tint: level(live.gpuPct, 10, 25))
                LKStat(value: live.energyMw > 0 ? String(format: "%.0f mW", live.energyMw) : "—", label: "Power", icon: "bolt", tint: level(live.energyMw, 600, 1500))
                LKStat(value: String(format: "%.0f MB", live.memoryMb), label: live.memoryAvailableMb >= 0 ? String(format: "Memory (%.0f MB free)", live.memoryAvailableMb) : "Memory", icon: "memorychip", tint: level(live.memoryMb, 500, 900))
                LKStat(value: String(format: "%.0f fps", live.fps), label: String(format: "Hitches %.1f ms/s", live.hitchRatio), icon: "speedometer", tint: level(live.hitchRatio, 5, 10))
                LKStat(value: live.thermal.capitalized, label: "Thermal state", icon: "thermometer.medium", tint: thermalTint(live.thermal))
            }

            LKCard(title: "CPU by thread", footer: "Share of one core over the last second. Device-wide load: \(Int(live.systemCpuPct))%. \(live.threadCount) threads.") {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(live.threads.prefix(8).enumerated()), id: \.offset) { _, t in
                        threadRow(t)
                    }
                    if live.threads.isEmpty {
                        Text("Measuring…").font(.caption).foregroundStyle(AppTheme.textSecondary)
                    }
                }
            }

            if let w = telemetry.lastWindow {
                LKCard(title: "Last minute", footer: telemetry.lastWindowSummary) {
                    VStack(alignment: .leading, spacing: 6) {
                        row("Frames", String(format: "p50 %.1f · p95 %.1f · p99 %.1f · max %.0f ms", w.frames.p50Ms, w.frames.p95Ms, w.frames.p99Ms, w.frames.maxMs))
                        row("Hitches", "\(w.frames.hitches) late, \(w.frames.longHitches) ≥100 ms, \(w.frames.frozen) frozen, \(w.frames.dropped) dropped — \(w.frames.rating)")
                        row("CPU", String(format: "avg %.0f%% · peak %.0f%%", w.cpuAvgPct, w.cpuMaxPct))
                        row("GPU", String(format: "%.0f ms (%.1f%%)", w.gpuMs, w.gpuAvgPct))
                        row("Power", w.energyMwAvg > 0 ? String(format: "avg %.0f · peak %.0f mW · %.0f wakeups/s", w.energyMwAvg, w.energyMwMax, w.wakeupsPerS) : "—")
                        row("Memory", String(format: "peak %.0f MB", w.memoryMbMax))
                        if w.batteryDrainPctPerHour > 0 {
                            row("Battery", String(format: "%.0f%% per hour", w.batteryDrainPctPerHour))
                        }
                        ForEach(Array(w.screens.prefix(4).enumerated()), id: \.offset) { _, s in
                            row(s.screen, String(format: "%.0f s · %.1f ms/s hitch · worst %.0f ms", s.seconds, s.hitchRatio, s.worstFrameMs))
                        }
                        ForEach(w.reasons, id: \.self) { r in
                            LKBanner(icon: "exclamationmark.triangle.fill", text: r, tint: AppTheme.warning)
                        }
                    }
                }
            }

            Text(String(format: "Sampler cost %.0f µs per reading. iOS has no temperature sensor for apps: power use and the thermal state stand in for heat.", live.samplerOverheadUs))
                .font(.caption2).foregroundStyle(AppTheme.textSecondary).padding(.horizontal, 6)
        }
        .onReceive(tick) { _ in live = perfLive() }
        .onAppear { PerformanceTelemetryService.shared.setScreen("Performance") }
    }

    private func threadRow(_ t: ThreadCpu) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(t.name).font(.caption.weight(.semibold)).foregroundStyle(AppTheme.textPrimary).lineLimit(1)
                Spacer()
                Text(String(format: "%.1f%%", t.cpuPct)).font(.caption.monospacedDigit()).foregroundStyle(AppTheme.textSecondary)
            }
            GeometryReader { g in
                Capsule().fill(level(t.cpuPct, 30, 70).gradient)
                    .frame(width: max(2, g.size.width * min(1, t.cpuPct / 100)))
            }
            .frame(height: 4)
        }
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.caption.weight(.semibold)).foregroundStyle(AppTheme.textPrimary).frame(width: 90, alignment: .leading)
            Text(value).font(.caption).foregroundStyle(AppTheme.textSecondary)
        }
    }

    private func level(_ v: Double, _ warn: Double, _ bad: Double) -> Color {
        v >= bad ? AppTheme.error : v >= warn ? AppTheme.warning : AppTheme.success
    }

    private func thermalTint(_ t: String) -> Color {
        switch t {
        case "critical", "serious": return AppTheme.error
        case "fair": return AppTheme.warning
        default: return AppTheme.success
        }
    }
}
