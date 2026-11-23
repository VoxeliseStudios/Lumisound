import SwiftUI

// MARK: - TVTheme — app-wide styles
//
// Lumisound TV ships several complete looks rather than one. A theme sets the
// ground (the near-black floor), the two "light" colours every primary action
// and selection uses, the lifted text tints, and the display typeface — so a
// change of theme restyles every screen at once (the palette and type scale in
// TVDesignSystem read from the active theme).
//
// Picked in Settings → Appearance; stored in UserDefaults ("tv.theme").

enum TVTheme: String, CaseIterable, Identifiable {
    case afterglow, prism, nocturne, vinylNoir, aurora, synthwave, studio

    var id: String { rawValue }

    /// The active theme, readable from anywhere (palette, fonts, non-view code).
    /// Set only by `TVThemeStore`.
    nonisolated(unsafe) static var current: TVTheme = {
        TVTheme(rawValue: UserDefaults.standard.string(forKey: TVThemeStore.key) ?? "") ?? .afterglow
    }()

    var displayName: String {
        switch self {
        case .afterglow: return "Afterglow"
        case .prism: return "Prism"
        case .nocturne: return "Neon Nocturne"
        case .vinylNoir: return "Vinyl Noir"
        case .aurora: return "Aurora"
        case .synthwave: return "Synthwave"
        case .studio: return "Studio Mono"
        }
    }

    var tagline: String {
        switch self {
        case .afterglow: return "Ember and magenta, like stage lights after the encore"
        case .prism: return "The icon's violet and blue, the original look"
        case .nocturne: return "Cyan and hot pink on midnight navy"
        case .vinylNoir: return "Gold leaf and serif type on black lacquer"
        case .aurora: return "Northern-lights teal drifting into violet"
        case .synthwave: return "Sunset pink and orange, monospaced and loud"
        case .studio: return "Graphite and silver, nothing but the music"
        }
    }

    // MARK: Colours

    var ground: Color {
        switch self {
        case .afterglow: return Color(red: 0.055, green: 0.031, blue: 0.035)
        case .prism: return Color(red: 0.035, green: 0.031, blue: 0.075)
        case .nocturne: return Color(red: 0.016, green: 0.027, blue: 0.075)
        case .vinylNoir: return Color(red: 0.02, green: 0.02, blue: 0.02)
        case .aurora: return Color(red: 0.016, green: 0.047, blue: 0.051)
        case .synthwave: return Color(red: 0.067, green: 0.02, blue: 0.098)
        case .studio: return Color(red: 0.055, green: 0.058, blue: 0.067)
        }
    }

    var surface: Color {
        switch self {
        case .afterglow: return Color(red: 0.14, green: 0.075, blue: 0.08)
        case .prism: return Color(red: 0.094, green: 0.086, blue: 0.173)
        case .nocturne: return Color(red: 0.05, green: 0.08, blue: 0.17)
        case .vinylNoir: return Color(red: 0.09, green: 0.08, blue: 0.065)
        case .aurora: return Color(red: 0.05, green: 0.12, blue: 0.12)
        case .synthwave: return Color(red: 0.16, green: 0.06, blue: 0.2)
        case .studio: return Color(red: 0.12, green: 0.125, blue: 0.14)
        }
    }

    var raised: Color {
        switch self {
        case .afterglow: return Color(red: 0.23, green: 0.12, blue: 0.12)
        case .prism: return Color(red: 0.149, green: 0.137, blue: 0.259)
        case .nocturne: return Color(red: 0.08, green: 0.13, blue: 0.26)
        case .vinylNoir: return Color(red: 0.16, green: 0.14, blue: 0.11)
        case .aurora: return Color(red: 0.08, green: 0.2, blue: 0.19)
        case .synthwave: return Color(red: 0.26, green: 0.09, blue: 0.3)
        case .studio: return Color(red: 0.2, green: 0.205, blue: 0.225)
        }
    }

    /// First light (top-left of every brand gradient).
    var primary: Color {
        switch self {
        case .afterglow: return Color(red: 1.0, green: 0.45, blue: 0.22)
        case .prism: return Color(red: 0.545, green: 0.361, blue: 0.965)
        case .nocturne: return Color(red: 0.0, green: 0.86, blue: 0.95)
        case .vinylNoir: return Color(red: 0.86, green: 0.68, blue: 0.34)
        case .aurora: return Color(red: 0.12, green: 0.84, blue: 0.68)
        case .synthwave: return Color(red: 1.0, green: 0.29, blue: 0.68)
        case .studio: return Color(red: 0.6, green: 0.62, blue: 0.68)
        }
    }

    /// Second light (bottom-right of every brand gradient).
    var secondary: Color {
        switch self {
        case .afterglow: return Color(red: 0.92, green: 0.2, blue: 0.5)
        case .prism: return Color(red: 0.231, green: 0.510, blue: 0.965)
        case .nocturne: return Color(red: 1.0, green: 0.18, blue: 0.62)
        case .vinylNoir: return Color(red: 0.55, green: 0.36, blue: 0.16)
        case .aurora: return Color(red: 0.43, green: 0.38, blue: 0.98)
        case .synthwave: return Color(red: 1.0, green: 0.58, blue: 0.18)
        case .studio: return Color(red: 0.3, green: 0.32, blue: 0.38)
        }
    }

    /// The second light lifted for legibility as text on the ground.
    var lift: Color {
        switch self {
        case .afterglow: return Color(red: 1.0, green: 0.68, blue: 0.45)
        case .prism: return Color(red: 0.443, green: 0.643, blue: 1.0)
        case .nocturne: return Color(red: 0.45, green: 0.95, blue: 1.0)
        case .vinylNoir: return Color(red: 0.97, green: 0.9, blue: 0.74)
        case .aurora: return Color(red: 0.55, green: 0.98, blue: 0.85)
        case .synthwave: return Color(red: 1.0, green: 0.76, blue: 0.45)
        case .studio: return Color(red: 0.93, green: 0.94, blue: 0.97)
        }
    }

    /// The first light lifted the same way (eyebrows, kickers).
    var liftAlt: Color {
        switch self {
        case .afterglow: return Color(red: 1.0, green: 0.56, blue: 0.72)
        case .prism: return Color(red: 0.690, green: 0.553, blue: 1.0)
        case .nocturne: return Color(red: 1.0, green: 0.52, blue: 0.82)
        case .vinylNoir: return Color(red: 0.93, green: 0.78, blue: 0.48)
        case .aurora: return Color(red: 0.7, green: 0.66, blue: 1.0)
        case .synthwave: return Color(red: 1.0, green: 0.55, blue: 0.85)
        case .studio: return Color(red: 0.78, green: 0.8, blue: 0.86)
        }
    }

    // MARK: Type

    /// The display face: titles, numbers, hero type.
    var design: Font.Design {
        switch self {
        case .afterglow, .prism, .aurora: return .rounded
        case .nocturne, .studio: return .default
        case .vinylNoir: return .serif
        case .synthwave: return .monospaced
        }
    }

    /// Kickers and eyebrows letter-spacing — wider for the quieter themes.
    var tracking: CGFloat {
        switch self {
        case .vinylNoir, .studio: return 4.2
        case .synthwave: return 1.6
        default: return 2.6
        }
    }

    func display(_ size: CGFloat, _ weight: Font.Weight = .heavy) -> Font {
        .system(size: size, weight: design == .serif ? .bold : weight, design: design)
    }
}

/// Owns the theme choice; views that must restyle the instant it changes read
/// `\.tvTheme` from the environment (set at the root).
@MainActor
final class TVThemeStore: ObservableObject {
    static let shared = TVThemeStore()
    nonisolated static let key = "tv.theme"

    @Published var theme: TVTheme {
        didSet {
            TVTheme.current = theme
            UserDefaults.standard.set(theme.rawValue, forKey: Self.key)
            TVRemoteLogger.log(category: "settings", event: "theme_changed", detail: ["theme": theme.rawValue])
        }
    }

    private init() {
        theme = TVTheme.current
    }
}

private struct TVThemeKey: EnvironmentKey {
    static let defaultValue: TVTheme = .afterglow
}

extension EnvironmentValues {
    /// The active theme. Reading it makes a view re-render on a theme change.
    var tvTheme: TVTheme {
        get { self[TVThemeKey.self] }
        set { self[TVThemeKey.self] = newValue }
    }
}

/// A theme's swatch: its ground with both lights — Settings → Appearance and
/// the theme preview chips.
struct TVThemeSwatch: View {
    let theme: TVTheme
    var size: CGFloat = 120

    var body: some View {
        ZStack {
            theme.ground
            RadialGradient(colors: [theme.primary.opacity(0.85), .clear],
                           center: .init(x: 0.2, y: 0.15), startRadius: 0, endRadius: size * 0.9)
            RadialGradient(colors: [theme.secondary.opacity(0.8), .clear],
                           center: .init(x: 0.85, y: 0.9), startRadius: 0, endRadius: size * 0.85)
            Text("Aa")
                .font(theme.display(size * 0.3))
                .foregroundStyle(theme.lift)
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.22, style: .continuous))
    }
}
