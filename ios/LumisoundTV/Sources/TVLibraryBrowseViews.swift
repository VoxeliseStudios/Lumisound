import SwiftUI

// MARK: - Grouping (mirrors LibraryManager's album/artist derivation on iOS:
// group by the metadata field verbatim — case-insensitive alphabetical order
// — falling back to "Unknown Album"/"Unknown Artist" for tracks with no tag.
// Note: the bridge itself already falls back to the containing folder name
// for `album` when a file has no album tag (see `/user/music` in
// ios-bridge/main.py), so grouping by `album` here also surfaces
// folder-organized uploads without needing a separate "folder" tab.)

struct TVAlbumGroup: Identifiable, Hashable {
    let name: String
    let tracks: [UserMusicTrack]  // always non-empty — built via Dictionary(grouping:)
    var id: String { name }
    var representativeTrack: UserMusicTrack { tracks[0] }
    var artistName: String {
        let a = tracks[0].artist
        return a.isEmpty ? "Unknown Artist" : a
    }
}

struct TVArtistGroup: Identifiable, Hashable {
    let name: String
    let tracks: [UserMusicTrack]
    var id: String { name }
    var albumCount: Int { Set(tracks.map { $0.album.isEmpty ? "Unknown Album" : $0.album }).count }
}

struct TVGenreGroup: Identifiable, Hashable {
    let name: String
    let tracks: [UserMusicTrack]
    var id: String { name }
}

/// Track order within an album: by track number (untagged tracks sort last),
/// then title — matches `AlbumDetailView`'s ordering on iOS.
private func albumSortKey(_ t: UserMusicTrack) -> (Int, String) {
    (Int(t.trackNumber) ?? Int.max, t.displayTitle)
}

func tvAlbumGroups(from library: [UserMusicTrack]) -> [TVAlbumGroup] {
    let groups = Dictionary(grouping: library) { $0.album.isEmpty ? "Unknown Album" : $0.album }
    return groups.keys
        .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
        .map { name in
            TVAlbumGroup(name: name, tracks: (groups[name] ?? []).sorted {
                let (n0, t0) = albumSortKey($0), (n1, t1) = albumSortKey($1)
                return n0 != n1 ? n0 < n1 : t0.localizedCaseInsensitiveCompare(t1) == .orderedAscending
            })
        }
}

func tvArtistGroups(from library: [UserMusicTrack]) -> [TVArtistGroup] {
    let groups = Dictionary(grouping: library) { $0.artist.isEmpty ? "Unknown Artist" : $0.artist }
    return groups.keys
        .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
        .map { name in
            TVArtistGroup(name: name, tracks: (groups[name] ?? []).sorted {
                $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
            })
        }
}

/// Tracks with no genre tag are omitted entirely (rather than grouped under
/// "Unknown Genre") — unlike album/artist, most uploads simply won't have a
/// genre tag, so an "Unknown Genre" bucket would just become a second,
/// noisier copy of the Songs tab instead of a useful browse dimension.
func tvGenreGroups(from library: [UserMusicTrack]) -> [TVGenreGroup] {
    let tagged = library.filter { !$0.genre.trimmingCharacters(in: .whitespaces).isEmpty }
    let groups = Dictionary(grouping: tagged) { $0.genre.trimmingCharacters(in: .whitespaces) }
    return groups.keys
        .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
        .map { name in
            TVGenreGroup(name: name, tracks: (groups[name] ?? []).sorted {
                $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
            })
        }
}

// MARK: - Albums: the crate

/// Albums as a crate of records you flip through: sleeves stand tilted back
/// in a row, the focused one rises upright and its liner notes (artist, length,
/// the first tracks) appear underneath. Select opens the gatefold.
struct TVAlbumsGridView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let library: [UserMusicTrack]

    @Environment(\.tvTheme) private var theme

    // `tvAlbumGroups` is a `Dictionary(grouping:)` + sort over the WHOLE
    // library; cached in `.task(id:)` so it isn't redone on every body
    // evaluation (any `client` publish re-evaluates this view).
    @State private var cachedAlbums: [TVAlbumGroup] = []
    @FocusState private var focusedAlbum: String?
    /// The last album that had focus — the liner notes stay put while focus
    /// visits the dividers.
    @State private var linerAlbum: TVAlbumGroup?

    var body: some View {
        Group {
            if cachedAlbums.isEmpty {
                TVLoadingState(text: "Gathering albums…")
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    crate
                    linerNotes
                    Spacer(minLength: 0)
                }
            }
        }
        .task(id: library.count) {
            let library = library
            cachedAlbums = await Task.detached(priority: .userInitiated) { tvAlbumGroups(from: library) }.value
            if linerAlbum == nil { linerAlbum = cachedAlbums.first }
        }
        .onChange(of: focusedAlbum) { id in
            if let id, let album = cachedAlbums.first(where: { $0.id == id }) { linerAlbum = album }
        }
    }

    private var crate: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            LazyHStack(alignment: .bottom, spacing: -92) {
                ForEach(cachedAlbums) { album in
                    NavigationLink {
                        TVAlbumDetailView(client: client, token: token, album: album)
                    } label: {
                        TVCrateSleeveLabel(
                            name: album.name,
                            artURL: client.userMusicArtworkURL(for: album.representativeTrack),
                            token: token
                        )
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .focused($focusedAlbum, equals: album.id)
                    // The focused sleeve is pulled up out of the crate, in
                    // front of its neighbours.
                    .zIndex(focusedAlbum == album.id ? 10 : 0)
                }
            }
            .padding(.leading, 40)
            .padding(.trailing, TVMetrics.margin + 200)
            .padding(.vertical, 50)
        }
        .frame(height: 470)
    }

    @ViewBuilder
    private var linerNotes: some View {
        if let album = linerAlbum {
            HStack(alignment: .top, spacing: 50) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("LINER NOTES")
                        .font(TVType.eyebrow)
                        .tracking(theme.tracking)
                        .foregroundStyle(TVPalette.neonAlt)
                    Text(album.name)
                        .font(theme.display(40))
                        .lineLimit(2)
                        .minimumScaleFactor(0.6)
                    Text(album.artistName)
                        .font(.system(size: 24, weight: .medium))
                        .foregroundStyle(TVPalette.textSecondary)
                    Text(tvCollectionMeta(album.tracks))
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textTertiary)
                }
                .frame(width: 520, alignment: .leading)

                // The side's running order, two columns, like the back of a sleeve.
                let shown = Array(album.tracks.prefix(10))
                LazyVGrid(columns: [GridItem(.flexible(), alignment: .leading), GridItem(.flexible(), alignment: .leading)],
                          alignment: .leading, spacing: 10) {
                    ForEach(Array(shown.enumerated()), id: \.element.id) { i, track in
                        HStack(spacing: 12) {
                            Text("\(i + 1)")
                                .font(.system(size: 17, weight: .bold, design: .monospaced))
                                .foregroundStyle(TVPalette.neon)
                                .frame(width: 30, alignment: .trailing)
                            Text(track.displayTitle)
                                .font(.system(size: 19))
                                .foregroundStyle(.white.opacity(0.8))
                                .lineLimit(1)
                        }
                    }
                }
            }
            .padding(.horizontal, 40)
            .padding(.trailing, TVMetrics.margin)
            .id(album.id)
            .transition(.opacity)
            .animation(.easeOut(duration: 0.25), value: album.id)
        }
    }
}

/// A record sleeve in the crate. At rest it leans back (rotated around its
/// vertical axis, darkened); focused it stands upright, larger, with the disc
/// peeking out of the top.
private struct TVCrateSleeveLabel: View {
    let name: String
    let artURL: URL?
    let token: String

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    var body: some View {
        ZStack(alignment: .top) {
            // The record, sliding up out of the sleeve on focus.
            TVVinylDisc(diameter: 290)
                .offset(y: isFocused ? -70 : 0)
                .opacity(isFocused ? 1 : 0)

            TVAuthImage(url: artURL, token: token) {
                TVGeneratedArt(seed: name, systemImage: "square.stack", title: name)
            }
            .frame(width: 300, height: 300)
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .strokeBorder(isFocused ? AnyShapeStyle(TVPalette.brand) : AnyShapeStyle(Color.white.opacity(0.12)),
                                  lineWidth: isFocused ? 3 : 1)
            }
            .overlay {
                // Shade on the leaning sleeves, so the upright one is the lit one.
                Color.black.opacity(isFocused ? 0 : 0.35)
            }
            .shadow(color: isFocused ? theme.primary.opacity(0.5) : .black.opacity(0.6),
                    radius: isFocused ? 34 : 14, x: isFocused ? 0 : -10, y: 14)
        }
        .frame(width: 300, height: 300)
        .rotation3DEffect(.degrees(isFocused ? 0 : 34), axis: (x: 0, y: 1, z: 0),
                          anchor: .leading, perspective: 0.55)
        .scaleEffect(isFocused ? 1.12 : 0.94, anchor: .bottom)
        .animation(.spring(response: 0.36, dampingFraction: 0.78), value: isFocused)
    }
}

/// A vinyl record: grooves, a light sheen, the label in the theme's lights.
struct TVVinylDisc: View, Equatable {
    var diameter: CGFloat = 300

    var body: some View {
        ZStack {
            Circle().fill(Color(white: 0.05))
            ForEach(0..<9, id: \.self) { i in
                Circle()
                    .strokeBorder(Color.white.opacity(0.05), lineWidth: 1)
                    .padding(diameter * (0.06 + CGFloat(i) * 0.035))
            }
            AngularGradient(colors: [.white.opacity(0.0), .white.opacity(0.12), .white.opacity(0.0),
                                     .white.opacity(0.1), .white.opacity(0.0)], center: .center)
                .clipShape(Circle())
            Circle()
                .fill(TVPalette.brand)
                .frame(width: diameter * 0.34, height: diameter * 0.34)
            Circle()
                .fill(Color.black)
                .frame(width: diameter * 0.04, height: diameter * 0.04)
        }
        .frame(width: diameter, height: diameter)
        .shadow(color: .black.opacity(0.6), radius: 16, y: 8)
    }
}

// MARK: - Album detail

struct TVAlbumDetailView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let album: TVAlbumGroup

    private var queue: [TVPlayable] {
        album.tracks.compactMap { client.playable(from: $0, token: token) }
    }

    var body: some View {
        TVGatefold(
            eyebrow: "Album",
            title: album.name,
            subtitle: album.artistName,
            meta: tvCollectionMeta(album.tracks)
        ) {
            TVAuthImage(url: client.userMusicArtworkURL(for: album.representativeTrack), token: token) {
                TVGeneratedArt(seed: album.name, systemImage: "square.stack")
            }
        } actions: {
            if let first = queue.first {
                NavigationLink(value: TVPlayContext(queue: queue, startID: first.id)) {
                    TVPillLabel(title: "Drop the Needle", systemImage: "play.fill")
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
            }
        } tracks: {
            TVTrackList {
                ForEach(Array(album.tracks.enumerated()), id: \.element.id) { index, track in
                    NavigationLink(value: TVPlayContext(queue: queue, startID: track.id)) {
                        // Numbered rather than showing the album's own
                        // cover once per track — see TVTrackRow.trackNumber.
                        TVTrackRow(
                            artworkURL: nil,
                            token: token,
                            title: track.displayTitle,
                            artist: track.artist,
                            detail: track.duration.tvDurationText,
                            isFavorite: client.isFavorite(track.id),
                            trackNumber: index + 1
                        )
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .tvTrackActions(client: client, token: token, track: track)
                }
            }
        }
        .background(TVCollectionBackdrop(url: client.userMusicArtworkURL(for: album.representativeTrack), token: token))
    }
}

// MARK: - Artists grid

struct TVArtistsGridView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let library: [UserMusicTrack]

    private var columns: [GridItem] { TVGridLayout.columns() }

    /// See `TVAlbumsGridView.cachedAlbums` — same fix, same reason.
    @State private var cachedArtists: [TVArtistGroup] = []

    var body: some View {
        Group {
            if cachedArtists.isEmpty {
                TVLoadingState(text: "Gathering artists…")
            } else {
                TVCardGrid(items: cachedArtists) { artist, cell in
                    NavigationLink {
                        TVArtistDetailView(client: client, token: token, artist: artist)
                    } label: {
                        artistCard(artist, width: cell)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
        }
        .task(id: library.count) {
            let library = library
            cachedArtists = await Task.detached(priority: .userInitiated) { tvArtistGroups(from: library) }.value
        }
    }

    private func artistCard(_ artist: TVArtistGroup, width: CGFloat) -> some View {
        // An artist has no artwork of its own server-side, so the first of
        // its tracks that HAS a cover stands in — the same substitution
        // every music app makes. Circular, so artists never read as albums.
        TVArtworkCardLabel(
            title: artist.name,
            subtitle: "\(artist.albumCount) \(artist.albumCount == 1 ? "album" : "albums") · \(tvSongCount(artist.tracks.count))",
            width: width,
            isCircle: true
        ) {
            TVAuthImage(url: tvArtistArtworkURL(artist, client: client), token: token) {
                TVGeneratedArt(seed: artist.name, systemImage: "music.mic")
            }
        }
    }
}

/// First track with stored artwork, or nil when none of them have any.
@MainActor
func tvArtistArtworkURL(_ artist: TVArtistGroup, client: TVBridgeClient) -> URL? {
    guard let track = artist.tracks.first(where: { $0.hasArtwork }) else { return nil }
    return client.userMusicArtworkURL(for: track)
}

// MARK: - Artist detail (grouped by album, like ArtistDetailView on iOS)

struct TVArtistDetailView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let artist: TVArtistGroup

    var body: some View {
        let albums = tvAlbumGroups(from: artist.tracks)
        // The header sits ABOVE the grid rather than inside it: TVCardGrid owns
        // its own ScrollView (it has to measure the available width), and
        // nesting one scroll view inside another gives two competing scroll
        // areas and a focus path that can fall between them.
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 34) {
                TVAuthImage(url: tvArtistArtworkURL(artist, client: client), token: token) {
                    TVGeneratedArt(seed: artist.name, systemImage: "music.mic")
                }
                .frame(width: 150, height: 150)
                .clipShape(Circle())
                .shadow(color: .black.opacity(0.5), radius: 24, y: 12)

                VStack(alignment: .leading, spacing: 8) {
                    Text("ARTIST")
                        .font(TVType.eyebrow)
                        .tracking(2.4)
                        .foregroundStyle(TVPalette.neonAlt)
                    Text(artist.name)
                        .font(TVType.display)
                        .lineLimit(1)
                        .minimumScaleFactor(0.6)
                    Text("\(albums.count) \(albums.count == 1 ? "album" : "albums") · \(tvSongCount(artist.tracks.count))")
                        .font(TVType.meta)
                        .foregroundStyle(TVPalette.textTertiary)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, TVMetrics.margin)
            .padding(.top, 44)

            TVCardGrid(items: albums) { album, cell in
                NavigationLink {
                    TVAlbumDetailView(client: client, token: token, album: album)
                } label: {
                    TVArtworkCardLabel(title: album.name, subtitle: tvSongCount(album.tracks.count), width: cell) {
                        TVAuthImage(url: client.userMusicArtworkURL(for: album.representativeTrack), token: token) {
                            TVGeneratedArt(seed: album.name, systemImage: "square.stack")
                        }
                    }
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
            }
        }
        .background(TVCollectionBackdrop(url: tvArtistArtworkURL(artist, client: client), token: token))
    }
}

// MARK: - Genres grid

struct TVGenresGridView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let library: [UserMusicTrack]

    private var columns: [GridItem] { TVGridLayout.columns() }

    /// See `TVAlbumsGridView.cachedAlbums` — same fix, same reason.
    @State private var cachedGenres: [TVGenreGroup] = []

    var body: some View {
        Group {
            if cachedGenres.isEmpty {
                TVEmptyState(systemImage: "guitars",
                             title: "No genres yet",
                             message: "Songs appear here once they carry a genre tag.")
            } else {
                // A board of stickers: each genre slapped on at its own slight
                // angle (stable per name), straightened when focused.
                TVCardGrid(items: cachedGenres) { genre, cell in
                    NavigationLink {
                        TVGenreDetailView(client: client, token: token, genre: genre)
                    } label: {
                        TVStickerLabel(title: genre.name, subtitle: tvSongCount(genre.tracks.count), width: cell)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
        }
        .task(id: library.count) {
            let library = library
            cachedGenres = await Task.detached(priority: .userInitiated) { tvGenreGroups(from: library) }.value
        }
    }

}

// MARK: - Genre detail

struct TVGenreDetailView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let genre: TVGenreGroup

    private var queue: [TVPlayable] {
        genre.tracks.compactMap { client.playable(from: $0, token: token) }
    }

    var body: some View {
        TVGatefold(
            eyebrow: "Genre",
            title: genre.name,
            meta: tvCollectionMeta(genre.tracks)
        ) {
            TVGeneratedArt(seed: genre.name, systemImage: "guitars")
        } actions: {
            if let first = queue.first {
                NavigationLink(value: TVPlayContext(queue: queue, startID: first.id)) {
                    TVPillLabel(title: "Play", systemImage: "play.fill")
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
            }
        } tracks: {
            TVTrackList {
                ForEach(genre.tracks) { track in
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
        .tvAmbientBackground()
    }
}

// MARK: - Favorites grid

struct TVFavoritesGridView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String

    /// Cached rather than a plain computed `var` — `client.library` can be
    /// up to the 100,000-track fetch limit, and a computed property here
    /// re-filtered the WHOLE thing from scratch on every read, including
    /// reads triggered by ANY of `client`'s ~20 other `@Published`
    /// properties changing while this screen happens to be visible (not
    /// just a `favoriteSongIDs`/`library` change). Same fix shape as
    /// `TVAlbumsGridView`/`TVArtistsGridView`/`TVGenresGridView`'s
    /// `cachedAlbums` etc. — recomputed only when the inputs that actually
    /// matter change, via `.task(id:)` below.
    @State private var favoriteTracks: [UserMusicTrack] = []
    @State private var queue: [TVPlayable] = []

    var body: some View {
        ScrollView {
            if client.isLoadingFavorites && client.favoriteSongIDs.isEmpty {
                TVLoadingState(text: "Loading favorites…")
            } else if favoriteTracks.isEmpty {
                TVEmptyState(systemImage: "star",
                             title: "No favorites yet",
                             message: "Press and hold Select on any song to add it to your favorites.")
            } else {
                // Rows, matching the Songs list — favourites are songs. Every
                // row here is a favourite by definition, so the star is left
                // off: a badge on every item distinguishes nothing.
                TVTrackList {
                    ForEach(favoriteTracks) { track in
                        NavigationLink(value: TVPlayContext(queue: queue, startID: track.id)) {
                            TVTrackRow(
                                artworkURL: client.userMusicArtworkURL(for: track),
                                token: token,
                                title: track.displayTitle,
                                artist: track.artist,
                                detail: track.duration.tvDurationText
                            )
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                        .tvTrackActions(client: client, token: token, track: track)
                    }
                }
                .padding(.top, 14)
                .padding(.bottom, 80)
            }
        }
        .task {
            if client.favoriteSongIDs.isEmpty { await client.fetchFavorites(token: token) }
        }
        // Keyed on both counts — either the library or the favorites set
        // changing is exactly when this needs to recompute, and nothing
        // else does.
        .task(id: "\(client.library.count)-\(client.favoriteSongIDs.count)") {
            let library = client.library
            let favoriteIDs = client.favoriteSongIDs
            // Only the filter itself (the part that scales with the whole
            // library) is detached — `client.playable(from:)` is MainActor-
            // isolated (it reads `client`'s own state), so that mapping
            // happens back here after the detached work returns.
            let resolved = await Task.detached(priority: .userInitiated) {
                library.filter { favoriteIDs.contains($0.id) }
            }.value
            favoriteTracks = resolved
            queue = resolved.compactMap { client.playable(from: $0, token: token) }
        }
    }
}

// MARK: - Shared detail helpers

/// "12 songs · 48 min" for a set of library tracks.
func tvCollectionMeta(_ tracks: [UserMusicTrack]) -> String {
    let seconds = tracks.reduce(0) { $0 + max(0, $1.duration) }
    let minutes = Int((seconds / 60).rounded())
    let length = minutes >= 60 ? "\(minutes / 60) hr \(minutes % 60) min" : "\(minutes) min"
    return minutes > 0 ? "\(tvSongCount(tracks.count)) · \(length)" : tvSongCount(tracks.count)
}

/// A collection detail screen's backdrop: the collection's OWN artwork, washed
/// out over the brand floor — so opening an album takes on that album's colour
/// rather than whatever happens to be playing.
struct TVCollectionBackdrop: View {
    let url: URL?
    let token: String?

    var body: some View {
        ZStack {
            TVAmbientBackground()
            if url != nil {
                TVAuthImage(url: url, token: token) { Color.clear }
                    .scaleEffect(1.4)
                    .blur(radius: 110, opaque: false)
                    .saturation(1.4)
                    .overlay(TVPalette.ground.opacity(0.62))
                    .mask(
                        LinearGradient(colors: [.black, .black.opacity(0.4), .clear],
                                       startPoint: .top, endPoint: .bottom)
                    )
            }
        }
        .ignoresSafeArea()
    }
}

// MARK: - Genre sticker

/// A genre as a sticker: die-cut rounded square of generated art with a white
/// border, tilted a few degrees (stable per name), straightened on focus.
private struct TVStickerLabel: View {
    let title: String
    let subtitle: String
    let width: CGFloat

    @Environment(\.isFocused) private var isFocused

    /// -4…4 degrees, from the name (stable across launches).
    private var tilt: Double {
        var h: UInt32 = 2_166_136_261
        for s in title.unicodeScalars { h = (h ^ s.value) &* 16_777_619 }
        return Double(Int(h % 9) - 4)
    }

    var body: some View {
        VStack(spacing: 16) {
            TVGeneratedArt(seed: title, systemImage: "guitars", title: title)
                .frame(width: width - 24, height: width - 24)
                .clipShape(RoundedRectangle(cornerRadius: 26, style: .continuous))
                .padding(12)
                .background(RoundedRectangle(cornerRadius: 34, style: .continuous).fill(Color.white))
                .shadow(color: .black.opacity(isFocused ? 0.55 : 0.35), radius: isFocused ? 26 : 10, y: isFocused ? 16 : 6)
                .rotationEffect(.degrees(isFocused ? 0 : tilt))
                .scaleEffect(isFocused ? 1.06 : 1)
            Text(subtitle)
                .font(TVType.meta)
                .foregroundStyle(isFocused ? TVPalette.textSecondary : TVPalette.textTertiary)
        }
        .frame(width: width)
        .animation(.spring(response: 0.32, dampingFraction: 0.75), value: isFocused)
    }
}
