import SwiftUI

// MARK: - TVHomeView — "Tonight on Lumisound"
//
// Home is a TV programme guide. A clock and a greeting at the top, a "Now
// Showing" feature, then the guide itself: one row per channel (Aria's pick,
// Recently Added, your playlists, Discover Mix, smart mixes, On This Day), each
// a strip of programmes whose WIDTH is the programme's length and whose air
// time runs forward from now — so a row reads like the evening's schedule of
// that channel, and a long DJ mix looks long.
//
// Every channel is data the app already fetches; Home doesn't introduce a
// parallel model.

struct TVHomeView: View {
    @ObservedObject var client: TVBridgeClient
    @ObservedObject private var aria = TVAria.shared
    let token: String

    @Environment(\.tvTheme) private var theme

    // Cached, NOT computed (see the sort-cost note in `rebuildRecentlyAdded`):
    // SwiftUI re-evaluates `body` whenever `client` publishes.
    @State private var recentlyAdded: [UserMusicTrack] = []
    @State private var libraryQueue: [TVPlayable] = []

    private var heroTrack: UserMusicTrack? { recentlyAdded.first }

    private var discoverQueue: [TVPlayable] {
        client.discoverMix.compactMap { client.playable(from: $0) }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 34) {
                masthead
                nowShowing
                guide
            }
            .padding(.bottom, 80)
        }
        .task {
            if client.library.isEmpty { await client.fetchLibrary(token: token) }
            if client.playlists.isEmpty { await client.fetchPlaylists(token: token) }
            if client.discoverMix.isEmpty { await client.fetchDiscoverMix(token: token) }
            if client.smartPlaylists.isEmpty { await client.fetchSmartPlaylists(token: token) }
            if client.onThisDay.isEmpty { await client.fetchOnThisDay(token: token) }
        }
        .task(id: client.library.count) { await rebuildRecentlyAdded() }
        .task {
            if aria.dailyPick == nil { await aria.loadDailyPick(token: token) }
        }
    }

    /// Newest first, sorted off the main actor: the sort parses an ISO 8601
    /// timestamp per comparison and only needs redoing when the library does.
    private func rebuildRecentlyAdded() async {
        let library = client.library
        let started = Date()
        let sorted = await Task.detached(priority: .userInitiated) {
            library.sorted {
                ($0.uploadedDate ?? .distantPast) > ($1.uploadedDate ?? .distantPast)
            }
        }.value
        recentlyAdded = sorted
        libraryQueue = sorted.compactMap { client.playable(from: $0, token: token) }
        TVRemoteLogger.log(
            category: "performance", event: "home_sort_completed",
            detail: ["trackCount": sorted.count,
                     "elapsedMs": Int(Date().timeIntervalSince(started) * 1000)]
        )
    }

    // MARK: Masthead

    private var greeting: String {
        let hour = Calendar.current.component(.hour, from: Date())
        let part = hour < 5 ? "Good night" : hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening"
        if let name = TVAccount.shared.user?.name, !name.isEmpty, name != "You" {
            return "\(part), \(name)"
        }
        return part
    }

    private var programmeName: String {
        let hour = Calendar.current.component(.hour, from: Date())
        switch hour {
        case 5..<12: return "THIS MORNING ON LUMISOUND"
        case 12..<18: return "THIS AFTERNOON ON LUMISOUND"
        case 18..<23: return "TONIGHT ON LUMISOUND"
        default: return "LATE NIGHT ON LUMISOUND"
        }
    }

    private var masthead: some View {
        HStack(alignment: .lastTextBaseline, spacing: 30) {
            VStack(alignment: .leading, spacing: 8) {
                Text(programmeName)
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text(greeting)
                    .font(theme.display(60))
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
            }
            Spacer(minLength: 20)
            // The guide's clock. Minute resolution: nothing on Home needs more.
            TimelineView(.everyMinute) { ctx in
                VStack(alignment: .trailing, spacing: 2) {
                    Text(ctx.date, format: .dateTime.hour().minute())
                        .font(theme.display(54, .bold).monospacedDigit())
                        .foregroundStyle(TVPalette.brandHorizontal)
                    Text(ctx.date, format: .dateTime.weekday(.wide).month(.abbreviated).day())
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textTertiary)
                }
            }
        }
        .padding(.horizontal, TVMetrics.margin)
        .padding(.top, 18)
    }

    // MARK: Now Showing

    @ViewBuilder
    private var nowShowing: some View {
        if let track = heroTrack, let first = libraryQueue.first {
            TVNowShowingCard(
                eyebrow: "NOW SHOWING · NEWEST IN YOUR LIBRARY",
                title: track.displayTitle,
                subtitle: track.artist.isEmpty ? "Unknown Artist" : track.artist,
                meta: [track.album.isEmpty ? nil : track.album, track.duration.tvDurationText].compactMap { $0 }.joined(separator: "  ·  "),
                art: {
                    TVAuthImage(url: client.userMusicArtworkURL(for: track), token: token) {
                        TVArtPlaceholder(systemImage: "music.note", iconScale: 2.4)
                    }
                },
                actions: {
                    NavigationLink(value: TVPlayContext(queue: libraryQueue, startID: first.id)) {
                        TVPillLabel(title: "Watch Live", systemImage: "play.fill")
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            )
        } else if client.isLoadingLibrary {
            TVLoadingState(text: "Tuning the guide…")
                .frame(height: 360)
        } else {
            offAir
        }
    }

    /// Nothing in the library yet: the station's test card.
    private var offAir: some View {
        VStack(spacing: 22) {
            TVTestCardBars()
                .frame(width: 560, height: 120)
                .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            Text("Off Air")
                .font(theme.display(52))
            Text("Add music to your Personal Cloud Library from the iPhone app and the guide fills in.")
                .font(TVType.body)
                .foregroundStyle(TVPalette.textSecondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 760)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 50)
    }

    // MARK: Guide

    private var guide: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 14) {
                Text("THE GUIDE")
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Rectangle().fill(Color.white.opacity(0.1)).frame(height: 1)
            }
            .padding(.horizontal, TVMetrics.margin)

            if let pick = aria.dailyPick, pick.isPresent {
                TVGuideChannel(number: 0, name: "Aria", detail: "Today's pick", systemImage: "sparkles") {
                    TVAriaDailyPickCard(pick: pick)
                        .frame(width: 900)
                }
            }

            recentlyAddedChannel
            playlistsChannel
            discoverChannel
            smartChannel
            onThisDayChannel
        }
    }

    @ViewBuilder
    private var recentlyAddedChannel: some View {
        let rest = Array(recentlyAdded.dropFirst().prefix(16))
        if !rest.isEmpty {
            TVGuideChannel(number: 1, name: "New Arrivals", detail: "Latest uploads", systemImage: "sparkle",
                           seeAll: { TVLibraryView(client: client, token: token) }) {
                let slots = TVGuideSlots(durations: rest.map(\.duration))
                ForEach(Array(rest.enumerated()), id: \.element.id) { i, track in
                    NavigationLink(value: TVPlayContext(queue: libraryQueue, startID: track.id)) {
                        TVProgrammeLabel(
                            airTime: slots.start(i),
                            title: track.displayTitle,
                            subtitle: track.artist.isEmpty ? "Unknown Artist" : track.artist,
                            width: TVGuideSlots.width(seconds: track.duration),
                            art: {
                                TVAuthImage(url: client.userMusicArtworkURL(for: track), token: token) {
                                    TVArtPlaceholder(systemImage: "music.note", iconScale: 0.6)
                                }
                            }
                        )
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .tvTrackActions(client: client, token: token, track: track)
                }
            }
        }
    }

    @ViewBuilder
    private var playlistsChannel: some View {
        if !client.playlists.isEmpty {
            TVGuideChannel(number: 2, name: "Your Playlists", detail: "\(client.playlists.count) on the schedule",
                           systemImage: "music.note.list",
                           seeAll: { TVPlaylistsView(client: client, token: token) }) {
                let lists = Array(client.playlists.prefix(16))
                // A playlist "runs" about 3.5 minutes per song.
                let slots = TVGuideSlots(durations: lists.map { Double($0.tracks.count) * 210 })
                ForEach(Array(lists.enumerated()), id: \.element.id) { i, playlist in
                    NavigationLink {
                        TVPlaylistDetailView(client: client, token: token, playlist: playlist)
                    } label: {
                        TVProgrammeLabel(
                            airTime: slots.start(i),
                            title: playlist.name,
                            subtitle: tvSongCount(playlist.tracks.count),
                            width: TVGuideSlots.width(seconds: Double(playlist.tracks.count) * 210),
                            art: {
                                TVPlaylistArtwork(name: playlist.name,
                                                  artworkURLs: client.artworkURLs(for: playlist, token: token),
                                                  token: token, showsTitle: false)
                            }
                        )
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
        }
    }

    @ViewBuilder
    private var discoverChannel: some View {
        if !client.discoverMix.isEmpty {
            TVGuideChannel(number: 3, name: "Discover Mix", detail: "From artists you play most",
                           systemImage: "dot.radiowaves.left.and.right",
                           seeAll: { TVDiscoverView(client: client, token: token) }) {
                let tracks = Array(client.discoverMix.prefix(16))
                let slots = TVGuideSlots(durations: tracks.map { Double($0.durationSeconds) })
                ForEach(Array(tracks.enumerated()), id: \.element.id) { i, track in
                    NavigationLink(value: TVPlayContext(queue: discoverQueue, startID: track.id)) {
                        trackProgramme(track, airTime: slots.start(i))
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .tvSearchTrackActions(client: client, token: token, track: track)
                }
            }
        }
    }

    @ViewBuilder
    private var smartChannel: some View {
        if !client.smartPlaylists.isEmpty, !client.smartPlaylists.allSatisfy({ $0.tracks.isEmpty }) {
            TVGuideChannel(number: 4, name: "Tempo Mixes", detail: "Built from your library's tempo",
                           systemImage: "metronome.fill",
                           seeAll: { TVDiscoverView(client: client, token: token) }) {
                let buckets = client.smartPlaylists.filter { !$0.tracks.isEmpty }
                let slots = TVGuideSlots(durations: buckets.map { Double($0.tracks.count) * 210 })
                ForEach(Array(buckets.enumerated()), id: \.element.id) { i, bucket in
                    NavigationLink {
                        TVSmartPlaylistDetailView(client: client, token: token, bucket: bucket)
                    } label: {
                        TVProgrammeLabel(
                            airTime: slots.start(i),
                            title: bucket.name,
                            subtitle: tvSongCount(bucket.tracks.count),
                            width: TVGuideSlots.width(seconds: Double(bucket.tracks.count) * 210),
                            art: {
                                TVGeneratedArt(seed: bucket.key, systemImage: tvSmartPlaylistIcon(bucket.key))
                            }
                        )
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
        }
    }

    @ViewBuilder
    private var onThisDayChannel: some View {
        if let group = client.onThisDay.first {
            let queue = group.tracks.compactMap { client.playable(from: $0) }
            TVGuideChannel(number: 5, name: "Rewind", detail: group.yearsAgo == 1 ? "1 year ago today" : "\(group.yearsAgo) years ago today",
                           systemImage: "clock.arrow.circlepath",
                           seeAll: { TVDiscoverView(client: client, token: token) }) {
                let slots = TVGuideSlots(durations: group.tracks.map { Double($0.durationSeconds) })
                ForEach(Array(group.tracks.enumerated()), id: \.element.id) { i, track in
                    NavigationLink(value: TVPlayContext(queue: queue, startID: track.id)) {
                        trackProgramme(track, airTime: slots.start(i))
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .tvSearchTrackActions(client: client, token: token, track: track)
                }
            }
        }
    }

    private func trackProgramme(_ track: TVTrack, airTime: Date) -> some View {
        TVProgrammeLabel(
            airTime: airTime,
            title: track.title,
            subtitle: track.artist.isEmpty ? "Unknown Artist" : track.artist,
            width: TVGuideSlots.width(seconds: Double(track.durationSeconds)),
            art: {
                TVAuthImage(url: URL(string: track.thumbnailURL), token: nil) {
                    TVArtPlaceholder(systemImage: "music.note", iconScale: 0.6)
                }
            }
        )
    }
}

// MARK: - Guide pieces

/// Air times for a channel: each programme starts when the previous one ends,
/// counted from now.
struct TVGuideSlots {
    let starts: [Date]

    init(durations: [Double], from now: Date = Date()) {
        var t = now
        var out: [Date] = []
        for d in durations {
            out.append(t)
            t = t.addingTimeInterval(max(60, d.isFinite ? d : 180))
        }
        starts = out
    }

    func start(_ i: Int) -> Date { starts.indices.contains(i) ? starts[i] : Date() }

    /// A programme's width on the guide: proportional to its length, clamped so
    /// a 40-second interlude is still readable and a 2-hour mix doesn't take
    /// the whole row.
    static func width(seconds: Double) -> CGFloat {
        let minutes = (seconds.isFinite && seconds > 0 ? seconds : 200) / 60
        return min(660, max(320, CGFloat(minutes) * 70))
    }
}

/// One channel row: the channel ident on the left (not focusable — it is a
/// label), the programmes scrolling on the right.
struct TVGuideChannel<Content: View>: View {
    let number: Int
    let name: String
    let detail: String
    let systemImage: String
    var seeAll: (() -> AnyView)? = nil
    @ViewBuilder let content: () -> Content

    @Environment(\.tvTheme) private var theme

    init(number: Int, name: String, detail: String, systemImage: String,
         @ViewBuilder content: @escaping () -> Content) {
        self.number = number
        self.name = name
        self.detail = detail
        self.systemImage = systemImage
        self.content = content
    }

    init<Destination: View>(number: Int, name: String, detail: String, systemImage: String,
                            @ViewBuilder seeAll: @escaping () -> Destination,
                            @ViewBuilder content: @escaping () -> Content) {
        self.number = number
        self.name = name
        self.detail = detail
        self.systemImage = systemImage
        self.seeAll = { AnyView(seeAll()) }
        self.content = content
    }

    var body: some View {
        HStack(alignment: .center, spacing: 0) {
            ident
                .frame(width: 270, alignment: .leading)
                .padding(.leading, TVMetrics.margin)

            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .center, spacing: 12) {
                    content()
                    if let seeAll {
                        NavigationLink {
                            seeAll().tvAmbientBackground()
                        } label: {
                            TVGuideMoreLabel()
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                }
                .padding(.leading, 18)
                .padding(.trailing, TVMetrics.margin + 40)
                .padding(.vertical, 20)
            }
            .padding(.vertical, -20)
        }
    }

    private var ident: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Text(String(format: "CH %02d", number))
                    .font(.system(size: 15, weight: .black, design: .monospaced))
                    .foregroundStyle(theme.ground)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(RoundedRectangle(cornerRadius: 5).fill(TVPalette.brand))
                Image(systemName: systemImage)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(TVPalette.neon)
            }
            Text(name)
                .font(theme.display(28, .bold))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Text(detail)
                .font(TVType.meta)
                .foregroundStyle(TVPalette.textTertiary)
                .lineLimit(1)
        }
    }
}

/// A programme block: air time, a thumbnail, title and subtitle. Wide blocks
/// are long programmes.
struct TVProgrammeLabel<Art: View>: View {
    let airTime: Date
    let title: String
    let subtitle: String
    let width: CGFloat
    @ViewBuilder let art: () -> Art

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    var body: some View {
        HStack(spacing: 18) {
            art()
                .frame(width: 112, height: 112)
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                .shadow(color: .black.opacity(0.4), radius: 8, y: 4)

            VStack(alignment: .leading, spacing: 6) {
                Text(airTime, format: .dateTime.hour().minute())
                    .font(.system(size: 16, weight: .heavy, design: .monospaced))
                    .foregroundStyle(isFocused ? TVPalette.neon : TVPalette.textTertiary)
                Text(title)
                    .font(.system(size: 23, weight: .bold))
                    .foregroundStyle(.white)
                    .lineLimit(2)
                Text(subtitle)
                    .font(TVType.rowDetail)
                    .foregroundStyle(TVPalette.textSecondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(14)
        .frame(width: width, height: 140, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(isFocused ? theme.raised.opacity(0.98) : theme.surface.opacity(0.55))
        }
        .overlay(alignment: .leading) {
            // The programme's edge in the guide grid.
            RoundedRectangle(cornerRadius: 2)
                .fill(TVPalette.brandVertical)
                .frame(width: 4)
                .padding(.vertical, 18)
                .opacity(isFocused ? 1 : 0.35)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .strokeBorder(isFocused ? AnyShapeStyle(TVPalette.brand) : AnyShapeStyle(Color.white.opacity(0.06)),
                              lineWidth: isFocused ? 2 : 1)
        }
        .shadow(color: theme.primary.opacity(isFocused ? 0.45 : 0), radius: 26, y: 10)
        .scaleEffect(isFocused ? 1.04 : 1)
        .animation(.spring(response: 0.3, dampingFraction: 0.8), value: isFocused)
    }
}

/// "Full listings ›" at the end of a channel.
private struct TVGuideMoreLabel: View {
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "list.bullet.rectangle")
                .font(.system(size: 30, weight: .semibold))
            Text("Full listings")
                .font(.system(size: 18, weight: .semibold))
        }
        .foregroundStyle(isFocused ? Color.black : TVPalette.textSecondary)
        .frame(width: 170, height: 140)
        .background {
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(isFocused ? Color.white : Color.white.opacity(0.05))
        }
        .scaleEffect(isFocused ? 1.05 : 1)
        .animation(.spring(response: 0.28, dampingFraction: 0.8), value: isFocused)
    }
}

/// The wide feature at the top of Home: the artwork as a broadcast still on
/// the left (with scanline texture), the listing on the right.
struct TVNowShowingCard<Art: View, Actions: View>: View {
    let eyebrow: String
    let title: String
    let subtitle: String
    var meta: String = ""
    @ViewBuilder let art: () -> Art
    @ViewBuilder let actions: () -> Actions

    @Environment(\.tvTheme) private var theme

    var body: some View {
        HStack(spacing: 50) {
            ZStack {
                art()
                    .frame(width: 600, height: 340)
                    .clipped()
                TVScanlines()
                    .opacity(0.35)
                // LIVE bug in the corner, like a channel's on-screen graphic.
                HStack(spacing: 8) {
                    Circle().fill(Color(red: 1, green: 0.25, blue: 0.3)).frame(width: 10, height: 10)
                    Text("LIVE")
                        .font(.system(size: 15, weight: .black, design: .monospaced))
                }
                .foregroundStyle(.white)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(Capsule().fill(Color.black.opacity(0.55)))
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(18)
            }
            .frame(width: 600, height: 340)
            .clipShape(RoundedRectangle(cornerRadius: 26, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 26, style: .continuous)
                    .strokeBorder(Color.white.opacity(0.12), lineWidth: 1)
            }
            .shadow(color: theme.primary.opacity(0.35), radius: 40, y: 20)

            VStack(alignment: .leading, spacing: 12) {
                Text(eyebrow)
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text(title)
                    .font(theme.display(54))
                    .lineLimit(2)
                    .minimumScaleFactor(0.6)
                Text(subtitle)
                    .font(.system(size: 28, weight: .medium))
                    .foregroundStyle(TVPalette.textSecondary)
                    .lineLimit(1)
                if !meta.isEmpty {
                    Text(meta)
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textTertiary)
                }
                HStack(spacing: 20) { actions() }
                    .padding(.top, 14)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, TVMetrics.margin)
    }
}

/// Horizontal scanlines, drawn once (no inputs): the broadcast texture.
struct TVScanlines: View, Equatable {
    var body: some View {
        Canvas { ctx, size in
            var y: CGFloat = 0
            while y < size.height {
                ctx.fill(Path(CGRect(x: 0, y: y, width: size.width, height: 1.5)), with: .color(.black.opacity(0.4)))
                y += 5
            }
        }
        .allowsHitTesting(false)
    }
}

/// SMPTE-style colour bars — the "off air" test card.
struct TVTestCardBars: View {
    private static let colors: [Color] = [
        Color(white: 0.78), Color(red: 0.78, green: 0.78, blue: 0), Color(red: 0, green: 0.78, blue: 0.78),
        Color(red: 0, green: 0.78, blue: 0), Color(red: 0.78, green: 0, blue: 0.78), Color(red: 0.78, green: 0, blue: 0),
        Color(red: 0, green: 0, blue: 0.78),
    ]

    var body: some View {
        HStack(spacing: 0) {
            ForEach(0..<Self.colors.count, id: \.self) { i in
                Self.colors[i]
            }
        }
    }
}
