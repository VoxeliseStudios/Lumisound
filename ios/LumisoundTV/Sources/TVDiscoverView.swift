import SwiftUI

// MARK: - TVDiscoverView
//
// Round 3: Discover Mix, On This Day, and server-computed Smart Playlists —
// all bridge-backed (GET /user/discover-mix, /user/on-this-day, /user/music/
// smart-playlists), unlike iOS's on-device smart-playlist engine / mood
// playlist service, which read the local library scan tvOS doesn't have.
// Discover Mix and On This Day both need real play history to have anything
// to show — TVPlayerModel now reports plays via POST /user/history as it
// plays, same as this screen's data ultimately depends on.

struct TVDiscoverView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String

    @Environment(\.tvTheme) private var theme
    @FocusState private var focusedStation: String?
    @State private var tuned: String?

    // MARK: Stations

    enum StationKind: Equatable {
        case subscriptions
        case discoverMix
        case onThisDay(year: Int)
        case smart(key: String)
    }

    struct Station: Identifiable, Equatable {
        let id: String
        let name: String
        let detail: String
        let systemImage: String
        let kind: StationKind
        var frequency: Double = 0
    }

    /// Everything Discover can play, as stations on the band, spaced from
    /// 88.1 up in 2.4 MHz steps.
    private var stations: [Station] {
        var out: [Station] = []
        if !client.discoverMix.isEmpty {
            out.append(Station(id: "mix", name: "Discover Mix", detail: "From artists you play most",
                               systemImage: "dot.radiowaves.left.and.right", kind: .discoverMix))
        }
        if !client.subscriptionFeed.isEmpty {
            out.append(Station(id: "subs", name: "Subscriptions", detail: "New from channels you follow",
                               systemImage: "antenna.radiowaves.left.and.right", kind: .subscriptions))
        }
        for group in client.onThisDay {
            out.append(Station(id: "otd-\(group.year)",
                               name: group.yearsAgo == 1 ? "1 Year Ago" : "\(group.yearsAgo) Years Ago",
                               detail: "On this day in \(group.year)", systemImage: "clock.arrow.circlepath",
                               kind: .onThisDay(year: group.year)))
        }
        for bucket in client.smartPlaylists where !bucket.tracks.isEmpty {
            out.append(Station(id: "smart-\(bucket.key)", name: bucket.name, detail: tvSongCount(bucket.tracks.count),
                               systemImage: tvSmartPlaylistIcon(bucket.key), kind: .smart(key: bucket.key)))
        }
        return out.enumerated().map { i, st in
            var s = st
            s.frequency = 88.1 + Double(i) * 2.4
            return s
        }
    }

    private var tunedStation: Station? {
        let all = stations
        return all.first(where: { $0.id == tuned }) ?? all.first
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                header
                if stations.isEmpty {
                    nothingYetHint
                } else {
                    dial
                        .focusSection()
                    if let station = tunedStation {
                        broadcast(station)
                            .id(station.id)
                            .transition(.opacity)
                    }
                }
            }
            .padding(.bottom, 80)
            .animation(.easeOut(duration: 0.3), value: tunedStation?.id)
        }
        .task {
            if client.discoverMix.isEmpty { await client.fetchDiscoverMix(token: token) }
            if client.onThisDay.isEmpty { await client.fetchOnThisDay(token: token) }
            if client.smartPlaylists.isEmpty { await client.fetchSmartPlaylists(token: token) }
            if client.subscriptionFeed.isEmpty { await client.fetchSubscriptionFeed(token: token) }
        }
        .onChange(of: focusedStation) { id in
            // Tuning follows the needle: moving along the dial changes the
            // broadcast, no press needed.
            if let id { tuned = id }
        }
    }

    private var header: some View {
        HStack(alignment: .lastTextBaseline, spacing: 26) {
            VStack(alignment: .leading, spacing: 6) {
                Text("THE DIAL")
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text("Discover")
                    .font(theme.display(58))
            }
            if let station = tunedStation {
                HStack(spacing: 12) {
                    Circle().fill(Color(red: 1, green: 0.25, blue: 0.3)).frame(width: 10, height: 10)
                    Text(String(format: "TUNED TO %.1f FM", station.frequency))
                        .font(.system(size: 18, weight: .black, design: .monospaced))
                        .foregroundStyle(TVPalette.textSecondary)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, TVMetrics.margin)
        .padding(.top, 20)
    }

    // MARK: Dial

    /// The tuner band: a scale of tick marks, a station at each frequency,
    /// and the red needle over whichever is tuned.
    private var dial: some View {
        VStack(alignment: .leading, spacing: 0) {
            TVDialScale()
                .frame(height: 34)
                .padding(.horizontal, 30)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 18) {
                    ForEach(stations) { station in
                        Button {
                            tuned = station.id
                        } label: {
                            TVStationLabel(station: station, isTuned: tunedStation?.id == station.id)
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                        .focused($focusedStation, equals: station.id)
                    }
                }
                .padding(.horizontal, 30)
                .padding(.vertical, 22)
            }
        }
        .padding(.vertical, 18)
        .background {
            RoundedRectangle(cornerRadius: 30, style: .continuous)
                .fill(LinearGradient(colors: [theme.surface.opacity(0.9), theme.ground.opacity(0.9)],
                                     startPoint: .top, endPoint: .bottom))
                .overlay {
                    RoundedRectangle(cornerRadius: 30, style: .continuous)
                        .strokeBorder(Color.white.opacity(0.08), lineWidth: 1)
                }
                // The dial's glass glow.
                .shadow(color: theme.primary.opacity(0.18), radius: 30, y: 10)
        }
        .padding(.horizontal, TVMetrics.margin)
    }

    // MARK: Broadcast

    @ViewBuilder
    private func broadcast(_ station: Station) -> some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(spacing: 16) {
                Image(systemName: station.systemImage)
                    .font(.system(size: 26, weight: .semibold))
                    .foregroundStyle(TVPalette.neon)
                VStack(alignment: .leading, spacing: 2) {
                    Text(station.name)
                        .font(theme.display(34, .bold))
                    Text(station.detail)
                        .font(TVType.rowDetail)
                        .foregroundStyle(TVPalette.textTertiary)
                }
            }
            .padding(.horizontal, TVMetrics.margin)

            switch station.kind {
            case .discoverMix:
                trackRow(client.discoverMix)
            case .subscriptions:
                trackRow(client.subscriptionFeed.compactMap { $0.track })
            case .onThisDay(let year):
                trackRow(client.onThisDay.first(where: { $0.year == year })?.tracks ?? [])
            case .smart(let key):
                if let bucket = client.smartPlaylists.first(where: { $0.key == key }) {
                    smartBroadcast(bucket)
                }
            }
        }
    }

    private func trackRow(_ tracks: [TVTrack]) -> some View {
        let queue = tracks.compactMap { client.playable(from: $0) }
        return ScrollView(.horizontal, showsIndicators: false) {
            LazyHStack(alignment: .top, spacing: 34) {
                ForEach(tracks) { track in
                    NavigationLink(value: TVPlayContext(queue: queue, startID: track.id)) {
                        TVTrackCard(track: track)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .tvSearchTrackActions(client: client, token: token, track: track)
                }
            }
            .padding(.horizontal, TVMetrics.margin)
            .padding(.vertical, 24)
        }
        .padding(.vertical, -24)
    }

    /// A tempo mix on air: its art, play and open, and the first of its songs
    /// as a running order.
    private func smartBroadcast(_ bucket: TVSmartPlaylistBucket) -> some View {
        let resolved = bucket.tracks.compactMap { client.resolvedTrack(for: $0) }
        let queue = resolved.compactMap { client.playable(from: $0, token: token) }
        return HStack(alignment: .top, spacing: 44) {
            TVGeneratedArt(seed: bucket.key, systemImage: tvSmartPlaylistIcon(bucket.key), title: bucket.name)
                .frame(width: 300, height: 300)
                .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
                .shadow(color: .black.opacity(0.5), radius: 24, y: 12)
            VStack(alignment: .leading, spacing: 16) {
                HStack(spacing: 20) {
                    if let first = queue.first {
                        NavigationLink(value: TVPlayContext(queue: queue, startID: first.id)) {
                            TVPillLabel(title: "Tune In", systemImage: "play.fill")
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                    NavigationLink {
                        TVSmartPlaylistDetailView(client: client, token: token, bucket: bucket)
                    } label: {
                        TVPillLabel(title: "Running Order", systemImage: "list.bullet", style: .secondary)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
                ForEach(Array(resolved.prefix(6).enumerated()), id: \.element.id) { i, track in
                    HStack(spacing: 14) {
                        Text(String(format: "%02d", i + 1))
                            .font(.system(size: 17, weight: .bold, design: .monospaced))
                            .foregroundStyle(TVPalette.neon)
                        Text(track.displayTitle)
                            .font(.system(size: 21, weight: .semibold))
                            .lineLimit(1)
                        Text(track.artist)
                            .font(TVType.rowDetail)
                            .foregroundStyle(TVPalette.textTertiary)
                            .lineLimit(1)
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, TVMetrics.margin)
        .task {
            if client.library.isEmpty { await client.fetchLibrary(token: token) }
        }
    }

    /// Shown only when there is no station at all yet.
    @ViewBuilder
    private var nothingYetHint: some View {
        let stillLoading = client.isLoadingDiscoverMix || client.isLoadingOnThisDay
            || client.isLoadingSubscriptionFeed || client.isLoadingSmartPlaylists
        if stillLoading {
            TVLoadingState(text: "Scanning the band…")
        } else {
            TVEmptyState(
                systemImage: "radio",
                title: "Nothing on the dial yet",
                message: "Play a few tracks and stations fill in: a Discover Mix, anniversaries and tempo mixes."
            )
        }
    }
}

/// A station on the band: its frequency, big, and its name. The tuned station
/// carries the red needle.
private struct TVStationLabel: View {
    let station: TVDiscoverView.Station
    let isTuned: Bool

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    var body: some View {
        VStack(spacing: 8) {
            Text(String(format: "%.1f", station.frequency))
                .font(.system(size: 34, weight: .heavy, design: .monospaced))
                .foregroundStyle(isFocused ? Color.black : (isTuned ? TVPalette.neon : Color.white.opacity(0.7)))
            HStack(spacing: 8) {
                Image(systemName: station.systemImage)
                    .font(.system(size: 15, weight: .semibold))
                Text(station.name)
                    .font(.system(size: 18, weight: .semibold))
                    .lineLimit(1)
            }
            .foregroundStyle(isFocused ? Color.black.opacity(0.75) : TVPalette.textSecondary)
        }
        .padding(.horizontal, 26)
        .padding(.vertical, 16)
        .frame(minWidth: 210)
        .background {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(isFocused ? Color.white : (isTuned ? theme.raised : Color.clear))
        }
        .overlay(alignment: .top) {
            // The needle.
            Capsule()
                .fill(Color(red: 1, green: 0.22, blue: 0.27))
                .frame(width: 4, height: 30)
                .shadow(color: Color(red: 1, green: 0.22, blue: 0.27), radius: 8)
                .offset(y: -34)
                .opacity(isTuned ? 1 : 0)
        }
        .scaleEffect(isFocused ? 1.06 : 1)
        .animation(.spring(response: 0.28, dampingFraction: 0.8), value: isFocused)
        .animation(.spring(response: 0.35, dampingFraction: 0.8), value: isTuned)
    }
}

/// The dial's printed scale: major and minor ticks with frequency numbers.
private struct TVDialScale: View, Equatable {
    var body: some View {
        Canvas { ctx, size in
            let count = 120
            for i in 0...count {
                let x = size.width * CGFloat(i) / CGFloat(count)
                let major = i % 10 == 0
                let h: CGFloat = major ? size.height * 0.7 : size.height * 0.35
                ctx.fill(Path(CGRect(x: x, y: size.height - h, width: major ? 2 : 1, height: h)),
                         with: .color(.white.opacity(major ? 0.35 : 0.15)))
            }
        }
    }
}

// MARK: - Smart playlist detail

struct TVSmartPlaylistDetailView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let bucket: TVSmartPlaylistBucket

    var body: some View {
        // Computed ONCE per render rather than as instance `var`s — each was
        // previously re-read (and recomputed from scratch) by `meta`, the
        // Play button, AND once per row's `ForEach` closure, so an N-track
        // bucket paid O(N) work (itself O(N · client.library.count) on a
        // `resolvedTrack(for:)` cache miss, since that's an unindexed linear
        // scan) N+2 times per render — O(N²) overall. Same fix shape as
        // `TVPlaylistDetailView`.
        let resolvedTracks: [UserMusicTrack] = bucket.tracks.compactMap { client.resolvedTrack(for: $0) }
        let queue: [TVPlayable] = resolvedTracks.compactMap { client.playable(from: $0, token: token) }
        TVGatefold(
            eyebrow: "Tempo Mix",
            title: bucket.name,
            subtitle: "Built from your cloud library's tempo",
            meta: tvCollectionMeta(resolvedTracks)
        ) {
            TVGeneratedArt(seed: bucket.key, systemImage: tvSmartPlaylistIcon(bucket.key))
        } actions: {
            if let first = queue.first {
                NavigationLink(value: TVPlayContext(queue: queue, startID: first.id)) {
                    TVPillLabel(title: "Tune In", systemImage: "play.fill")
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
            }
        } tracks: {
            if client.isLoadingLibrary {
                TVLoadingState(text: "Matching against your library…")
            } else if resolvedTracks.isEmpty {
                TVEmptyState(systemImage: "waveform.slash",
                             title: "Nothing matched",
                             message: "None of this playlist's songs were found in your cloud library.")
            } else {
                TVTrackList {
                    ForEach(resolvedTracks) { track in
                        NavigationLink(value: TVPlayContext(queue: queue, startID: track.id)) {
                            TVTrackRow(
                                artworkURL: client.userMusicArtworkURL(for: track),
                                token: token,
                                title: track.displayTitle,
                                artist: track.artist,
                                detail: track.duration.tvDurationText,
                                isFavorite: client.isFavorite(track.id)
                            )
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                        .tvTrackActions(client: client, token: token, track: track)
                    }
                }
            }
        }
        .tvAmbientBackground()
        .task {
            if client.library.isEmpty { await client.fetchLibrary(token: token) }
        }
    }
}
