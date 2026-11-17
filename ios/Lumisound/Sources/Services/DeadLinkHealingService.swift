import Foundation

// MARK: - DeadLinkHealingService
//
// A downloaded track's audio is untouched even after its upstream source
// (a YouTube video) is removed/made private — the local file keeps playing
// exactly as before. What breaks is FUTURE re-downloads: a fresh install,
// a new device, or `LumisoundExclusiveExtensionService`'s own"leftover
// pre-conversion file" retry path all rely on `sourceTrackID` still
// resolving to something real. This periodically re-checks already-
// downloaded tracks' sources and, for ones that have gone dead, searches
// for a replacement and relinks — but ONLY when the replacement clears the
// exact same "same normalized title/artist + duration within tolerance"
// bar `DuplicateFinderService` already trusts elsewhere in this app for
// treating two tracks as genuinely the same recording. Anything less
// certain is left alone rather than guessed at — a wrong relink is worse
// than a merely-stale one, since it would silently point a real, currently-
// working track at unrelated audio for any FUTURE re-download.
@MainActor
enum DeadLinkHealingService {
    private static let lastRunKey = "deadLinkHealing.lastRun"
    private static let interval: TimeInterval = 24 * 60 * 60

    /// Real yt-dlp metadata fetches server-side, one per candidate — capped
    /// per pass so this can't turn into an unbounded scan of a large
    /// library hammering the bridge every 24h.
    private static let maxChecksPerPass = 25

    static func runIfNeeded() async {
        let lastRun = UserDefaults.standard.double(forKey: lastRunKey)
        guard Date().timeIntervalSince1970 - lastRun >= interval else { return }
        guard let library = LibraryManager.shared, let streaming = StreamingService.shared else { return }

        // Only YouTube sources have a reconstructable watch URL to re-check
        // right now (see AccountService+Avatar.swift's pushPlaybackState
        // for the same "youtube only" reconstruction limit).
        let candidates: [(song: Song, bareID: String)] = library.importedSongs.compactMap { song in
            guard let sourceTrackID = song.sourceTrackID,
                  let colon = sourceTrackID.firstIndex(of: ":"),
                  sourceTrackID[sourceTrackID.startIndex..<colon] == "youtube" else { return nil }
            let bareID = String(sourceTrackID[sourceTrackID.index(after: colon)...])
            guard !bareID.isEmpty else { return nil }
            return (song, bareID)
        }

        guard !candidates.isEmpty else {
            UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: lastRunKey)
            return
        }

        var checked = 0
        var healed = 0
        for (song, bareID) in candidates.prefix(maxChecksPerPass) {
            checked += 1
            let watchURL = "https://youtube.com/watch?v=\(bareID)"
            guard await streaming.checkTrackAvailable(url: watchURL) == false else { continue }

            appWarn("DeadLinkHealingService: \"\(song.title)\" source is no longer available, searching for a replacement", category: "library")
            let results = await streaming.searchSilently(query: "\(song.title) \(song.artist)", source: "youtube")
            guard let replacement = bestMatch(for: song, among: results) else {
                appWarn("DeadLinkHealingService: no confident replacement found for \"\(song.title)\" — left as-is", category: "library")
                continue
            }

            if library.updateSourceTrackID(songID: song.id, to: replacement.sourceTrackID) {
                healed += 1
                appLog("DeadLinkHealingService: relinked \"\(song.title)\" to \(replacement.sourceTrackID)", category: "library")
                RemoteLogger.log(category: "library", event: "dead_link_healed", detail: ["title": song.title])
                AriaActivityLog.shared.logDeadLinkHealed(title: song.title, artist: song.artist)
            }
        }

        appLog("DeadLinkHealingService: checked \(checked)/\(candidates.count) track(s), healed \(healed)", category: "library")
        UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: lastRunKey)
    }

    /// Same normalised title and artist, duration within 2 s — the bar
    /// duplicate merging already trusts (core `dup_best_match`).
    private static func bestMatch(for song: Song, among candidates: [StreamTrack]) -> StreamTrack? {
        let target = DupTrack(title: song.title, artist: song.artist, duration: song.duration)
        let pool = candidates.map { DupTrack(title: $0.title, artist: $0.artist, duration: $0.duration) }
        return dupBestMatch(target: target, candidates: pool).map { candidates[Int($0)] }
    }
}
