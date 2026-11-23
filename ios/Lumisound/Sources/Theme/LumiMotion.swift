import SwiftUI

// MARK: - LumiMotion
//
// Lumisound's own motion language. Every animation in the app goes through
// one of these instead of stock easeInOut/spring defaults, so things move
// the same way everywhere:
//
// - `press`   — a quick, slightly springy squash for taps (buttons, chips).
// - `snap`    — toggles and selections: fast in, a hint of overshoot.
// - `glide`   — the house curve for layout changes: a long, soft
//               deceleration (cubic 0.22, 1, 0.36, 1) that never bounces.
// - `settle`  — sheets, cards and overlays landing: spring with weight.
// - `arrive`  — first appearance of a screen's hero content.
// - `fade`    — cross-fades and content swaps (symmetric, calm).
// - `drift`   — slow ambient loops (glows, auras), always repeating.
// - `beat`    — a pulse that loops at a musical feel (≈ 100 BPM).
//
// Frame rate: Core Animation transitions run at up to 60 fps (display links
// use `displayRange`; Info.plist leaves CADisableMinimumFrameDurationOnPhone
// off so ProMotion iPhones never run above 60 Hz). Hand-drawn TimelineView
// animation (artwork styles, seekers, avatar effects) uses `frame`, which
// follows the heat governor's content rate (core thermal.rs `content_fps`:
// 30 normally, less when hot or in Low Power Mode). At a fixed 60 fps those
// effects were the biggest main-thread cost on Now Playing in 2.0.0.66.
enum LumiMotion {
    /// Frames per second for Core Animation (springs, transitions).
    static let fps: Double = 60
    /// Content frame rate from the heat governor (set by `HeatGovernor`).
    nonisolated(unsafe) static var contentFps: Double = 30
    /// TimelineView minimum interval for hand-drawn animation. Paused
    /// content (rate 0) still gets a slow 1 s tick so views don't freeze mid-state.
    static var frame: Double { contentFps > 0 ? 1.0 / contentFps : 1.0 }
    /// For CADisplayLink.
    static let displayRange = CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)

    static let press = Animation.spring(response: 0.24, dampingFraction: 0.68, blendDuration: 0.04)
    static let snap = Animation.spring(response: 0.3, dampingFraction: 0.78, blendDuration: 0.05)
    static let glide = Animation.timingCurve(0.22, 1.0, 0.36, 1.0, duration: 0.44)
    static let settle = Animation.spring(response: 0.46, dampingFraction: 0.84, blendDuration: 0.08)
    static let arrive = Animation.spring(response: 0.62, dampingFraction: 0.76, blendDuration: 0.1)
    static let fade = Animation.timingCurve(0.45, 0.0, 0.25, 1.0, duration: 0.32)
    static let quick = Animation.timingCurve(0.3, 0.0, 0.2, 1.0, duration: 0.18)

    /// Slow ambient loop.
    static func drift(_ seconds: Double = 3.2) -> Animation {
        .timingCurve(0.37, 0.0, 0.63, 1.0, duration: seconds).repeatForever(autoreverses: true)
    }

    /// Pulse at a musical tempo (default ≈ 100 BPM).
    static func beat(bpm: Double = 100) -> Animation {
        .timingCurve(0.2, 0.0, 0.4, 1.0, duration: 60 / max(bpm, 40)).repeatForever(autoreverses: true)
    }

    /// Continuous rotation/spin loops.
    static func spin(_ seconds: Double) -> Animation {
        .linear(duration: seconds).repeatForever(autoreverses: false)
    }

    /// The house curve with a custom length (stays the same shape).
    static func glide(_ seconds: Double) -> Animation {
        .timingCurve(0.22, 1.0, 0.36, 1.0, duration: seconds)
    }

    /// Calm cross-fade with a custom length.
    static func fade(_ seconds: Double) -> Animation {
        .timingCurve(0.45, 0.0, 0.25, 1.0, duration: seconds)
    }
}

extension AnyTransition {
    /// Lumisound's entrance: rises a little while sharpening into view.
    static var lumiRise: AnyTransition {
        .modifier(active: LumiRiseModifier(progress: 0), identity: LumiRiseModifier(progress: 1))
    }
}

private struct LumiRiseModifier: ViewModifier {
    let progress: Double
    func body(content: Content) -> some View {
        content
            .opacity(progress)
            .offset(y: (1 - progress) * 14)
            .scaleEffect(0.97 + 0.03 * progress)
            .blur(radius: (1 - progress) * 4)
    }
}
