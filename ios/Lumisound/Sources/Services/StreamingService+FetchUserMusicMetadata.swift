import Foundation
import UIKit

extension StreamingService {

    // MARK: - Fetch User Music Metadata

    /// Fetches rich metadata for all uploaded tracks from `/user/music/metadata`.
    ///
    /// Callers that overlap (backup pass, thumbnail backfill, upload screen)
    /// share one request instead of each downloading the whole list, and a
    /// result younger than `maxAge` seconds is reused. The JSON (thousands of
    /// tracks) is decoded off the main thread.
    @discardableResult
    func fetchUserMusicMetadata(token: String, maxAge: TimeInterval = 0) async throws -> [UserMusicMetadataTrack] {
        if maxAge > 0, let at = userMusicMetadataFetchedAt, Date().timeIntervalSince(at) < maxAge {
            return userMusicMetadata
        }
        if let inFlight = userMusicMetadataTask {
            return try await inFlight.value
        }
        let task = Task { [weak self] () async throws -> [UserMusicMetadataTrack] in
            guard let self else { throw CancellationError() }
            return try await self.loadUserMusicMetadata(token: token)
        }
        userMusicMetadataTask = task
        defer { userMusicMetadataTask = nil }
        return try await task.value
    }

    private func loadUserMusicMetadata(token: String) async throws -> [UserMusicMetadataTrack] {
        appLog("fetchUserMusicMetadata", category: "network")
        isLoadingUserMusicMetadata = true
        defer { isLoadingUserMusicMetadata = false }

        // The core keeps the list current incrementally (`?since=`: only rows
        // changed since the last answer, plus deletions), so after the first
        // fetch a large cloud library no longer downloads in full each time.
        _ = makeRequest("/user/music/metadata")  // keeps the core's bridge session current
        let data: Data
        do {
            data = try await cloudMetadataFetch(accountToken: token)
        } catch CoreError.Http(let status, _) {
            throw StreamingError.httpError(Int(status))
        } catch CoreError.Unauthorized {
            throw StreamingError.httpError(401)
        }

        let tracks = try await Task.detached(priority: .userInitiated) {
            try JSONDecoder().decode(UserMusicMetadataResponse.self, from: data).tracks
        }.value
        userMusicMetadata = tracks
        userMusicMetadataFetchedAt = Date()
        appLog("fetchUserMusicMetadata: \(tracks.count) tracks", category: "network")
        return tracks
    }

    /// Adds or replaces one track in the cached list (after an upload), so
    /// views see it without re-downloading the whole list.
    func mergeUserMusicMetadata(_ track: UserMusicMetadataTrack) {
        guard !track.id.isEmpty else { return }
        if let i = userMusicMetadata.firstIndex(where: { $0.id == track.id }) {
            userMusicMetadata[i] = track
        } else {
            userMusicMetadata.insert(track, at: 0)
        }
    }
}

private struct UserMusicMetadataResponse: Decodable {
    let tracks: [UserMusicMetadataTrack]
}
