import Foundation

// MARK: - Library Hub Content
//
// The Library home screen's shelves (recently added / played, most played,
// forgotten favourites, genres, top artists, decades, deeper cuts, weekly
// recap), tile collages and listened fractions. Computed by the Rust core
// (rust/core/src/hub.rs) from the songs plus the core's own play history;
// this file only converts songs to the core's record and ids back to songs.
// Safe to call off the main actor (`LibraryHubView` / `LumenHomeView` build
// the snapshot in a detached task).

struct HubContentBuilder {
    let songs: [Song]

    private var byID: [String: Song] {
        Dictionary(songs.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
    }

    static func record(_ s: Song) -> HubSong {
        HubSong(id: s.id, artist: s.artist, artistName: s.artistName, genre: s.genre, year: s.year,
                duration: s.duration, dateAdded: s.dateAdded?.timeIntervalSince1970, albumKey: s.groupableAlbumName)
    }

    private func snapshotRaw(favorites: [Song], songs limit: Int, genres: Int = 12, artists: Int = 12, decades: Int = 10) -> HubSnapshot {
        hubSnapshot(songs: songs.map(Self.record), favoriteIds: favorites.map(\.id),
                    limits: HubLimits(songs: UInt32(limit), genres: UInt32(genres), artists: UInt32(artists), decades: UInt32(decades)))
    }

    private static func songs(_ ids: [String], _ byID: [String: Song]) -> [Song] {
        ids.compactMap { byID[$0] }
    }

    func recentlyAddedSongs(limit: Int = 25) -> [Song] {
        Self.songs(hubShelf(songs: songs.map(Self.record), shelf: .recentlyAdded, limit: UInt32(limit)), byID)
    }

    func mostPlayedSongs(limit: Int = 25) -> [Song] {
        Self.songs(hubShelf(songs: songs.map(Self.record), shelf: .mostPlayed, limit: UInt32(limit)), byID)
    }

    func listenedFraction(of songs: [Song]) -> Double? {
        hubListenedFraction(songIds: songs.map(\.id))
    }

    func genreGroups(limit: Int = 12) -> [(genre: String, songs: [Song])] {
        let map = byID
        return snapshotRaw(favorites: [], songs: 0, genres: limit).genreGroups.map { (genre: $0.name, songs: Self.songs($0.songIds, map)) }
    }

    /// Collage tiles are drawn while scrolling, so only the first 256 songs
    /// are considered (enough to find four distinct albums in practice).
    func collageSongs(from songs: [Song], limit: Int = 4) -> [Song] {
        guard songs.count > limit else { return Array(songs.prefix(limit)) }
        let head = songs.prefix(256)
        let map = Dictionary(head.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        return Self.songs(hubCollage(songs: head.map(Self.record), limit: UInt32(limit)), map)
    }

    func topArtistGroups(limit: Int = 12) -> [(artist: String, songs: [Song], playCount: Int)] {
        let map = byID
        return snapshotRaw(favorites: [], songs: 0, artists: limit).topArtistGroups.map {
            (artist: $0.name, songs: Self.songs($0.songIds, map), playCount: Int($0.playCount))
        }
    }

    struct HubWeeklyRecap {
        let songsPlayed: Int
        let estimatedMinutes: Int
        let topArtist: String?
    }

    /// Every Home shelf, computed in one go — see `snapshot(favorites:)`.
    struct Snapshot {
        var recentlyAdded: [Song] = []
        var mostPlayed: [Song] = []
        var forgottenFavorites: [Song] = []
        var recentlyPlayed: [Song] = []
        var genreGroups: [(genre: String, songs: [Song])] = []
        var topArtistGroups: [(artist: String, songs: [Song], playCount: Int)] = []
        var decadeGroups: [(decade: String, songs: [Song])] = []
        var deeperCuts: [Song] = []
        var weeklyRecap: HubWeeklyRecap?
    }

    /// All of the hub's shelves in one core call (one pass over the library).
    func snapshot(favorites: [Song]) -> Snapshot {
        let raw = snapshotRaw(favorites: favorites, songs: 20)
        let map = byID
        return Snapshot(
            recentlyAdded: Self.songs(raw.recentlyAdded, map),
            mostPlayed: Self.songs(raw.mostPlayed, map),
            forgottenFavorites: Self.songs(raw.forgottenFavorites, map),
            recentlyPlayed: Self.songs(raw.recentlyPlayed, map),
            genreGroups: raw.genreGroups.map { (genre: $0.name, songs: Self.songs($0.songIds, map)) },
            topArtistGroups: raw.topArtistGroups.map { (artist: $0.name, songs: Self.songs($0.songIds, map), playCount: Int($0.playCount)) },
            decadeGroups: raw.decadeGroups.map { (decade: $0.name, songs: Self.songs($0.songIds, map)) },
            deeperCuts: Self.songs(raw.deeperCuts, map),
            weeklyRecap: raw.weeklyRecap.map {
                HubWeeklyRecap(songsPlayed: Int($0.songsPlayed), estimatedMinutes: Int($0.estimatedMinutes), topArtist: $0.topArtist)
            }
        )
    }
}

// MARK: - LibraryManager conveniences

extension LibraryManager {
    typealias HubWeeklyRecap = HubContentBuilder.HubWeeklyRecap

    var hubContent: HubContentBuilder {
        HubContentBuilder(songs: allSongs)
    }

    func recentlyAddedSongs(limit: Int = 25) -> [Song] { hubContent.recentlyAddedSongs(limit: limit) }
    func mostPlayedSongs(limit: Int = 25) -> [Song] { hubContent.mostPlayedSongs(limit: limit) }
    func listenedFraction(of songs: [Song]) -> Double? { hubContent.listenedFraction(of: songs) }
    func genreGroups(limit: Int = 12) -> [(genre: String, songs: [Song])] { hubContent.genreGroups(limit: limit) }
    func collageSongs(from songs: [Song], limit: Int = 4) -> [Song] { hubContent.collageSongs(from: songs, limit: limit) }
    func topArtistGroups(limit: Int = 12) -> [(artist: String, songs: [Song], playCount: Int)] {
        hubContent.topArtistGroups(limit: limit)
    }

    /// Library songs last played today (local time).
    func songsPlayedTodayCount() -> Int {
        Int(hubSongsPlayedToday(libraryIds: Array(songsByID.keys)))
    }
}
