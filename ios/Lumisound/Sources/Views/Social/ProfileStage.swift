import SwiftUI

// MARK: - Profile "stage" (2026-11-15 redesign)
//
// The profile used to be one long column of identical titled cards. It is now
// a stage: a full-bleed banner hero with the avatar and identity centred on
// it, a stat strip, the live "on air" card, and three tabs — Showcase (what
// the person chose to show), Music (what their listening says), Wall
// (guestbook, visitors). Panels are plain shapes (`cellSurface`), not live
// Liquid Glass, and nothing here runs a per-frame SwiftUI timeline.

enum ProfileStageTab: String, CaseIterable, Identifiable {
    case showcase = "Showcase"
    case music = "Music"
    case wall = "Wall"
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .showcase: return "sparkles.rectangle.stack"
        case .music: return "waveform"
        case .wall: return "bubble.left.and.text.bubble.right"
        }
    }
}

// MARK: Hero

struct ProfileStageHero<Avatar: View>: View {
    let mainAccent: Color
    let subAccent: Color
    let bannerImage: UIImage?
    let avatarFrame: String
    let avatarDecoration: String
    let profileEffect: String
    let isOnline: Bool?
    @ViewBuilder let avatar: () -> Avatar

    static var bannerHeight: CGFloat { 220 }
    private let avatarSize: CGFloat = 108

    var body: some View {
        ZStack(alignment: .bottom) {
            banner
                .frame(height: Self.bannerHeight)
                .frame(maxWidth: .infinity)
                .clipped()
                // Pull-down stretch, applied by the renderer from the scroll
                // position (no GeometryReader body re-runs while scrolling).
                .visualEffect { content, proxy in
                    let pull = max(0, proxy.frame(in: .scrollView).minY)
                    return content
                        .scaleEffect(1 + pull / Self.bannerHeight, anchor: .bottom)
                }
            avatarStack
                .offset(y: avatarSize / 2)
        }
        .padding(.bottom, avatarSize / 2)
    }

    private var banner: some View {
        ZStack {
            if let bannerImage {
                AnimatedImageView(image: bannerImage, contentMode: .scaleAspectFill)
            } else {
                LinearGradient(colors: [mainAccent, subAccent], startPoint: .topLeading, endPoint: .bottomTrailing)
            }
            ProfileEffectOverlay(style: ProfileEffectStyle.from(profileEffect), mainTint: mainAccent, subTint: subAccent)
            LinearGradient(
                colors: [.black.opacity(0.25), .clear, AppTheme.background.opacity(0.85)],
                startPoint: .top, endPoint: .bottom
            )
        }
    }

    private var avatarStack: some View {
        ZStack(alignment: .bottomTrailing) {
            ZStack {
                AvatarFrameOverlay(style: AvatarFrameStyle.from(avatarFrame), diameter: avatarSize, mainTint: mainAccent, subTint: subAccent)
                avatar()
                    .frame(width: avatarSize, height: avatarSize)
                    .clipShape(Circle())
                    .overlay(Circle().stroke(AngularGradient(colors: [mainAccent, subAccent, mainAccent], center: .center), lineWidth: 4))
                    .shadowBehind(Circle(), opacity: 0.4, radius: 14, y: 8)
                AvatarDecorationOverlay(style: AvatarDecorationStyle.from(avatarDecoration), diameter: avatarSize, mainTint: mainAccent, subTint: subAccent)
            }
            if let isOnline {
                Circle()
                    .fill(isOnline ? AppTheme.success : AppTheme.textSecondary.opacity(0.6))
                    .frame(width: 22, height: 22)
                    .overlay(Circle().stroke(AppTheme.background, lineWidth: 4))
                    .offset(x: -4, y: -4)
            }
        }
    }
}

// MARK: Identity

struct ProfileIdentityBlock: View {
    let displayName: String
    let username: String
    let pronouns: String?
    let statusEmoji: String?
    let statusText: String?
    let discordVerified: Bool
    let discordUsername: String?
    let tint: Color

    var body: some View {
        VStack(spacing: 6) {
            HStack(spacing: 6) {
                Text(displayName)
                    .font(.system(.title, design: .serif).weight(.bold))
                    .foregroundStyle(AppTheme.textPrimary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                if discordVerified {
                    Image(systemName: "checkmark.seal.fill")
                        .font(.system(size: 17, weight: .bold))
                        .foregroundStyle(Color(red: 0.345, green: 0.396, blue: 0.949))
                }
            }
            HStack(spacing: 6) {
                Text("@\(username)")
                if let pronouns, !pronouns.isEmpty {
                    Text("·")
                    Text(pronouns)
                }
                if discordVerified, let discordUsername, !discordUsername.isEmpty {
                    Text("·")
                    Image(systemName: "gamecontroller.fill").font(.caption2)
                    Text(discordUsername)
                }
            }
            .font(.footnote)
            .foregroundStyle(AppTheme.textSecondary)
            .lineLimit(1)

            if let statusText, !statusText.isEmpty {
                HStack(spacing: 6) {
                    if let statusEmoji, !statusEmoji.isEmpty { Text(statusEmoji) }
                    Text(statusText).lineLimit(1)
                }
                .font(.subheadline.weight(.medium))
                .foregroundStyle(AppTheme.textPrimary)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .background(Capsule().fill(tint.opacity(0.16)))
                .overlay(Capsule().stroke(tint.opacity(0.35), lineWidth: 1))
                .padding(.top, 4)
            }
        }
        .multilineTextAlignment(.center)
        .padding(.horizontal, 20)
    }
}

// MARK: Stat strip

struct ProfileStat: Identifiable {
    var id: String { label }
    let value: String
    let label: String
    let icon: String
}

struct ProfileStatStrip: View {
    let stats: [ProfileStat]
    let tint: Color

    var body: some View {
        if !stats.isEmpty {
            HStack(spacing: 0) {
                ForEach(Array(stats.enumerated()), id: \.element.id) { index, stat in
                    VStack(spacing: 3) {
                        HStack(spacing: 4) {
                            Image(systemName: stat.icon).font(.caption.weight(.bold)).foregroundStyle(tint)
                            Text(stat.value)
                                .font(.system(.title3, design: .rounded).weight(.bold))
                                .foregroundStyle(AppTheme.textPrimary)
                                .monospacedDigit()
                        }
                        Text(stat.label)
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(AppTheme.textSecondary)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity)
                    if index < stats.count - 1 {
                        Rectangle().fill(AppTheme.textSecondary.opacity(0.2)).frame(width: 1, height: 28)
                    }
                }
            }
            .padding(.vertical, 12)
            .cellSurface(in: RoundedRectangle(cornerRadius: 20, style: .continuous), fallback: AppTheme.surface.opacity(0.55))
            .padding(.horizontal, 16)
        }
    }
}

// MARK: Tab picker

struct ProfileStageTabPicker: View {
    @Binding var selection: ProfileStageTab
    let tint: Color
    @Namespace private var pill

    var body: some View {
        HStack(spacing: 4) {
            ForEach(ProfileStageTab.allCases) { tab in
                Button {
                    withAnimation(LumiMotion.snap) { selection = tab }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: tab.icon).font(.caption.weight(.bold))
                        Text(tab.rawValue).font(.subheadline.weight(.semibold))
                    }
                    .foregroundStyle(selection == tab ? Color.white : AppTheme.textSecondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background {
                        if selection == tab {
                            Capsule().fill(tint.gradient).matchedGeometryEffect(id: "profileTab", in: pill)
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(4)
        .background(Capsule().fill(AppTheme.background.opacity(0.92)))
        .overlay(Capsule().stroke(Color.white.opacity(0.08), lineWidth: 1))
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
    }
}

// MARK: Panel

/// A titled section on the stage — the cheap replacement for the old
/// glass `ProfileInfoCard`.
struct ProfilePanel<Content: View>: View {
    let title: String
    let icon: String
    let tint: Color
    var trailing: String? = nil
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: icon)
                    .font(.footnote.weight(.bold))
                    .foregroundStyle(tint)
                    .frame(width: 26, height: 26)
                    .background(Circle().fill(tint.opacity(0.16)))
                Text(title)
                    .font(.system(.headline, design: .serif))
                    .foregroundStyle(AppTheme.textPrimary)
                Spacer()
                if let trailing {
                    Text(trailing).font(.caption.weight(.semibold)).foregroundStyle(AppTheme.textSecondary)
                }
            }
            content()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .cellSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous), fallback: AppTheme.surface.opacity(0.6))
        .padding(.horizontal, 16)
    }
}

// MARK: Showcase pieces

/// Pinned tracks as record sleeves on a shelf. Tapping one plays it (your
/// copy if you have it).
struct PinnedSleeveShelf: View {
    let tracks: [PinnedTrack]
    let mainAccent: Color
    let subAccent: Color
    var onPlay: (PinnedTrack) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            LazyHStack(spacing: 14) {
                ForEach(Array(tracks.enumerated()), id: \.offset) { index, track in
                    Button { onPlay(track) } label: { sleeve(track, index: index) }
                        .buttonStyle(PressableButtonStyle())
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 4)
        }
    }

    private func sleeve(_ track: PinnedTrack, index: Int) -> some View {
        let hue = Double(abs(track.title.hashValue % 360)) / 360
        let colors = index % 2 == 0
            ? [mainAccent, Color(hue: hue, saturation: 0.55, brightness: 0.45)]
            : [subAccent, Color(hue: hue, saturation: 0.5, brightness: 0.35)]
        return VStack(alignment: .leading, spacing: 8) {
            ZStack(alignment: .trailing) {
                // The record, peeking out of its sleeve.
                Circle()
                    .fill(RadialGradient(colors: [Color(white: 0.18), .black], center: .center, startRadius: 4, endRadius: 60))
                    .overlay(Circle().fill(colors[0]).frame(width: 34, height: 34))
                    .overlay(Circle().fill(Color.black).frame(width: 6, height: 6))
                    .frame(width: 118, height: 118)
                    .offset(x: 34)
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing))
                    .frame(width: 128, height: 128)
                    .overlay(alignment: .topLeading) {
                        Text("\(index + 1)")
                            .font(.system(size: 40, weight: .black, design: .serif))
                            .foregroundStyle(.white.opacity(0.85))
                            .padding(10)
                    }
                    .overlay(alignment: .bottomTrailing) {
                        Image(systemName: "play.circle.fill")
                            .font(.system(size: 26))
                            .foregroundStyle(.white)
                            .padding(8)
                    }
                    .shadowBehind(RoundedRectangle(cornerRadius: 14, style: .continuous), opacity: 0.4, radius: 10, y: 6)
            }
            .frame(width: 162, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text(track.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(AppTheme.textPrimary)
                    .lineLimit(1)
                Text(track.artist ?? " ")
                    .font(.caption)
                    .foregroundStyle(AppTheme.textSecondary)
                    .lineLimit(1)
            }
            .frame(width: 140, alignment: .leading)
        }
    }
}

/// Earned badges as a medal wall.
struct ProfileBadgeWall: View {
    let badges: [ProfileBadge]
    let tint: Color

    var body: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 3), spacing: 10) {
            ForEach(badges) { badge in
                VStack(spacing: 6) {
                    Image(systemName: badge.icon)
                        .font(.system(size: 22, weight: .bold))
                        .foregroundStyle(medalColor(badge.tier))
                        .frame(width: 48, height: 48)
                        .background(Circle().fill(medalColor(badge.tier).opacity(0.16)))
                        .overlay(Circle().stroke(medalColor(badge.tier).opacity(0.45), lineWidth: 1.5))
                    Text(badge.label)
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(AppTheme.textPrimary)
                        .multilineTextAlignment(.center)
                        .lineLimit(2)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color.white.opacity(0.04)))
            }
        }
    }

    private func medalColor(_ tier: String) -> Color {
        switch tier.lowercased() {
        case "gold": return Color(red: 0.98, green: 0.78, blue: 0.25)
        case "silver": return Color(white: 0.82)
        case "bronze": return Color(red: 0.85, green: 0.55, blue: 0.33)
        case "platinum", "diamond": return Color(red: 0.6, green: 0.85, blue: 1)
        default: return tint
        }
    }
}

/// Top artists as a strip of portraits.
struct TopArtistsStrip: View {
    let artists: [String]
    let tint: Color

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            LazyHStack(alignment: .top, spacing: 14) {
                ForEach(Array(artists.prefix(10).enumerated()), id: \.offset) { index, artist in
                    NavigationLink {
                        ArtistDetailView(artist: artist)
                    } label: {
                        VStack(spacing: 6) {
                            ArtistAvatar(artist: artist, size: 72)
                                .overlay(Circle().stroke(index == 0 ? tint : Color.white.opacity(0.12), lineWidth: index == 0 ? 3 : 1))
                            Text(artist)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(AppTheme.textPrimary)
                                .lineLimit(1)
                        }
                        .frame(width: 80)
                    }
                    .buttonStyle(PressableButtonStyle())
                }
            }
            .padding(.horizontal, 2)
        }
    }
}

/// Genres as a wrap of tinted chips.
struct GenreChipCloud: View {
    let genres: [String]
    let tint: Color

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { chips }
            ScrollView(.horizontal, showsIndicators: false) { HStack(spacing: 8) { chips } }
        }
    }

    private var chips: some View {
        ForEach(Array(genres.prefix(8).enumerated()), id: \.offset) { index, genre in
            Text(genre)
                .font(.caption.weight(.semibold))
                .foregroundStyle(index == 0 ? Color.white : AppTheme.textPrimary)
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .background(Capsule().fill(index == 0 ? AnyShapeStyle(tint.gradient) : AnyShapeStyle(tint.opacity(0.14))))
        }
    }
}
