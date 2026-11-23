import SwiftUI

// MARK: - TVLibraryView (per-user cloud library)

struct TVLibraryView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    @Environment(\.tvTheme) private var theme

    private enum Mode: String, CaseIterable, Identifiable {
        case songs = "Songs"
        case albums = "Albums"
        case artists = "Artists"
        case genres = "Genres"
        case favorites = "Favorites"
        var id: String { rawValue }

        var systemImage: String {
            switch self {
            case .songs: return "music.note"
            case .albums: return "square.stack"
            case .artists: return "music.mic"
            case .genres: return "guitars"
            case .favorites: return "star.fill"
            }
        }

        /// The letter printed on the crate divider's tab.
        var tab: String {
            switch self {
            case .songs: return "S"
            case .albums: return "A"
            case .artists: return "R"
            case .genres: return "G"
            case .favorites: return "★"
            }
        }
    }
    @State private var mode: Mode = .songs
    @State private var searchText = ""

    /// Local, in-memory filter over the already-loaded library — cheaper and
    /// far more responsive than re-hitting `/user/music` per keystroke,
    /// which does a full filesystem walk + ffprobe pass on every request.
    /// Only the Songs tab is filtered by it; Albums/Artists/Genres still
    /// group the whole library so browsing by those dimensions isn't
    /// truncated by an unrelated in-progress search.
    ///
    /// Cached rather than a plain computed property: with a several-
    /// thousand-track library this `.filter`/`.compactMap` pair (`queue`
    /// chains directly off it) re-ran on EVERY `body` evaluation — not just
    /// per keystroke, but on every unrelated re-render too (mode switches,
    /// favorite toggles, `client` publishing anything) — since a computed
    /// property has no memory of whether its inputs actually changed. Same
    /// class of fix as `TVAlbumsGridView.cachedAlbums`.
    @State private var filteredSongs: [UserMusicTrack] = []
    @State private var queue: [TVPlayable] = []
    @State private var filterDebounceTask: Task<Void, Never>?

    private func recomputeFilteredSongs() {
        let q = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let library = client.library
        let filtered = q.isEmpty ? library : library.filter {
            $0.title.lowercased().contains(q) || $0.artist.lowercased().contains(q) || $0.album.lowercased().contains(q)
        }
        filteredSongs = filtered
        queue = filtered.compactMap { client.playable(from: $0, token: token) }
    }

    /// Debounced for `searchText` (fires once per pause in typing, not once
    /// per keystroke); immediate for the library actually loading/changing.
    private func scheduleFilterRecompute() {
        filterDebounceTask?.cancel()
        filterDebounceTask = Task {
            try? await Task.sleep(nanoseconds: 250_000_000)
            guard !Task.isCancelled else { return }
            recomputeFilteredSongs()
        }
    }

    /// Count line beside the screen title — the only place the library's size
    /// is stated, and a cheap sanity check that a sync actually happened.
    private var libraryDetail: String? {
        guard !client.library.isEmpty else { return nil }
        let n = client.library.count
        let shown = filteredSongs.count
        if shown != n && !searchText.isEmpty { return "\(shown) of \(n) tracks" }
        return "\(n) track\(n == 1 ? "" : "s")"
    }

    // MARK: The crate
    //
    // Library is a record crate. Down the left, the crate's dividers — tall
    // tabs for each way of browsing (and Search, which is a pushed screen of
    // its own so the system keyboard gets the whole width). On the right, the
    // records: a song list, a flip-through crate of album sleeves, a wall of
    // artist portraits, a board of genre stickers.
    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            dividers
                .frame(width: 300)
                .focusSection()

            Group {
                if client.isLoadingLibrary {
                    TVLoadingState(text: "Flipping through your records…")
                } else if let err = client.libraryError {
                    TVEmptyState(systemImage: "exclamationmark.icloud",
                                 title: "Couldn't load your library",
                                 message: err) {
                        Button {
                            Task { await client.fetchLibrary(token: token) }
                        } label: {
                            TVPillLabel(title: "Try Again", systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                } else if client.library.isEmpty {
                    TVEmptyState(systemImage: "shippingbox",
                                 title: "The crate is empty",
                                 message: "Add music to your Personal Cloud Library from the iPhone app and it will show up here.")
                } else {
                    switch mode {
                    case .songs: songsList
                    case .albums: TVAlbumsGridView(client: client, token: token, library: client.library)
                    case .artists: TVArtistsGridView(client: client, token: token, library: client.library)
                    case .genres: TVGenresGridView(client: client, token: token, library: client.library)
                    case .favorites: TVFavoritesGridView(client: client, token: token)
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .focusSection()
        }
        // NO `.searchable` here. On tvOS that modifier does not render a compact
        // field — it lays out a full search surface, prompt plus a letter picker
        // spanning the whole width, as a sibling of this view's own content. In
        // the old single-column shell it had the screen to itself and looked
        // fine. Inside the three-column shell it drew across the content column
        // AND over the player column beside it, which is the alphabet strip
        // running over everything in the 1.9.0 screenshot.
        //
        // Search now lives behind the first crate divider and is presented as
        // its own pushed screen (`librarySearchScreen`), which is the platform's
        // actual pattern for tvOS search and gets dictation for free.
        .task {
            if client.library.isEmpty { await client.fetchLibrary(token: token) }
            if client.favoriteSongIDs.isEmpty { await client.fetchFavorites(token: token) }
            recomputeFilteredSongs()
        }
        .onChange(of: searchText) { _ in scheduleFilterRecompute() }
        .onChange(of: client.library.count) { _ in recomputeFilteredSongs() }
    }

    /// The crate dividers: the library's size on the label up top, then a tab
    /// per browse mode, Search first (it is a different kind of action).
    private var dividers: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text("THE CRATE")
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text("Library")
                    .font(theme.display(46))
                if let libraryDetail {
                    Text(libraryDetail)
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textTertiary)
                }
            }
            .padding(.bottom, 18)

            NavigationLink {
                librarySearchScreen
            } label: {
                TVCrateDividerLabel(tab: "⌕", title: searchText.isEmpty ? "Search" : searchText,
                                    systemImage: "magnifyingglass", isSelected: !searchText.isEmpty)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()

            ForEach(Mode.allCases) { m in
                Button {
                    mode = m
                } label: {
                    TVCrateDividerLabel(tab: m.tab, title: m.rawValue, systemImage: m.systemImage, isSelected: mode == m)
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
            }
            Spacer(minLength: 0)
        }
        .padding(.leading, TVMetrics.margin)
        .padding(.top, 20)
    }

    /// Songs as a dense list rather than a grid of covers — see `TVTrackRow`
    /// for why. `.buttonStyle(.plain)`, not `.card`: the system card style
    /// applies its own lift-and-shadow treatment sized for square artwork,
    /// which on a full-width row reads as the whole screen jumping. The row
    /// draws its own focus state instead.
    private var songsList: some View {
        ScrollView {
            if filteredSongs.isEmpty {
                TVEmptyState(systemImage: "magnifyingglass",
                             title: "No matches",
                             message: "No songs match “\(searchText)”.")
            } else {
                TVTrackList {
                    ForEach(filteredSongs) { track in
                        trackRow(track)
                    }
                }
                .padding(.top, 14)
                .padding(.bottom, 80)
            }
        }
    }

    /// One library row. Shared by the browse list and the search screen so the
    /// two cannot drift apart in styling or in what a select actually does.
    private func trackRow(_ track: UserMusicTrack) -> some View {
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

    /// Library search as its own pushed screen, built on the same
    /// `UISearchController` wrapper the Search tab uses — so it gets the system
    /// tvOS keyboard, and with it dictation, which `.searchable` has no
    /// supported way to offer.
    ///
    /// Pushed rather than inline: a search surface on tvOS wants the whole
    /// screen for its keyboard, and giving it one is what stops it drawing over
    /// the neighbouring columns.
    private var librarySearchScreen: some View {
        TVDictationSearch(
            text: $searchText,
            placeholder: "Search your library",
            onChange: { _ in scheduleFilterRecompute() }
        ) {
            ScrollView {
                if filteredSongs.isEmpty {
                    TVEmptyState(
                        systemImage: "magnifyingglass",
                        title: searchText.isEmpty ? "Search your library" : "No matches",
                        message: searchText.isEmpty
                            ? "Find songs by title, artist or album."
                            : "No songs match “\(searchText)”."
                    )
                } else {
                    TVTrackList {
                        ForEach(filteredSongs) { track in
                            trackRow(track)
                        }
                    }
                    .padding(.vertical, 30)
                }
            }
            // A pushed destination replaces the shell, so it draws its own
            // backdrop — unlike the root tab screens, which sit inside it.
            .tvAmbientBackground()
        }
        .ignoresSafeArea()
    }
}

// MARK: - Crate divider

/// A crate divider: a tab with a letter sticking out on the left, the browse
/// mode's name on the card. Selected dividers stand proud in the theme's
/// light; focus lifts and whitens the tab.
struct TVCrateDividerLabel: View {
    let tab: String
    let title: String
    let systemImage: String
    let isSelected: Bool

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    var body: some View {
        HStack(spacing: 0) {
            Text(tab)
                .font(theme.display(26, .black))
                .foregroundStyle(isFocused ? Color.black : (isSelected ? theme.ground : Color.white.opacity(0.7)))
                .frame(width: 54, height: 72)
                .background {
                    UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 16,
                                           bottomTrailingRadius: 4, topTrailingRadius: 4, style: .continuous)
                        .fill(isFocused ? AnyShapeStyle(Color.white)
                              : isSelected ? AnyShapeStyle(TVPalette.brandVertical)
                              : AnyShapeStyle(Color.white.opacity(0.1)))
                }
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.system(size: 18, weight: .semibold))
                Text(title)
                    .font(.system(size: 22, weight: isSelected ? .bold : .semibold))
                    .lineLimit(1)
            }
            .foregroundStyle(isSelected || isFocused ? Color.white : Color.white.opacity(0.6))
            .padding(.horizontal, 16)
            .frame(width: 176, height: 72, alignment: .leading)
            .background {
                UnevenRoundedRectangle(topLeadingRadius: 4, bottomLeadingRadius: 4,
                                       bottomTrailingRadius: 16, topTrailingRadius: 16, style: .continuous)
                    .fill(isFocused ? theme.raised : (isSelected ? theme.surface : theme.surface.opacity(0.35)))
            }
        }
        .shadow(color: (isSelected || isFocused) ? theme.primary.opacity(0.35) : .clear, radius: 18, y: 6)
        .offset(x: isSelected ? 10 : 0)
        .scaleEffect(isFocused ? 1.05 : 1, anchor: .leading)
        .animation(.spring(response: 0.3, dampingFraction: 0.8), value: isFocused)
        .animation(.spring(response: 0.35, dampingFraction: 0.8), value: isSelected)
    }
}
