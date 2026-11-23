import SwiftUI

// MARK: - TVDesignSystem — "Stage"
//
// The shared visual language for the tvOS app. Every screen is its own set
// piece (Home is a programme guide, Library a record crate, Playlists a wall
// of cassettes, Discover a radio dial, Account a backstage pass, Stats a
// split-flap scoreboard, Settings a mixing desk, Now Playing the stage), and
// they share four rules:
//
//   1. **Colour comes from the theme.** `TVTheme` (Settings → Appearance) sets
//      the ground and the two lights; `TVPalette` below reads it, so every
//      screen restyles together. Colour still marks meaning: the lights appear
//      where something is selected or primary.
//   2. **Focus is light, not a border.** A focused element lifts, brightens
//      and glows; small controls invert to white (the tvOS idiom); large
//      surfaces never go white.
//   3. **The backdrop comes from the music.** `TVAmbientBackground` renders the
//      playing artwork blurred behind every screen, over the theme's floor.
//   4. **One type scale.** Display type uses the theme's face (rounded, serif,
//      monospaced…); the sizes below are the only sizes screens use.

// MARK: Metrics and type scale

enum TVMetrics {
    /// Horizontal screen margin. tvOS overscan eats the outer ~60pt on some
    /// sets, so content starts well inboard of the frame edge.
    static let margin: CGFloat = 60
    /// Gap between major sections down a screen.
    static let section: CGFloat = 52
    /// Gap between sibling rows in a list. Small, because Prism rows have no
    /// card of their own at rest — they read as one list, not a stack of tiles.
    static let row: CGFloat = 6
    static let cardCorner: CGFloat = 18
    static let panelCorner: CGFloat = 30
    /// Default width of a square artwork card on a horizontal shelf.
    static let shelfCard: CGFloat = 228
}

enum TVType {
    /// Screen titles ("Library", "Discover") — one per screen, nothing else.
    static var display: Font { TVTheme.current.display(58, .bold) }
    /// Hero/Now Playing track titles.
    static var hero: Font { TVTheme.current.display(38, .bold) }
    /// Section headings inside a screen.
    static var section: Font { TVTheme.current.display(30, .bold) }
    /// The primary line of a list row or card.
    static let rowTitle = Font.system(size: 24, weight: .semibold)
    /// Secondary/supporting text — artists, counts, hints.
    static let rowDetail = Font.system(size: 20, weight: .regular)
    /// Body copy in panels and empty states.
    static let body = Font.system(size: 22, weight: .regular)
    /// Numeric/trailing metadata: durations, indices, "3 of 40".
    static let meta = Font.system(size: 18, weight: .medium).monospacedDigit()
    /// All-caps kicker above a title.
    static var eyebrow: Font { .system(size: 15, weight: .heavy, design: TVTheme.current.design) }
}

// MARK: Palette

enum TVPalette {
    /// Base ground — the theme's near-black floor.
    static var ground: Color { TVTheme.current.ground }
    /// Quiet raised surface — panels, sheets.
    static var surface: Color { TVTheme.current.surface }
    /// A step above `surface` — focused rows, inset wells.
    static var raised: Color { TVTheme.current.raised }

    /// The theme's first light (was the icon's violet; the name stays so the
    /// many call sites keep meaning "the primary light").
    static var violet: Color { TVTheme.current.primary }
    /// The theme's second light.
    static var blue: Color { TVTheme.current.secondary }

    /// The second light lifted for legibility as text/glyph colour.
    static var neon: Color { TVTheme.current.lift }
    /// The first light lifted the same way.
    static var neonAlt: Color { TVTheme.current.liftAlt }

    static let textSecondary = Color.white.opacity(0.62)
    static let textTertiary = Color.white.opacity(0.38)

    /// Both lights, diagonal — fills, selected states, primary actions.
    static var brand: LinearGradient {
        LinearGradient(colors: [violet, blue], startPoint: .topLeading, endPoint: .bottomTrailing)
    }

    /// Both lights, horizontal — progress bars and meters.
    static var brandHorizontal: LinearGradient {
        LinearGradient(colors: [violet, blue], startPoint: .leading, endPoint: .trailing)
    }

    /// Both lights, top to bottom.
    static var brandVertical: LinearGradient {
        LinearGradient(colors: [violet, blue], startPoint: .top, endPoint: .bottom)
    }
}

// MARK: Surface (focus treatment for rows and cards)

/// The recurring surface: quiet glass at rest, lifted and lit when focused.
///
/// At rest a surface has NO outline. The previous pass drew a rim on every row,
/// which meant a list of thirty songs was thirty boxes — the outlines were the
/// most visible thing on screen. Prism leaves resting rows nearly flat and puts
/// all the contrast into the focused one, which is the only row the person is
/// actually looking at.
///
/// Still named `TVNeonCard`/`tvNeonCard` so the many existing call sites keep
/// compiling; "neon" now just means "this surface lights up".
struct TVNeonCard: ViewModifier {
    var cornerRadius: CGFloat = TVMetrics.cardCorner
    var isFocused: Bool = false
    /// Tints the focus edge and glow — pass an artwork-derived or semantic
    /// colour to make one surface stand apart (Aria's cards use violet).
    var tint: Color? = nil
    /// When true the surface keeps a visible fill at rest (panels, standalone
    /// cards). When false it is nearly invisible until focused (list rows).
    var isProminent: Bool = true

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
    }

    private var edge: LinearGradient {
        if let tint {
            return LinearGradient(colors: [tint, tint.opacity(0.6)],
                                  startPoint: .topLeading, endPoint: .bottomTrailing)
        }
        return LinearGradient(colors: [TVPalette.neonAlt, TVPalette.neon],
                              startPoint: .topLeading, endPoint: .bottomTrailing)
    }

    func body(content: Content) -> some View {
        content
            .background {
                shape.fill(
                    isFocused ? TVPalette.raised.opacity(0.95)
                    : Color.white.opacity(isProminent ? 0.06 : 0.0)
                )
            }
            .overlay {
                shape
                    .strokeBorder(edge, lineWidth: 2)
                    .opacity(isFocused ? 1 : 0)
            }
            .overlay {
                // A hairline at rest on prominent surfaces only, so a panel
                // still has an edge against a bright backdrop.
                if isProminent && !isFocused {
                    shape.strokeBorder(Color.white.opacity(0.07), lineWidth: 1)
                }
            }
            .shadow(color: (tint ?? TVPalette.violet).opacity(isFocused ? 0.45 : 0),
                    radius: isFocused ? 28 : 0, y: isFocused ? 10 : 0)
            .scaleEffect(isFocused ? 1.025 : 1)
            .animation(.spring(response: 0.3, dampingFraction: 0.8), value: isFocused)
    }
}

extension View {
    func tvNeonCard(cornerRadius: CGFloat = TVMetrics.cardCorner,
                    isFocused: Bool = false,
                    tint: Color? = nil,
                    isProminent: Bool = true) -> some View {
        modifier(TVNeonCard(cornerRadius: cornerRadius, isFocused: isFocused,
                            tint: tint, isProminent: isProminent))
    }
}

// MARK: Glass panel

/// A translucent, blurred panel — for content that sits on top of the ambient
/// backdrop and needs a legible surface (profile header, settings groups, stat
/// tiles, the player's panels).
struct TVGlassPanel: ViewModifier {
    var cornerRadius: CGFloat = TVMetrics.panelCorner
    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        content
            .background {
                ZStack {
                    shape.fill(.ultraThinMaterial)
                    shape.fill(TVPalette.surface.opacity(0.55))
                }
            }
            .overlay {
                shape.strokeBorder(
                    LinearGradient(colors: [.white.opacity(0.16), .white.opacity(0.03)],
                                   startPoint: .top, endPoint: .bottom),
                    lineWidth: 1
                )
            }
            .shadow(color: .black.opacity(0.35), radius: 30, y: 14)
    }
}

extension View {
    func tvGlassPanel(cornerRadius: CGFloat = TVMetrics.panelCorner) -> some View {
        modifier(TVGlassPanel(cornerRadius: cornerRadius))
    }
}

// MARK: Ambient background

/// The backdrop behind every screen: the **currently playing artwork**, blown
/// up, heavily blurred and darkened, over the Prism floor.
///
/// Performance notes, because this sits behind scrolling content on every
/// screen:
///   - The blurred artwork layer is **not animated**. A large `.blur` is cheap
///     to composite while static and expensive to re-composite every frame.
///   - `.scaleEffect` past the bounds hides the soft transparent edge a blur
///     leaves behind, which is cheaper than drawing a second bleed layer.
///   - The floor is always drawn, so there is never a flat black frame while
///     artwork loads, and screens still look intentional with nothing playing.
struct TVAmbientBackground: View {
    var accent: Color = TVPalette.violet
    /// This is the app-wide player, observed so the backdrop follows track
    /// changes.
    @StateObject private var player = TVPlayerModel.shared

    private var track: TVPlayable? { player.current }

    var body: some View {
        ZStack {
            floor

            if let track {
                TVAuthImage(url: track.artworkURL, token: track.authToken) {
                    Color.clear
                }
                // Overscale to push the blur's feathered edges off-screen.
                .scaleEffect(1.4)
                .blur(radius: 120, opaque: false)
                .saturation(1.4)
                // A wash, not an image: without this nothing on top is legible.
                .overlay(TVPalette.ground.opacity(0.74))
                .id(track.id)
                .transition(.opacity)
                .animation(.easeInOut(duration: 0.8), value: track.id)
            }

            // Top and bottom vignettes so titles and the bottom of scrolling
            // lists always sit on something dark.
            LinearGradient(
                colors: [TVPalette.ground.opacity(0.55), .clear, .clear, TVPalette.ground.opacity(0.9)],
                startPoint: .top, endPoint: .bottom
            )
        }
        .ignoresSafeArea()
    }

    /// The brand light: the icon's two colours as soft fields bleeding in from
    /// opposite corners of the near-black ground — violet from the top, blue
    /// from the bottom, the same direction they run on the icon.
    private var floor: some View {
        ZStack {
            TVPalette.ground
            RadialGradient(
                colors: [TVPalette.violet.opacity(0.34), .clear],
                center: .init(x: 0.18, y: -0.08), startRadius: 0, endRadius: 1100
            )
            RadialGradient(
                colors: [TVPalette.blue.opacity(0.26), .clear],
                center: .init(x: 0.92, y: 1.08), startRadius: 0, endRadius: 1000
            )
        }
    }
}

extension View {
    /// Drops `TVAmbientBackground` behind this view — apply once, near the
    /// root of a screen's `body`, not per row/card.
    func tvAmbientBackground(accent: Color = TVPalette.violet) -> some View {
        background(TVAmbientBackground(accent: accent))
    }
}

// MARK: Brand mark

/// The Lumisound mark: the icon's five capsule bars. Drawn rather than loaded
/// so it scales cleanly anywhere from the rail to the sign-in screen.
struct TVBrandMark: View {
    var height: CGFloat = 44
    /// White bars on the gradient tile (the icon itself) when true; gradient
    /// bars on nothing when false.
    var onTile: Bool = true

    /// Relative bar heights, measured off the app icon.
    private static let bars: [CGFloat] = [0.41, 0.73, 1.0, 0.63, 0.49]

    var body: some View {
        let barWidth = height * 0.105
        let bars = HStack(alignment: .center, spacing: barWidth * 0.72) {
            ForEach(Array(Self.bars.enumerated()), id: \.offset) { _, h in
                Capsule().frame(width: barWidth, height: height * h)
            }
        }
        .frame(height: height)

        if onTile {
            bars
                .foregroundStyle(.white)
                .padding(height * 0.36)
                .background {
                    RoundedRectangle(cornerRadius: height * 0.42, style: .continuous)
                        .fill(TVPalette.brandVertical)
                }
                .shadow(color: TVPalette.violet.opacity(0.55), radius: height * 0.4, y: height * 0.12)
        } else {
            bars.foregroundStyle(TVPalette.brandVertical)
        }
    }
}

// MARK: Grid layout

/// Card grids sized by COLUMN COUNT rather than by a minimum card width, so the
/// "Cards per row" setting decides the card size.
@MainActor
enum TVGridLayout {
    static func columns(spacing: CGFloat = 40) -> [GridItem] {
        let count = max(2, min(4, TVAudioSettings.shared.gridColumns))
        return Array(repeating: GridItem(.flexible(), spacing: spacing), count: count)
    }
}

// MARK: Screen title

/// The one large title at the top of a screen, with an optional count/summary
/// as a pill beside it. Every tab shares this anchor.
struct TVScreenTitle: View {
    let title: String
    var detail: String? = nil
    var eyebrow: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let eyebrow {
                Text(eyebrow.uppercased())
                    .font(TVType.eyebrow)
                    .tracking(TVTheme.current.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
            }
            HStack(alignment: .center, spacing: 22) {
                Text(title)
                    .font(TVType.display)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                if let detail {
                    Text(detail)
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textSecondary)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 7)
                        .background(Color.white.opacity(0.08), in: Capsule())
                }
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, TVMetrics.margin)
        .padding(.top, 36)
    }
}

// MARK: Generated artwork

/// Artwork for things that have none of their own — playlists, genres, smart
/// playlists, missing covers.
///
/// Every one of these used to be the same accent-to-black square with a glyph
/// in the middle, so a shelf of eight playlists was eight identical tiles and
/// the only way to tell them apart was to read the captions. A gradient chosen
/// deterministically from the item's NAME gives each one a stable colour of its
/// own — the same playlist is always the same colour, across screens and across
/// launches — while staying inside the brand's family of hues.
struct TVGeneratedArt: View {
    let seed: String
    var systemImage: String = "music.note"
    /// Printed on the art itself, Apple Music–style. Omit for small tiles.
    var title: String? = nil

    /// Brand-adjacent pairs: the icon's own violet/blue first, then neighbours
    /// around it so a shelf of them reads as one family.
    private static let palettes: [(Color, Color)] = [
        (Color(red: 0.55, green: 0.36, blue: 0.97), Color(red: 0.23, green: 0.51, blue: 0.97)),
        (Color(red: 0.86, green: 0.31, blue: 0.73), Color(red: 0.49, green: 0.27, blue: 0.93)),
        (Color(red: 0.16, green: 0.67, blue: 0.86), Color(red: 0.25, green: 0.33, blue: 0.91)),
        (Color(red: 0.98, green: 0.45, blue: 0.47), Color(red: 0.70, green: 0.27, blue: 0.85)),
        (Color(red: 0.20, green: 0.78, blue: 0.66), Color(red: 0.16, green: 0.45, blue: 0.86)),
        (Color(red: 0.99, green: 0.66, blue: 0.30), Color(red: 0.93, green: 0.30, blue: 0.53)),
        (Color(red: 0.39, green: 0.40, blue: 0.95), Color(red: 0.12, green: 0.16, blue: 0.45)),
        (Color(red: 0.62, green: 0.46, blue: 0.98), Color(red: 0.93, green: 0.42, blue: 0.75)),
    ]

    /// FNV-1a over the seed's scalars. NOT `hashValue`, which Swift randomises
    /// per launch — the colour would change every time the app opened.
    private var paletteIndex: Int {
        var hash: UInt32 = 2_166_136_261
        for scalar in seed.unicodeScalars {
            hash ^= scalar.value
            hash = hash &* 16_777_619
        }
        return Int(hash % UInt32(Self.palettes.count))
    }

    var body: some View {
        let pair = Self.palettes[paletteIndex]
        GeometryReader { geo in
            let side = min(geo.size.width, geo.size.height)
            ZStack(alignment: .topLeading) {
                LinearGradient(colors: [pair.0, pair.1], startPoint: .topLeading, endPoint: .bottomTrailing)
                RadialGradient(colors: [.white.opacity(0.28), .clear],
                               center: .init(x: 0.15, y: 0.1), startRadius: 0, endRadius: side * 0.8)
                Image(systemName: systemImage)
                    .font(.system(size: side * 0.42, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.22))
                    .rotationEffect(.degrees(-8))
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
                    .offset(x: side * 0.08, y: side * 0.06)
                if let title {
                    Text(title)
                        .font(.system(size: max(16, side * 0.11), weight: .heavy, design: TVTheme.current.design))
                        .foregroundStyle(.white)
                        .lineLimit(3)
                        .minimumScaleFactor(0.6)
                        .shadow(color: .black.opacity(0.25), radius: 6, y: 2)
                        .padding(side * 0.09)
                }
            }
            .frame(width: geo.size.width, height: geo.size.height)
        }
        .clipped()
    }
}

/// The fallback while artwork loads or when a track has none. Brand gradient
/// with a soft glyph — clearly a placeholder, never mistaken for real artwork.
struct TVArtPlaceholder: View {
    let systemImage: String
    var iconScale: CGFloat = 1.0

    var body: some View {
        ZStack {
            LinearGradient(
                colors: [TVPalette.violet.opacity(0.55), TVPalette.blue.opacity(0.35), TVPalette.surface],
                startPoint: .topLeading, endPoint: .bottomTrailing
            )
            Image(systemName: systemImage)
                .font(.system(size: 44 * iconScale, weight: .medium))
                .foregroundStyle(.white.opacity(0.7))
        }
    }
}

// MARK: Section header

/// Heading for a section or shelf within a screen.
struct TVSectionHeader: View {
    let title: String
    var subtitle: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(TVType.section)
                .lineLimit(1)
            if let subtitle {
                // Well below the title in size and contrast: a hint, not a
                // second heading. tvOS guidance is to minimise on-screen text.
                Text(subtitle)
                    .font(TVType.rowDetail)
                    .foregroundStyle(TVPalette.textTertiary)
                    .lineLimit(1)
            }
        }
    }
}

// MARK: - Top-level destinations
//
// The shell's six destinations, rendered by `TVMarqueeBar`. Kept separate from
// whatever draws it, which has changed several times.

enum TVDestination: String, CaseIterable, Identifiable {
    case home, library, playlists, discover, search, account
    var id: String { rawValue }

    var systemImage: String {
        switch self {
        case .home:      return "house.fill"
        // NOT `music.note.house.fill`: next to Home's `house.fill` the two are
        // the same silhouette at a glance.
        case .library:   return "square.stack.fill"
        case .playlists: return "music.note.list"
        case .discover:  return "sparkles"
        case .search:    return "magnifyingglass"
        case .account:   return "person.crop.circle.fill"
        }
    }

    func title(accountName: String) -> String {
        switch self {
        case .home:      return "Home"
        case .library:   return "Library"
        case .playlists: return "Playlists"
        case .discover:  return "Discover"
        case .search:    return "Search"
        case .account:   return accountName
        }
    }
}

// MARK: - Chip

/// Filter pills — Library's Songs/Albums/Artists selector, settings choices,
/// anywhere a segmented control would otherwise be needed.
///
/// Three states that must be distinguishable at once: resting (glass), selected
/// (brand gradient), and focused (white — the strongest tvOS focus signal, used
/// on small controls only).
struct TVChip: View {
    let title: String
    let isSelected: Bool
    var systemImage: String? = nil
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        HStack(spacing: 10) {
            if let systemImage {
                Image(systemName: systemImage)
                    .font(.system(size: 18, weight: .semibold))
            }
            Text(title)
                .lineLimit(1)
        }
        .font(.system(size: 21, weight: isSelected || isFocused ? .semibold : .medium, design: TVTheme.current.design))
        .foregroundStyle(
            isFocused ? Color.black
            : isSelected ? Color.white
            : Color.white.opacity(0.7)
        )
        .padding(.horizontal, 26)
        .padding(.vertical, 13)
        .background {
            if isFocused {
                Capsule().fill(Color.white)
            } else if isSelected {
                Capsule().fill(TVPalette.brand)
            } else {
                Capsule().fill(Color.white.opacity(0.08))
            }
        }
        .shadow(color: (isFocused ? Color.white : TVPalette.violet)
                    .opacity(isFocused ? 0.35 : (isSelected ? 0.45 : 0)),
                radius: isFocused ? 18 : 14, y: 6)
        .scaleEffect(isFocused ? 1.07 : 1.0)
        .animation(.spring(response: 0.3, dampingFraction: 0.78), value: isFocused)
    }
}

// MARK: - Pill button label

/// The label for every action button in the port — Play, Shuffle, Retry, Sign
/// In, See All. Used inside a `.plain` button/link with `.focusEffectDisabled()`
/// so focus is drawn here rather than by the system.
///
/// These replace `Label(...)` inside `.buttonStyle(.card)`, which drew a grey
/// card around the text that looked like nothing else in the app and lifted
/// like a piece of artwork.
struct TVPillLabel: View {
    enum Style { case primary, secondary }

    let title: String
    var systemImage: String? = nil
    var style: Style = .primary
    /// Fixed width when set — for a button that should not resize as its label
    /// changes (e.g. Sign In ↔ a spinner).
    var width: CGFloat? = nil
    var isLoading: Bool = false

    @Environment(\.isFocused) private var isFocused

    var body: some View {
        HStack(spacing: 12) {
            if isLoading {
                ProgressView()
                    .tint(isFocused ? .black : .white)
            } else {
                if let systemImage {
                    Image(systemName: systemImage)
                        .font(.system(size: 22, weight: .bold))
                }
                Text(title)
                    .lineLimit(1)
            }
        }
        .font(.system(size: 24, weight: .semibold, design: TVTheme.current.design))
        .foregroundStyle(isFocused ? Color.black : Color.white)
        .padding(.horizontal, 34)
        .frame(width: width)
        .frame(height: 66)
        .background {
            if isFocused {
                Capsule().fill(Color.white)
            } else if style == .primary {
                Capsule().fill(TVPalette.brand)
            } else {
                Capsule().fill(Color.white.opacity(0.12))
            }
        }
        .shadow(color: isFocused ? Color.white.opacity(0.3)
                    : (style == .primary ? TVPalette.violet.opacity(0.45) : .clear),
                radius: isFocused ? 22 : 16, y: 8)
        .scaleEffect(isFocused ? 1.07 : 1)
        .animation(.spring(response: 0.3, dampingFraction: 0.78), value: isFocused)
    }
}

/// Round icon-only button label — utility controls, close buttons.
struct TVIconButtonLabel: View {
    let systemImage: String
    var isOn: Bool = false
    var size: CGFloat = 64
    /// Colour of the glyph when `isOn` and unfocused.
    var tint: Color = TVPalette.neon

    @Environment(\.isFocused) private var isFocused

    var body: some View {
        Image(systemName: systemImage)
            .font(.system(size: size * 0.36, weight: .semibold))
            .foregroundStyle(isFocused ? Color.black : (isOn ? tint : Color.white.opacity(0.75)))
            .frame(width: size, height: size)
            .background {
                Circle().fill(isFocused ? Color.white
                              : (isOn ? tint.opacity(0.18) : Color.white.opacity(0.08)))
            }
            .overlay(alignment: .bottom) {
                // A small dot under an active toggle, so on/off is readable
                // even while focus is elsewhere and the glyph colour is subtle.
                if isOn && !isFocused {
                    Circle().fill(tint).frame(width: 6, height: 6).offset(y: 12)
                }
            }
            .shadow(color: isFocused ? .white.opacity(0.3) : .clear, radius: 16)
            .scaleEffect(isFocused ? 1.12 : 1)
            .animation(.spring(response: 0.28, dampingFraction: 0.75), value: isFocused)
    }
}

// MARK: - Artwork card label

/// The label for every square (or 16:9) artwork card — albums, playlists,
/// shelves, search results.
///
/// Drawn by the port rather than by `.buttonStyle(.card)`: the system card
/// style lifts the caption along with the artwork and puts its own white halo
/// around the whole thing. Here only the ARTWORK lifts and lights; the caption
/// stays put and simply brightens, which is how the platform's own media apps
/// treat a poster.
struct TVArtworkCardLabel<Art: View>: View {
    let title: String
    var subtitle: String? = nil
    let width: CGFloat
    var aspectRatio: CGFloat = 1
    var isCircle: Bool = false
    @ViewBuilder var art: () -> Art

    @Environment(\.isFocused) private var isFocused

    private var artHeight: CGFloat { width / aspectRatio }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            artLayer
                .frame(width: width, height: artHeight)
                .scaleEffect(isFocused ? 1.07 : 1)
                .shadow(color: isFocused ? TVPalette.violet.opacity(0.5) : .black.opacity(0.4),
                        radius: isFocused ? 30 : 14, y: isFocused ? 16 : 8)
                .zIndex(1)

            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.system(size: 21, weight: .semibold))
                    .foregroundStyle(isFocused ? Color.white : Color.white.opacity(0.88))
                    .lineLimit(2, reservesSpace: true)
                    .multilineTextAlignment(.leading)
                if let subtitle {
                    Text(subtitle)
                        .font(.system(size: 18))
                        .foregroundStyle(TVPalette.textTertiary)
                        .lineLimit(1)
                }
            }
            // Definite width, so the text wraps inside the card instead of
            // reporting an ideal width that stretches it.
            .frame(width: width, alignment: .leading)
            .offset(y: isFocused ? 8 : 0)
        }
        .frame(width: width)
        .animation(.spring(response: 0.32, dampingFraction: 0.78), value: isFocused)
    }

    @ViewBuilder
    private var artLayer: some View {
        if isCircle {
            art()
                .clipShape(Circle())
                .overlay {
                    Circle()
                        .strokeBorder(TVPalette.brand, lineWidth: 3)
                        .opacity(isFocused ? 1 : 0)
                }
        } else {
            let shape = RoundedRectangle(cornerRadius: TVMetrics.cardCorner, style: .continuous)
            art()
                .clipShape(shape)
                .overlay {
                    shape.strokeBorder(Color.white.opacity(isFocused ? 0.5 : 0.08),
                                       lineWidth: isFocused ? 2 : 1)
                }
        }
    }
}

// MARK: - Empty / loading / error states

/// One shape for every "nothing here" moment, so an empty Library, an empty
/// Playlists and a failed fetch all look like the same app.
struct TVEmptyState<Action: View>: View {
    let systemImage: String
    let title: String
    var message: String? = nil
    @ViewBuilder var action: () -> Action

    var body: some View {
        VStack(spacing: 20) {
            ZStack {
                Circle()
                    .fill(TVPalette.brand)
                    .opacity(0.22)
                    .frame(width: 150, height: 150)
                    .blur(radius: 20)
                Image(systemName: systemImage)
                    .font(.system(size: 60, weight: .semibold))
                    .foregroundStyle(TVPalette.brandVertical)
            }
            .frame(height: 150)

            Text(title)
                .font(.system(size: 34, weight: .bold, design: TVTheme.current.design))
                .multilineTextAlignment(.center)
            if let message {
                Text(message)
                    .font(TVType.body)
                    .foregroundStyle(TVPalette.textSecondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 720)
                    .fixedSize(horizontal: false, vertical: true)
            }
            action()
                .padding(.top, 10)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 90)
        .padding(.horizontal, TVMetrics.margin)
    }
}

extension TVEmptyState where Action == EmptyView {
    init(systemImage: String, title: String, message: String? = nil) {
        self.init(systemImage: systemImage, title: title, message: message) { EmptyView() }
    }
}

/// Loading indicator for a whole screen or section.
struct TVLoadingState: View {
    var text: String = "Loading…"

    var body: some View {
        VStack(spacing: 22) {
            TVLoadingBars()
            Text(text)
                .font(TVType.rowDetail)
                .foregroundStyle(TVPalette.textSecondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 120)
    }
}

/// The brand mark's bars, bouncing — the loading indicator. A system spinner
/// is the one element that looked identical in every app on the box.
struct TVLoadingBars: View {
    var height: CGFloat = 54
    @State private var phase = false

    private static let rest: [CGFloat] = [0.41, 0.73, 1.0, 0.63, 0.49]

    var body: some View {
        HStack(alignment: .center, spacing: height * 0.09) {
            ForEach(0..<5, id: \.self) { i in
                Capsule()
                    .fill(TVPalette.brandVertical)
                    .frame(width: height * 0.12,
                           height: height * (phase ? Self.rest[4 - i] : Self.rest[i]))
                    .animation(
                        .easeInOut(duration: 0.55)
                            .repeatForever(autoreverses: true)
                            .delay(Double(i) * 0.08),
                        value: phase
                    )
            }
        }
        .frame(height: height)
        .onAppear { phase = true }
    }
}

// MARK: - Shelf section

/// A titled, horizontally-scrolling row of cards with an optional "See All"
/// destination — the recurring shape every Home/Discover shelf reduces to.
/// The destination is type-erased (`AnyView`) so the no-"See All" initializer
/// doesn't need a stand-in type.
struct TVShelfSection<Content: View>: View {
    let title: String
    var subtitle: String? = nil
    let content: () -> Content
    var seeAll: (() -> AnyView)? = nil

    init(title: String, subtitle: String? = nil, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.subtitle = subtitle
        self.content = content
        self.seeAll = nil
    }

    init<Destination: View>(
        title: String,
        subtitle: String? = nil,
        @ViewBuilder content: @escaping () -> Content,
        @ViewBuilder seeAll: @escaping () -> Destination
    ) {
        self.title = title
        self.subtitle = subtitle
        self.content = content
        self.seeAll = { AnyView(seeAll()) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(alignment: .center) {
                TVSectionHeader(title: title, subtitle: subtitle)
                Spacer(minLength: 20)
                if let seeAll {
                    NavigationLink {
                        // A pushed screen replaces the whole shell, backdrop
                        // included, so it brings its own.
                        seeAll().tvAmbientBackground()
                    } label: {
                        TVSeeAllLabel()
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
            .padding(.horizontal, TVMetrics.margin)

            ScrollView(.horizontal, showsIndicators: false) {
                // `LazyHStack`, not a plain `HStack` — every other large
                // collection in this codebase (the card grids via
                // `TVCardGrid`'s `LazyVGrid`, lists via `TVTrackList`'s
                // `LazyVStack`) already renders lazily; this was the one
                // container still building every child eagerly regardless
                // of what's actually on/near screen.
                LazyHStack(alignment: .top, spacing: 34) { content() }
                    .padding(.leading, TVMetrics.margin)
                    // Room on the trailing edge so the row runs out in empty
                    // space instead of being cut through a card, and vertical
                    // room for a focused card's lift and glow, which a scroll
                    // view would otherwise clip.
                    .padding(.trailing, TVMetrics.margin + 40)
                    .padding(.vertical, 24)
            }
            .padding(.vertical, -24)
        }
    }
}

/// "See All ›" — small, so it never competes with the shelf title.
private struct TVSeeAllLabel: View {
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        HStack(spacing: 8) {
            Text("See All")
            Image(systemName: "chevron.right")
                .font(.system(size: 16, weight: .bold))
        }
        .font(.system(size: 20, weight: .semibold, design: TVTheme.current.design))
        .foregroundStyle(isFocused ? Color.black : TVPalette.textSecondary)
        .padding(.horizontal, 22)
        .padding(.vertical, 10)
        .background(Capsule().fill(isFocused ? Color.white : Color.white.opacity(0.07)))
        .scaleEffect(isFocused ? 1.08 : 1)
        .animation(.spring(response: 0.28, dampingFraction: 0.78), value: isFocused)
    }
}

// MARK: - Card grid

/// A grid of artwork cards laid out at a DEFINITE width per cell.
///
/// Measuring the container and handing each card an exact width removes the
/// ambiguity of flexible columns: a card is told its size instead of inferring
/// one from its caption (which made cards grow as wide as their longest title).
/// That is also what makes the column-count setting mean anything.
struct TVCardGrid<Item: Identifiable, Card: View>: View {
    let items: [Item]
    var spacing: CGFloat = 40
    var topPadding: CGFloat = 4
    @ViewBuilder var card: (Item, CGFloat) -> Card

    var body: some View {
        GeometryReader { geo in
            let count = max(2, min(4, TVAudioSettings.shared.gridColumns))
            let usable = geo.size.width - TVMetrics.margin * 2 - spacing * CGFloat(count - 1)
            let cell = max(140, usable / CGFloat(count))
            ScrollView {
                LazyVGrid(
                    columns: Array(repeating: GridItem(.fixed(cell), spacing: spacing), count: count),
                    spacing: spacing + 16
                ) {
                    ForEach(items) { item in
                        card(item, cell)
                    }
                }
                .padding(.horizontal, TVMetrics.margin)
                .padding(.top, topPadding + 20)
                .padding(.bottom, 80)
            }
        }
    }
}

/// The caption under a grid card, clamped to the cell's width. Kept for any
/// caller still composing its own card; new cards use `TVArtworkCardLabel`.
struct TVCardCaption: View {
    let title: String
    let subtitle: String
    let width: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.system(size: 21, weight: .semibold))
                .lineLimit(2, reservesSpace: true)
                .multilineTextAlignment(.leading)
            Text(subtitle)
                .font(.system(size: 18))
                .foregroundStyle(TVPalette.textTertiary)
                .lineLimit(1)
        }
        .frame(width: width, alignment: .leading)
    }
}

// MARK: - Gatefold (collection detail layout)

/// Every collection detail screen — album, genre, playlist, smart mix — opens
/// like a gatefold sleeve: the cover on the left with its record sliding out
/// from behind it, the title and actions under it, and the tracklist down the
/// right as its own scrolling column. Left and right are separate focus
/// regions, so moving through a long tracklist never drags the header along
/// and the Play button is always one press to the left.
struct TVGatefold<Art: View, Actions: View, Tracks: View>: View {
    let eyebrow: String
    let title: String
    var subtitle: String? = nil
    var meta: String? = nil
    @ViewBuilder var art: () -> Art
    @ViewBuilder var actions: () -> Actions
    @ViewBuilder var tracks: () -> Tracks

    @Environment(\.tvTheme) private var theme

    var body: some View {
        HStack(alignment: .top, spacing: 40) {
            VStack(alignment: .leading, spacing: 14) {
                ZStack(alignment: .leading) {
                    TVVinylDisc(diameter: 380)
                        .offset(x: 150)
                    art()
                        .frame(width: 400, height: 400)
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .overlay {
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .strokeBorder(Color.white.opacity(0.14), lineWidth: 1)
                        }
                        .shadow(color: .black.opacity(0.6), radius: 30, x: 10, y: 18)
                }
                .frame(width: 560, height: 400, alignment: .leading)
                .padding(.bottom, 16)

                Text(eyebrow.uppercased())
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text(title)
                    .font(theme.display(46))
                    .lineLimit(2)
                    .minimumScaleFactor(0.55)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.system(size: 24, weight: .medium))
                        .foregroundStyle(TVPalette.textSecondary)
                        .lineLimit(2)
                }
                if let meta {
                    Text(meta)
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textTertiary)
                }
                HStack(spacing: 20) { actions() }
                    .padding(.top, 12)
                Spacer(minLength: 0)
            }
            .frame(width: 560, alignment: .leading)
            .padding(.leading, TVMetrics.margin)
            .padding(.top, 50)
            .focusSection()

            ScrollView {
                tracks()
                    .padding(.top, 50)
                    .padding(.bottom, 80)
            }
            .focusSection()
        }
    }
}

/// "12 songs" / "1 song".
func tvSongCount(_ n: Int) -> String {
    "\(n) \(n == 1 ? "song" : "songs")"
}
