import Foundation

// MARK: - Download retry queue
//
// The bridge keeps downloads that failed only because YouTube walled the
// cookies (bot check) or rate-limited the session, and says when that cause
// has cleared: live event `downloads_retry` while the app is connected, and
// GET /api/download/retry (core `downloadRetryPending`) at launch for an app
// that was away. The core already cleared those tracks' back-off; this
// re-queues them through the normal download path and acknowledges them.
extension AccountService {

    /// Download these tracks again (at most 3 at a time), then tell the bridge.
    func retryBlockedDownloads(_ tracks: [RetryTrack], library: LibraryManager) {
        guard !tracks.isEmpty, let streaming = DeviceSyncService.shared?.streaming else { return }
        let existing = library.allSongs
        Task { @MainActor in
            var got = 0
            await withTaskGroup(of: Bool.self) { group in
                var next = 0
                func startNext() {
                    guard next < tracks.count else { return }
                    let t = tracks[next]
                    next += 1
                    let track = StreamTrack(id: t.id, title: t.title ?? t.id, artist: t.artist ?? "", durationSeconds: 0,
                                            thumbnailURL: "", source: t.source,
                                            youtubeURL: t.source == "youtube" ? "https://www.youtube.com/watch?v=\(t.id)" : "")
                    group.addTask { @MainActor in
                        do {
                            _ = try await streaming.downloadToLibrary(track: track, existingSongs: existing, reportExistingAsSkipped: true)
                            return true
                        } catch {
                            return false
                        }
                    }
                }
                for _ in 0..<min(3, tracks.count) { startNext() }
                for await ok in group {
                    if ok { got += 1 }
                    startNext()
                }
            }
            downloadRetryAck(keys: tracks.map { "\($0.source):\($0.id)" })
            if got > 0 {
                library.scanLocalDocuments()
                ToastCenter.shared.show("Downloaded \(got) track\(got == 1 ? "" : "s") YouTube had blocked earlier", category: .download)
            }
            appLog("downloads retry: \(got)/\(tracks.count) re-downloaded", category: "network")
        }
    }

    /// Launch / foreground: pick up retries announced while the app was away.
    func fetchPendingDownloadRetries(library: LibraryManager) async {
        guard isLoggedIn, let ready = try? await downloadRetryPending(), !ready.isEmpty else { return }
        retryBlockedDownloads(ready, library: library)
    }
}
