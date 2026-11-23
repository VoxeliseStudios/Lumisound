import SwiftUI

// MARK: - Root

struct TVContentView: View {
    @StateObject private var account = TVAccount.shared
    @StateObject private var client = TVBridgeClient.shared
    // Adopted here (not constructed) for the same reason `client`/`account`
    // are — see TVPlayerModel.shared's doc comment.
    @StateObject private var player = TVPlayerModel.shared
    @StateObject private var themeStore = TVThemeStore.shared
    @State private var selection: TVDestination = .home

    var body: some View {
        Group {
            if account.isLoggedIn, let token = account.token {
                signedIn(token: token)
            } else {
                TVLoginView(account: account)
            }
        }
        // Every view that reads `\.tvTheme` restyles the moment the theme
        // changes (Settings → Appearance), without rebuilding the navigation.
        .environment(\.tvTheme, themeStore.theme)
    }

    @ViewBuilder
    private func signedIn(token: String) -> some View {
            NavigationStack {
                // The marquee across the top, the screen below at full width.
                //
                // Earlier shells put navigation in a left rail and the player in a
                // right column; both took width from every screen and the player
                // column had to be hidden on Search because the system keyboard
                // drew over it. Here the only chrome is one strip, and what's
                // playing lives in its On Air pill.
                VStack(spacing: 0) {
                    TVMarqueeBar(
                        selection: $selection,
                        player: player,
                        client: client,
                        token: token,
                        user: account.user,
                        accountBadge: client.notifications.filter(\.isUnread).count
                    )
                    .zIndex(1)

                    ZStack {
                        switch selection {
                        case .home:
                            TVHomeView(client: client, token: token)
                        case .library:
                            TVLibraryView(client: client, token: token)
                        case .playlists:
                            TVPlaylistsView(client: client, token: token)
                        case .discover:
                            TVDiscoverView(client: client, token: token)
                        case .search:
                            TVSearchView(client: client, token: token)
                        case .account:
                            TVAccountView(client: client, account: account, token: token)
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .focusSection()
                }
                .background(TVAmbientBackground())
                .navigationDestination(for: TVPlayContext.self) { ctx in
                    TVPlayerView(context: ctx, client: client, token: token)
                }
            }
            // Fetched here (not just inside TVAccountView.task, as before) so
            // the nav bar's unread badge is accurate even before Account is
            // ever visited this session.
            .task {
                // Runs for the life of the session — see TVAdvancedTelemetry.
                TVAdvancedTelemetry.HitchMonitor.shared.start()
                TVAdvancedTelemetry.noteScreen(selection.rawValue)
                if client.notifications.isEmpty { await client.fetchNotifications(token: token) }
                // Eagerly primes the stream-ticket cache (see TVBridgeClient's
                // doc comment on it) right at session start — covers both a
                // cold launch already signed in and a fresh interactive
                // sign-in, since this `.task` fires either way the first time
                // this branch of the view appears. Without this, the very
                // first track played each session would still go out with no
                // ticket (streamURL(for:)'s own opportunistic refresh only
                // helps from the SECOND track onward).
                await client.refreshStreamTicketIfNeeded(token: token)
            }
            .onChange(of: selection) { newValue in
                TVAdvancedTelemetry.noteScreen(newValue.rawValue)
            }
    }
}

// MARK: - Search tab

struct TVSearchView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    @State private var query = ""

    private var queue: [TVPlayable] { client.results.compactMap { client.playable(from: $0) } }

    var body: some View {
        // UISearchController-backed so the system tvOS keyboard (and its
        // dictation microphone) is available — SwiftUI's `.searchable` has no
        // supported way to offer voice input. See TVDictationSearch.
        TVDictationSearch(text: $query, placeholder: "Search YouTube", onChange: runSearch) {
            resultsBody
        }
        .ignoresSafeArea()
    }

    @ViewBuilder
    private var resultsBody: some View {
        if client.isSearching {
            TVLoadingState(text: "Searching… this can take a moment")
                .frame(maxHeight: .infinity, alignment: .top)
        } else if let err = client.searchError {
            TVEmptyState(systemImage: "exclamationmark.magnifyingglass",
                         title: "Search didn't work",
                         message: err)
                .frame(maxHeight: .infinity, alignment: .top)
        } else if client.results.isEmpty {
            ScrollView { emptyState }
        } else {
            VStack(alignment: .leading, spacing: 0) {
                TVSignalHeader(query: query, count: client.results.count)
                    .padding(.horizontal, TVMetrics.margin)
                    .padding(.top, 20)
                TVCardGrid(items: client.results) { track, cell in
                    NavigationLink(value: TVPlayContext(queue: queue, startID: track.id)) {
                        TVTrackCard(track: track, width: cell)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .tvSearchTrackActions(client: client, token: token, track: track)
                }
            }
        }
    }

    /// Debounced search-as-you-type. tvOS search keyboards don't reliably fire
    /// a submit action, so the query runs as the text changes — including text
    /// arriving from dictation, which produces no submit event at all.
    private func runSearch(_ value: String) {
        Task {
            try? await Task.sleep(nanoseconds: 600_000_000)
            guard query == value else { return }  // user kept typing/speaking
            if value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                client.results = []
                client.searchError = nil
            } else {
                await client.search(value)
            }
        }
    }

    /// An empty query surfaces something real to browse straight away — the
    /// same Discover Mix data the Discover tab shows, framed as suggestions.
    @ViewBuilder
    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 30) {
            TVEmptyState(
                systemImage: "antenna.radiowaves.left.and.right",
                title: "Searching for a signal",
                message: "Type a song, artist or album — or press the microphone and just say it."
            )
            .padding(.bottom, 20)

            if !client.discoverMix.isEmpty {
                let suggestQueue = client.discoverMix.compactMap { client.playable(from: $0) }
                TVShelfSection(title: "Strong Signals", subtitle: "Picked from the artists you play most") {
                    ForEach(client.discoverMix) { track in
                        NavigationLink(value: TVPlayContext(queue: suggestQueue, startID: track.id)) {
                            TVTrackCard(track: track)
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                        .tvSearchTrackActions(client: client, token: token, track: track)
                    }
                }
            }
        }
        .padding(.bottom, 60)
        .task {
            if client.discoverMix.isEmpty { await client.fetchDiscoverMix(token: token) }
        }
    }
}

/// A YouTube/search result as a 16:9 thumbnail card — the shape the source
/// material actually comes in, so the thumbnail isn't cropped to a square.
struct TVTrackCard: View {
    let track: TVTrack
    var width: CGFloat = 340

    var body: some View {
        TVArtworkCardLabel(
            title: track.title,
            subtitle: track.artist.isEmpty ? "Unknown Artist" : track.artist,
            width: width,
            aspectRatio: 16.0 / 9.0
        ) {
            TVAuthImage(url: URL(string: track.thumbnailURL), token: nil) {
                TVArtPlaceholder(systemImage: "music.note")
            }
        }
    }
}

// MARK: - Account tab — "Backstage Pass"
//
// The account as a backstage pass on a lanyard: the laminate on the left
// (photo, name, handle, a holographic ALL ACCESS band, the headline numbers
// stamped on it, and Sign Out), and the "zones" the pass opens on the right —
// stats, notifications, sessions, Listen Together, settings — with friends'
// listening and the activity feed underneath.

struct TVAccountView: View {
    @ObservedObject var client: TVBridgeClient
    @ObservedObject var account: TVAccount
    let token: String

    @Environment(\.tvTheme) private var theme

    private var unreadNotificationCount: Int {
        client.notifications.filter(\.isUnread).count
    }

    var body: some View {
        ScrollView {
            HStack(alignment: .top, spacing: 60) {
                TVBackstagePass(account: account, client: client)
                    // Its own focus region: Sign Out is the escape hatch from
                    // a bad restored session, so it must be one "left" away.
                    .focusSection()

                VStack(alignment: .leading, spacing: 40) {
                    VStack(alignment: .leading, spacing: 16) {
                        Text("ACCESS ZONES")
                            .font(TVType.eyebrow)
                            .tracking(theme.tracking)
                            .foregroundStyle(TVPalette.neonAlt)
                        zone("A", "Listening Stats", subtitle: "Plays, streaks, badges", systemImage: "chart.bar.fill") {
                            TVStatsView(client: client, token: token)
                        }
                        zone("B", "Notifications",
                             subtitle: unreadNotificationCount > 0 ? "\(unreadNotificationCount) unread" : "All caught up",
                             systemImage: "bell.fill", badge: unreadNotificationCount) {
                            TVNotificationsView(client: client, token: token)
                        }
                        zone("C", "Sessions", subtitle: "Devices signed in", systemImage: "rectangle.stack.badge.person.crop") {
                            TVSessionsView(client: client, account: account, token: token)
                        }
                        zone("D", "Listen Together",
                             subtitle: TVListenTogether.shared.isInRoom ? "In a room now" : "Join a friend's room",
                             systemImage: "person.2.wave.2.fill") {
                            TVListenTogetherView(client: client, token: token)
                        }
                        zone("E", "Settings", subtitle: "Playback, sound, appearance", systemImage: "slider.horizontal.3") {
                            TVSettingsView()
                        }
                    }
                    .focusSection()

                    if !client.friendsListening.isEmpty {
                        VStack(alignment: .leading, spacing: 18) {
                            TVSectionHeader(title: "In the Green Room", subtitle: "Friends listening now")
                            TVFriendsListeningCard(friendsListening: client.friendsListening)
                        }
                    }

                    VStack(alignment: .leading, spacing: 18) {
                        TVSectionHeader(title: "Overheard",
                                        subtitle: "What people who share their listening have played")
                        TVSocialActivityFeed(activity: client.socialActivity)
                    }
                }
                .frame(maxWidth: 1000, alignment: .leading)
            }
            .padding(.horizontal, TVMetrics.margin)
            .padding(.top, 20)
            .padding(.bottom, 80)
        }
        .task {
            if client.notifications.isEmpty { await client.fetchNotifications(token: token) }
            if client.stats == nil { await client.fetchStats(token: token) }
            if client.friendsListening.isEmpty { await client.fetchFriendsListening(token: token) }
            if client.socialActivity.isEmpty { await client.fetchSocialActivity(token: token) }
        }
    }

    @ViewBuilder
    private func zone<Destination: View>(
        _ code: String, _ title: String, subtitle: String, systemImage: String, badge: Int = 0,
        @ViewBuilder destination: () -> Destination
    ) -> some View {
        NavigationLink {
            destination()
        } label: {
            TVZoneLabel(code: code, title: title, subtitle: subtitle, systemImage: systemImage, badge: badge)
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
    }
}

/// The laminate. Fixed size: a label in a tvOS button is offered unlimited
/// width, and the pass is the one thing on the screen that must not grow.
private struct TVBackstagePass: View {
    @ObservedObject var account: TVAccount
    @ObservedObject var client: TVBridgeClient

    @Environment(\.tvTheme) private var theme

    var body: some View {
        VStack(spacing: 0) {
            // Lanyard clip
            Capsule()
                .fill(Color.black.opacity(0.65))
                .frame(width: 110, height: 22)
                .overlay(Capsule().strokeBorder(Color.white.opacity(0.18), lineWidth: 1))
                .padding(.top, 22)

            TVAvatarView(user: account.user, baseURL: client.baseURL, diameter: 170)
                .padding(.top, 26)

            Text(account.user?.name ?? "Signed in")
                .font(theme.display(40))
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .padding(.top, 18)
                .padding(.horizontal, 24)
            if let username = account.user?.username, !username.isEmpty {
                Text("@\(username)")
                    .font(TVType.rowDetail)
                    .foregroundStyle(TVPalette.textSecondary)
            }

            TVHoloBand(text: "ALL ACCESS")
                .padding(.top, 22)

            if let stats = client.stats {
                HStack(spacing: 14) {
                    stamp(value: "\(stats.totalPlays)", label: "PLAYS")
                    stamp(value: listenHours(stats.totalListenSeconds), label: "HOURS")
                }
                .padding(.top, 22)
            }

            Spacer(minLength: 20)

            Button(role: .destructive) {
                account.logout()
            } label: {
                TVPillLabel(title: "Sign Out", systemImage: "rectangle.portrait.and.arrow.right",
                            style: .secondary)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()
            .padding(.bottom, 30)
        }
        .frame(width: 460, height: 800)
        .background {
            RoundedRectangle(cornerRadius: 34, style: .continuous)
                .fill(LinearGradient(colors: [theme.raised, theme.surface, theme.ground],
                                     startPoint: .top, endPoint: .bottom))
        }
        .overlay {
            // The laminate's sheen.
            RoundedRectangle(cornerRadius: 34, style: .continuous)
                .fill(LinearGradient(colors: [.white.opacity(0.12), .clear, .white.opacity(0.05)],
                                     startPoint: .topLeading, endPoint: .bottomTrailing))
                .allowsHitTesting(false)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 34, style: .continuous)
                .strokeBorder(TVPalette.brand, lineWidth: 2)
        }
        .shadow(color: theme.primary.opacity(0.35), radius: 40, y: 20)
        .rotationEffect(.degrees(-1.5))
    }

    private func stamp(value: String, label: String) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(theme.display(30, .black).monospacedDigit())
                .lineLimit(1)
                .minimumScaleFactor(0.6)
            Text(label)
                .font(.system(size: 13, weight: .black, design: .monospaced))
                .tracking(2)
                .foregroundStyle(TVPalette.textTertiary)
        }
        .frame(width: 160, height: 92)
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(TVPalette.neon.opacity(0.6), style: StrokeStyle(lineWidth: 2, dash: [6, 4]))
        }
        .rotationEffect(.degrees(label == "PLAYS" ? -3 : 2.5))
    }

    private func listenHours(_ seconds: Int) -> String {
        let h = Double(seconds) / 3600
        return h >= 10 ? "\(Int(h))" : String(format: "%.1f", h)
    }
}

/// A holographic band: a stripe of the theme's lights that shimmers slowly
/// sideways (an offset of a static gradient — a cheap transform at 60 fps).
private struct TVHoloBand: View {
    let text: String
    @Environment(\.tvTheme) private var theme

    var body: some View {
        TimelineView(.animation) { ctx in
            let phase = CGFloat(ctx.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 6) / 6)
            ZStack {
                LinearGradient(colors: [theme.primary, .white.opacity(0.85), theme.secondary, theme.lift, theme.primary],
                               startPoint: .leading, endPoint: .trailing)
                    .frame(width: 920)
                    .offset(x: -230 + 460 * phase)
                Text(text)
                    .font(.system(size: 22, weight: .black, design: .monospaced))
                    .tracking(8)
                    .foregroundStyle(Color.black.opacity(0.75))
            }
            .frame(width: 460, height: 52)
            .clipped()
        }
        .frame(width: 460, height: 52)
    }
}

/// An access zone row: the zone letter in a block, the title and subtitle,
/// a badge when something waits there.
private struct TVZoneLabel: View {
    let code: String
    let title: String
    let subtitle: String
    let systemImage: String
    var badge: Int = 0

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    var body: some View {
        HStack(spacing: 22) {
            Text(code)
                .font(theme.display(34, .black))
                .foregroundStyle(isFocused ? Color.black : theme.ground)
                .frame(width: 74, height: 74)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(isFocused ? AnyShapeStyle(Color.white) : AnyShapeStyle(TVPalette.brand)))
            VStack(alignment: .leading, spacing: 4) {
                Text("ZONE \(code) · \(title.uppercased())")
                    .font(.system(size: 13, weight: .black, design: .monospaced))
                    .foregroundStyle(TVPalette.textTertiary)
                Text(title)
                    .font(.system(size: 26, weight: .bold))
                Text(subtitle)
                    .font(TVType.rowDetail)
                    .foregroundStyle(TVPalette.textSecondary)
            }
            Spacer(minLength: 10)
            if badge > 0 {
                Text("\(badge)")
                    .font(.system(size: 18, weight: .bold, design: TVTheme.current.design))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(Capsule().fill(Color(red: 0.96, green: 0.33, blue: 0.47)))
            }
            Image(systemName: systemImage)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(isFocused ? TVPalette.neon : TVPalette.textTertiary)
        }
        .padding(18)
        .frame(width: 900)
        .tvNeonCard(cornerRadius: 26, isFocused: isFocused)
    }
}

/// "12 results for 'x'" as signal strength.
private struct TVSignalHeader: View {
    let query: String
    let count: Int
    @Environment(\.tvTheme) private var theme

    var body: some View {
        HStack(alignment: .bottom, spacing: 18) {
            HStack(alignment: .bottom, spacing: 5) {
                ForEach(0..<5, id: \.self) { i in
                    RoundedRectangle(cornerRadius: 2)
                        .fill(i < min(5, max(1, count / 4 + 1)) ? AnyShapeStyle(TVPalette.brandVertical) : AnyShapeStyle(Color.white.opacity(0.12)))
                        .frame(width: 8, height: CGFloat(10 + i * 7))
                }
            }
            VStack(alignment: .leading, spacing: 2) {
                Text("SIGNAL ACQUIRED")
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text("\(count) \(count == 1 ? "match" : "matches") for “\(query)”")
                    .font(TVType.section)
                    .lineLimit(1)
            }
        }
    }
}
