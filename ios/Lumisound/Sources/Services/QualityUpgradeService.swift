import Foundation
import SwiftUI

// MARK: - QualityUpgradeService
//
// YouTube Premium audio for the library. When the account's YouTube cookies
// are Premium (the bridge probes them: GET /user/ytdlp-cookies/premium),
// tracks that came from YouTube below Premium quality are fetched again as
// Lumisound Audio (format "lumi": YouTube Music's ~256 kbps Opus for
// Premium) and swapped in place as LMS3 — same file, same song, same tags,
// artwork and play history. All the deciding and replacing happens in the
// Rust core (`quality_upgrade_candidates` / `quality_replace_audio`, which
// refuse anything that isn't a better copy of the same recording); this
// type only schedules it and publishes progress for Settings.
//
// Paced like every other library pass: a bounded batch per run, never the
// playing track, waits for thermal headroom, one download at a time.

struct PremiumAudioStatus: Decodable, Equatable {
    let premium: Bool
    let bestKbps: Int
    let maxKbps: Int
    let hasCookies: Bool
    let checkedAt: String?

    enum CodingKeys: String, CodingKey {
        case premium
        case bestKbps = "best_kbps"
        case maxKbps = "max_kbps"
        case hasCookies = "has_cookies"
        case checkedAt = "checked_at"
    }
}

@MainActor
final class QualityUpgradeService: ObservableObject {
    static let shared = QualityUpgradeService()

    static let enabledKey = "quality_auto_upgrade_premium"
    private static let lastRunKey = "quality_upgrade_last_run"
    private static let upgradedCountKey = "quality_upgrade_total"
    /// Tracks tried per pass (each is a full download).
    private static let batchSize = 25
    private static let minInterval: TimeInterval = 30 * 60

    @Published private(set) var status: PremiumAudioStatus?
    @Published private(set) var isRunning = false
    @Published private(set) var remaining = 0
    @Published private(set) var upgradedTotal = UserDefaults.standard.integer(forKey: QualityUpgradeService.upgradedCountKey)
    @Published private(set) var lastDetail: String?

    var isEnabled: Bool {
        get { UserDefaults.standard.object(forKey: Self.enabledKey) as? Bool ?? true }
        set {
            UserDefaults.standard.set(newValue, forKey: Self.enabledKey)
            objectWillChange.send()
        }
    }

    /// Tracks the core said aren't better (or failed) this session.
    private var skipped: Set<String> = []

    private init() {}

    // MARK: Premium status

    @discardableResult
    func refreshStatus(force: Bool = false) async -> PremiumAudioStatus? {
        guard let account = AccountService.shared, account.isLoggedIn else { return nil }
        do {
            let data = try await account.makeRequest("/user/ytdlp-cookies/premium" + (force ? "?refresh=true" : ""))
            let s = try JSONDecoder().decode(PremiumAudioStatus.self, from: data)
            status = s
            return s
        } catch {
            appWarn("QualityUpgradeService: premium status unavailable: \(error.localizedDescription)", category: "network")
            return status
        }
    }

    // MARK: Upgrade pass

    /// Runs a bounded upgrade batch when the account is Premium, upgrades are
    /// on, and the last pass was a while ago (`force` skips the interval).
    func runPassIfNeeded(force: Bool = false) async {
        guard isEnabled, !isRunning else { return }
        let last = UserDefaults.standard.double(forKey: Self.lastRunKey)
        if !force, Date().timeIntervalSince1970 - last < Self.minInterval { return }
        guard let s = await refreshStatus(), s.premium else { return }
        guard let library = LibraryManager.shared, let streaming = StreamingService.shared else { return }
        UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: Self.lastRunKey)
        isRunning = true
        defer { isRunning = false }

        let playingPath = AudioPlayerManager.shared?.currentSong?.url?.path
        let paths = library.importedSongs.compactMap { $0.url?.path }.filter { $0 != playingPath && !skipped.contains($0) }
        let candidates = await Task.detached(priority: .utility) {
            qualityUpgradeCandidates(paths: paths)
        }.value
        remaining = candidates.count
        guard !candidates.isEmpty else { return }
        appLog("QualityUpgradeService: \(candidates.count) track(s) below Premium quality; upgrading up to \(Self.batchSize)", category: "background")

        var upgraded = 0
        for c in candidates.prefix(Self.batchSize) {
            if Task.isCancelled || !isEnabled { break }
            guard await HeatGovernor.waitForHeadroom(job: "Premium quality upgrade") else { break }
            // The listener may have started this one meanwhile.
            if AudioPlayerManager.shared?.currentSong?.url?.path == c.path { continue }
            guard let file = await download(youtubeID: c.youtubeId, streaming: streaming) else {
                skipped.insert(c.path)
                continue
            }
            let outcome = await Task.detached(priority: .utility) {
                qualityReplaceAudio(path: c.path, newAudioPath: file, quality: "")
            }.value
            remaining = max(0, remaining - 1)
            lastDetail = outcome.detail
            if outcome.replaced {
                upgraded += 1
                upgradedTotal += 1
                UserDefaults.standard.set(upgradedTotal, forKey: Self.upgradedCountKey)
                appLog("QualityUpgradeService: upgraded \(URL(fileURLWithPath: c.path).lastPathComponent): \(outcome.detail)", category: "background")
            } else {
                skipped.insert(c.path)
                appLog("QualityUpgradeService: kept \(URL(fileURLWithPath: c.path).lastPathComponent): \(outcome.detail)", category: "background")
            }
        }
        if upgraded > 0 {
            NotificationCenter.default.post(name: .lumisoundLibraryAudioReplaced, object: nil)
        }
    }

    /// The track as Lumisound Audio from the bridge, into a temp file (not
    /// the library). Nil on any failure (logged).
    private func download(youtubeID: String, streaming: StreamingService) async -> String? {
        var components = URLComponents()
        components.path = "/api/download"
        components.queryItems = [
            URLQueryItem(name: "id", value: youtubeID),
            URLQueryItem(name: "source", value: "youtube"),
            URLQueryItem(name: "format", value: "lumi"),
            URLQueryItem(name: "title", value: "upgrade-\(youtubeID)"),
        ]
        guard var request = streaming.makeRequest(components.string ?? "/api/download"), let url = request.url?.absoluteString else { return nil }
        if let token = AccountService.shared?.token {
            request.setValue(token, forHTTPHeaderField: "X-Account-Token")
        }
        let headers = (request.allHTTPHeaderFields ?? [:]).map { HttpHeader(name: $0.key, value: $0.value) }
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("lumisound-upgrade").path
        do {
            let job = try await BackgroundDownloadManager.run(named: "lumisound.upgrade.\(youtubeID)") {
                try await bridgeDownloadJob(method: "GET", startUrl: url, headers: headers, body: nil, tmpDir: tmp)
            }
            return job.filePath
        } catch {
            appWarn("QualityUpgradeService: download of \(youtubeID) failed: \(error)", category: "network")
            return nil
        }
    }
}

extension Notification.Name {
    /// Library files had their audio replaced in place (quality upgrade):
    /// cached durations/format details may be stale.
    static let lumisoundLibraryAudioReplaced = Notification.Name("lumisoundLibraryAudioReplaced")
}
