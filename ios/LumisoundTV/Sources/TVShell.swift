import SwiftUI

// MARK: - TVMarqueeBar — the shell's navigation
//
// A theatre marquee across the top of the screen: the brand mark, the
// destinations as lit words (the current one carries a light bar under it),
// and on the right an "On Air" pill for whatever is playing plus the account.
//
// It replaces the side rail and the Now Playing column. Navigation runs
// left/right along one strip and content scrolls below it, so moving through a
// list never crosses the navigation, and the content gets the full width of the
// screen. The On Air pill is the route to the full player; the Siri Remote's
// play/pause works from anywhere.
//
// Focus notes (lessons from earlier shells): the bar is one focus section that
// is never empty (the destinations are always there), and the On Air pill is
// added with a plain `if` — no zero-size placeholder that could become a focus
// target of its own.

struct TVMarqueeBar: View {
    @Binding var selection: TVDestination
    @ObservedObject var player: TVPlayerModel
    @ObservedObject var client: TVBridgeClient
    let token: String
    let user: TVUser?
    var accountBadge: Int = 0

    @Environment(\.tvTheme) private var theme

    private var destinations: [TVDestination] {
        TVDestination.allCases.filter { $0 != .account }
    }

    var body: some View {
        HStack(spacing: 30) {
            TVBrandMark(height: 34)
                .padding(.trailing, 14)

            HStack(spacing: 10) {
                ForEach(destinations) { dest in
                    Button {
                        selection = dest
                    } label: {
                        TVMarqueeItemLabel(title: dest.title(accountName: ""),
                                           systemImage: dest.systemImage,
                                           isSelected: selection == dest)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }

            Spacer(minLength: 20)

            if let track = player.current, !player.isShowingFullPlayer {
                NavigationLink {
                    TVPlayerView(client: client, token: token)
                } label: {
                    TVOnAirPillLabel(track: track, isPlaying: player.isPlaying, isBuffering: player.isBuffering)
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
                .transition(.move(edge: .top).combined(with: .opacity))
            }

            Button {
                selection = .account
            } label: {
                TVMarqueeAccountLabel(user: user, baseURL: client.baseURL,
                                      isSelected: selection == .account, badge: accountBadge)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()
        }
        .padding(.horizontal, TVMetrics.margin)
        .padding(.top, 34)
        .padding(.bottom, 14)
        .background {
            LinearGradient(colors: [theme.ground.opacity(0.92), theme.ground.opacity(0)],
                           startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea()
        }
        .animation(.easeOut(duration: 0.3), value: player.current?.id)
        .focusSection()
    }
}

/// One destination: the word, lit. Selected = light bar underneath and full
/// white; focused = a white plate (the strongest tvOS signal on a small control).
private struct TVMarqueeItemLabel: View {
    let title: String
    let systemImage: String
    let isSelected: Bool

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    var body: some View {
        VStack(spacing: 6) {
            HStack(spacing: 10) {
                Image(systemName: systemImage)
                    .font(.system(size: 20, weight: .semibold))
                Text(title)
                    .font(theme.display(24, isSelected ? .heavy : .semibold))
                    .lineLimit(1)
            }
            .foregroundStyle(isFocused ? Color.black : (isSelected ? Color.white : Color.white.opacity(0.55)))
            .padding(.horizontal, 22)
            .padding(.vertical, 12)
            .background {
                Capsule().fill(isFocused ? Color.white : Color.clear)
            }

            // The marquee light under the current destination.
            Capsule()
                .fill(TVPalette.brandHorizontal)
                .frame(width: isSelected ? 46 : 0, height: 4)
                .shadow(color: theme.primary.opacity(0.9), radius: 8)
                .opacity(isSelected ? 1 : 0)
        }
        .scaleEffect(isFocused ? 1.06 : 1)
        .animation(.spring(response: 0.28, dampingFraction: 0.8), value: isFocused)
        .animation(.spring(response: 0.35, dampingFraction: 0.8), value: isSelected)
    }
}

/// The avatar at the end of the marquee — the Account destination.
private struct TVMarqueeAccountLabel: View {
    let user: TVUser?
    let baseURL: String
    let isSelected: Bool
    var badge: Int = 0

    @Environment(\.isFocused) private var isFocused

    var body: some View {
        TVAvatarView(user: user, baseURL: baseURL, diameter: 62, showsRing: false)
            .overlay {
                Circle()
                    .strokeBorder(isFocused ? AnyShapeStyle(Color.white)
                                  : isSelected ? AnyShapeStyle(TVPalette.brand)
                                  : AnyShapeStyle(Color.white.opacity(0.15)),
                                  lineWidth: isFocused || isSelected ? 3 : 1)
            }
            .overlay(alignment: .topTrailing) {
                if badge > 0 {
                    Text("\(badge)")
                        .font(.system(size: 14, weight: .bold, design: TVTheme.current.design))
                        .foregroundStyle(.white)
                        .frame(minWidth: 24, minHeight: 24)
                        .background(Circle().fill(Color(red: 0.96, green: 0.33, blue: 0.47)))
                        .offset(x: 6, y: -6)
                }
            }
            .scaleEffect(isFocused ? 1.12 : 1)
            .animation(.spring(response: 0.28, dampingFraction: 0.8), value: isFocused)
    }
}

/// "On Air": cover, title, a live meter. Lights up red-hot while playing,
/// dims to a standby glow when paused.
private struct TVOnAirPillLabel: View {
    let track: TVPlayable
    let isPlaying: Bool
    let isBuffering: Bool

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    private static let onAir = Color(red: 1.0, green: 0.25, blue: 0.3)

    var body: some View {
        HStack(spacing: 14) {
            TVAuthImage(url: track.artworkURL, token: track.authToken) {
                TVArtPlaceholder(systemImage: "music.note", iconScale: 0.4)
            }
            .frame(width: 52, height: 52)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Circle()
                        .fill(isPlaying ? Self.onAir : Color.white.opacity(0.35))
                        .frame(width: 9, height: 9)
                        .shadow(color: isPlaying ? Self.onAir : .clear, radius: 6)
                    Text(isBuffering ? "TUNING IN" : (isPlaying ? "ON AIR" : "STANDBY"))
                        .font(.system(size: 13, weight: .black, design: theme.design))
                        .tracking(2)
                        .foregroundStyle(isFocused ? Color.black.opacity(0.7) : (isPlaying ? Self.onAir : TVPalette.textTertiary))
                }
                Text(track.title)
                    .font(.system(size: 20, weight: .bold))
                    .foregroundStyle(isFocused ? Color.black : Color.white)
                    .lineLimit(1)
            }
            .frame(maxWidth: 300, alignment: .leading)

            TVPlayingMeter(isAnimating: isPlaying && !isBuffering)
                .scaleEffect(0.75)
                .colorMultiply(isFocused ? .black : .white)
        }
        .padding(.leading, 8)
        .padding(.trailing, 20)
        .padding(.vertical, 8)
        .background {
            Capsule().fill(isFocused ? AnyShapeStyle(Color.white) : AnyShapeStyle(theme.surface.opacity(0.85)))
        }
        .overlay {
            Capsule().strokeBorder(isPlaying ? Self.onAir.opacity(isFocused ? 0 : 0.55) : Color.white.opacity(0.1),
                                   lineWidth: 1.5)
        }
        .shadow(color: isPlaying ? Self.onAir.opacity(isFocused ? 0.2 : 0.35) : .clear, radius: 18)
        .scaleEffect(isFocused ? 1.05 : 1)
        .animation(.spring(response: 0.3, dampingFraction: 0.8), value: isFocused)
    }
}
