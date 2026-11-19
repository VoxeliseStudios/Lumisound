import ActivityKit
import Foundation
import UIKit

/// Starts/updates/ends the Lock Screen + Dynamic Island Live Activity. Purely
/// local (no push updates). Fed only by `WidgetDataService.publish`, so the
/// Live Activity always shows exactly the snapshot the widgets show.
///
/// Delivery rules (the activity used to stay on the previous track after a
/// crossfade, or after pause → skip/rewind while the app was in the background):
/// - Every live activity is updated, not just the one this process remembers.
///   A crash or relaunch could leave two; the one that was updated was not
///   always the one on screen. Extras are ended.
/// - Updates go out one at a time, newest last, coalesced over a few
///   milliseconds (track change + artwork + play state land as one update),
///   under a background-task assertion so a paused, backgrounded app is not
///   suspended between `update` being called and iOS receiving it.
/// - After sending, what each activity actually holds (`content.state`) is
///   compared with the latest snapshot and resent if it differs. A watchdog
///   keeps checking for a while after every change — independent of the
///   player's position timer, which stops while paused.
/// - Each snapshot carries a `revision`; the Live Activity view also reads the
///   App Group copy and shows whichever is newer (see `LumisoundLiveActivity`),
///   so a redraw after a button tap shows the new track even if the
///   ActivityKit update was dropped.
@MainActor
enum LiveActivityManager {
    private static var lastPublished: NowPlayingSnapshot?
    /// The activity this process started or adopted; kept when extras are ended.
    private static var primaryID: String?

    /// Starting an activity needs the app in the foreground; while backgrounded
    /// (normal during playback) `request` throws, and each failed request used
    /// to stack another "Allow Live Activities?" prompt. Requests are skipped
    /// in the background and backed off after a failure.
    private static var lastRequestFailureAt: Date?
    private static let requestRetryCooldown: TimeInterval = 60

    // MARK: Telemetry
    //
    // Counters (core `live_activity_count`, summarised by `sendSummary` with the
    // periodic diagnostics snapshot) for every path below, plus immediate events
    // for the moments worth seeing on their own: started, start failed, dismissed
    // by the user, ended by the system, Live Activities turned on/off.

    /// Skip reasons are counted once per track, not on every publish.
    private static var lastSkip: String?
    /// Set while this app ends activities, so a .dismissed/.ended state seen by
    /// the observer can be told apart from the user swiping the card away.
    private static var endingByApp = Set<String>()
    private static var observed = Set<String>()
    private static var enablementTask: Task<Void, Never>?

    private static func count(_ kind: String) { liveActivityCount(kind: kind) }

    private static func countSkip(_ reason: String, _ snapshot: NowPlayingSnapshot) {
        let key = reason + "|" + snapshot.trackID
        guard lastSkip != key else { return }
        lastSkip = key
        count("skipped_" + reason)
    }

    private static var stateName: String {
        UIApplication.shared.applicationState == .active ? "foreground" : "background"
    }

    /// Follows one activity's lifecycle: who ended it, and when it went stale.
    private static func observe(_ activity: Activity<LumisoundActivityAttributes>) {
        guard observed.insert(activity.id).inserted else { return }
        let id = activity.id
        Task { @MainActor in
            for await state in activity.activityStateUpdates {
                switch state {
                case .stale:
                    count("stale")
                case .dismissed, .ended:
                    let byApp = endingByApp.remove(id) != nil
                    let what = state == .dismissed ? "dismissed" : "ended"
                    count(byApp ? "\(what)_by_app" : "\(what)_by_system")
                    if !byApp {
                        // The user swiped it away (dismissed) or iOS ended it (e.g. the 8 h limit).
                        RemoteLogger.log(category: "widget", event: "live_activity_\(what)", level: "warning",
                                         message: "Live Activity \(what) outside the app",
                                         detail: ["state": stateName, "trackID": activity.content.state.now.trackID])
                    }
                    observed.remove(id)
                    return
                default:
                    break
                }
            }
            observed.remove(id)
        }
    }

    /// Reports Live Activities being turned off/on for the app in Settings.
    private static func watchEnablement() {
        guard enablementTask == nil else { return }
        enablementTask = Task { @MainActor in
            var last = ActivityAuthorizationInfo().areActivitiesEnabled
            for await enabled in ActivityAuthorizationInfo().activityEnablementUpdates where enabled != last {
                last = enabled
                count(enabled ? "enabled_on" : "enabled_off")
                RemoteLogger.log(category: "widget", event: "live_activity_enablement", level: enabled ? "info" : "warning",
                                 message: enabled ? "Live Activities turned on" : "Live Activities turned off",
                                 detail: ["enabled": enabled])
            }
        }
    }

    /// One `live_activity_summary` event with every counter since the last one,
    /// including the widget extension's marks. Called with the periodic snapshot.
    static func sendSummary() {
        if let url = LiveActivityMarks.fileURL { liveActivityIngestMarks(path: url.path) }
        guard let json = liveActivityTakeSummary(),
              var detail = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] else { return }
        let auth = ActivityAuthorizationInfo()
        detail["enabled"] = auth.areActivitiesEnabled
        detail["frequentPushesEnabled"] = auth.frequentPushesEnabled
        detail["activities"] = Activity<LumisoundActivityAttributes>.activities.count
        detail["settingOn"] = AudioPlayerManager.showsLiveActivity
        RemoteLogger.log(category: "widget", event: "live_activity_summary", message: "Live Activity counters", detail: detail)
    }

    private static var liveActivities: [Activity<LumisoundActivityAttributes>] {
        Activity<LumisoundActivityAttributes>.activities.filter { $0.activityState == .active || $0.activityState == .stale }
    }

    static func publish(_ snapshot: NowPlayingSnapshot) {
        guard snapshot.hasTrack, AudioPlayerManager.showsLiveActivity else {
            end(reason: snapshot.hasTrack ? "setting_off" : "no_track")
            return
        }
        watchEnablement()
        let live = liveActivities
        // Nothing the card would show differently (a favourite update that
        // changed nothing, or a small timing re-anchor the bar absorbs): skip
        // it. iOS budgets Live Activity updates, and spending them on drift
        // corrections is what let a track change be dropped in the background.
        if let last = lastPublished, !live.isEmpty, !materiallyDifferent(last, snapshot) { return }
        lastPublished = snapshot

        if !live.isEmpty {
            send(snapshot)
            return
        }

        guard ActivityAuthorizationInfo().areActivitiesEnabled else { countSkip("disabled", snapshot); return }
        guard UIApplication.shared.applicationState == .active else { countSkip("background", snapshot); return }
        if let lastFailure = lastRequestFailureAt, Date().timeIntervalSince(lastFailure) < requestRetryCooldown {
            countSkip("cooldown", snapshot)
            return
        }

        do {
            let a = try Activity<LumisoundActivityAttributes>.request(
                attributes: LumisoundActivityAttributes(),
                content: content(for: snapshot),
                pushType: nil
            )
            primaryID = a.id
            lastRequestFailureAt = nil
            observe(a)
            count("started")
            RemoteLogger.log(category: "widget", event: "live_activity_started", message: "Live Activity started",
                             detail: ["frequentPushesEnabled": ActivityAuthorizationInfo().frequentPushesEnabled,
                                      "hasAdvanceAt": snapshot.advanceAt != nil, "upNext": snapshot.upNext?.count ?? 0])
            watch()
        } catch {
            lastRequestFailureAt = Date()
            appError("LiveActivityManager: failed to start activity: \(error.localizedDescription)", category: "widget")
            count("start_failed")
            RemoteLogger.log(category: "widget", event: "live_activity_start_failed", level: "error",
                             message: error.localizedDescription,
                             detail: ["existing": Activity<LumisoundActivityAttributes>.activities.count,
                                      "error": String(describing: error)])
            // "Maximum number of activities" — orphans from earlier sessions
            // (a crash never runs `end()`) fill the per-app cap. End them so
            // the next attempt has room.
            let orphans = Activity<LumisoundActivityAttributes>.activities
            guard !orphans.isEmpty else { return }
            appWarn("LiveActivityManager: ending \(orphans.count) orphaned activity(s) from prior session(s)", category: "widget")
            for o in orphans { count("orphan_ended"); endingByApp.insert(o.id) }
            Task {
                for orphan in orphans {
                    await orphan.end(orphan.content, dismissalPolicy: .immediate)
                }
            }
        }
    }

    /// Whether `b` would look different on the Lock Screen than `a`.
    private static func materiallyDifferent(_ a: NowPlayingSnapshot, _ b: NowPlayingSnapshot) -> Bool {
        if a.trackID != b.trackID || a.isPlaying != b.isPlaying || a.isFavorite != b.isFavorite { return true }
        if a.title != b.title || a.artist != b.artist || a.detail != b.detail { return true }
        if a.thumbFile != b.thumbFile || a.tint != b.tint || a.artworkAspect != b.artworkAspect { return true }
        if (a.upNext ?? []).map(\.id) != (b.upNext ?? []).map(\.id) || a.queueIndex != b.queueIndex || a.queueCount != b.queueCount { return true }
        if abs(a.rate - b.rate) > 0.001 || abs(a.duration - b.duration) > 1 { return true }
        switch (a.advanceAt, b.advanceAt) {
        case (nil, nil): break
        case let (x?, y?): if abs(x.timeIntervalSince(y)) > 1 { return true }
        default: return true
        }
        let now = Date()
        return abs(a.elapsed(at: now) - b.elapsed(at: now)) > 1.5
    }

    /// Track of the last change whose delivery was logged (one line per track).
    private static var loggedDeliveryFor: String?

    /// Called every player tick and by the watchdog. If what an activity
    /// actually holds is a different track, play state or timeline than the
    /// last published snapshot, send the latest one again.
    static func verify() {
        guard let latest = lastPublished, pending == nil, !sending else { return }
        guard let stale = liveActivities.first(where: { !$0.content.state.now.isDelivery(of: latest) }) else { return }
        let foreground = inIntent || UIApplication.shared.applicationState == .active
        if !foreground {
            // In the background iOS budgets (and may refuse) these updates:
            // retry only when the card shows the wrong track or play state,
            // at most every 5 s (the player tick keeps calling this while
            // playing, so a refused track change is retried until it lands).
            let shown = stale.content.state.now
            guard shown.trackID != latest.trackID || shown.isPlaying != latest.isPlaying else { return }
            if let last = lastVerifyResend, Date().timeIntervalSince(last) < 5 { return }
        }
        if let last = lastVerifyResend, Date().timeIntervalSince(last) < 1.5 { return }
        lastVerifyResend = Date()
        count(foreground ? "resend_fg" : "resend_bg")
        appWarn("LiveActivityManager: activity showed \"\(stale.content.state.now.title)\", resending \"\(latest.title)\"", category: "widget")
        send(latest)
    }

    /// Waits (up to ~2 s) until queued updates have been delivered and every
    /// activity holds the latest snapshot. Called before a widget/Live
    /// Activity intent returns, since iOS redraws the activity at that moment
    /// and may suspend the (paused) app right after.
    private static var inIntent = false

    static func flush() async {
        inIntent = true
        defer { inIntent = false }
        // A command can change the track a turn of the run loop later
        // (engine callbacks hop onto the main actor): let those land first.
        for _ in 0..<3 { await Task.yield() }
        for _ in 0..<100 {
            if !sending, pending == nil {
                guard let latest = lastPublished,
                      liveActivities.contains(where: { !$0.content.state.now.isDelivery(of: latest) })
                else { return }
                lastVerifyResend = nil
                verify()
            }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
    }

    private static var pending: NowPlayingSnapshot?
    private static var sending = false
    private static var lastVerifyResend: Date?

    /// Updates go out one at a time, newest last. Separate `Task`s per update
    /// could finish out of order, leaving the previous track on screen.
    private static func send(_ snapshot: NowPlayingSnapshot) {
        pending = snapshot
        watch()
        guard !sending else { return }
        sending = true
        let bgTask = beginBackgroundWork()
        Task { @MainActor in
            // Coalesce a burst (track change, artwork, play state) into one update.
            try? await Task.sleep(nanoseconds: 40_000_000)
            var rounds = 0
            while let next = pending, rounds < 6 {
                pending = nil
                rounds += 1
                let live = endExtras(liveActivities)
                for a in live {
                    observe(a)
                    await a.update(content(for: next))
                    count("update_sent")
                }
                // Confirm what the system now holds; anything that didn't take
                // goes round again (bounded).
                let missed = live.contains(where: { !$0.content.state.now.isDelivery(of: next) })
                if pending == nil, missed {
                    pending = next
                }
                // One line per track change: did it reach the Lock Screen, and
                // was the app in the background (where iOS may refuse it)?
                if loggedDeliveryFor != next.trackID, pending == nil || !missed || rounds >= 6 {
                    loggedDeliveryFor = next.trackID
                    let state = stateName
                    count((missed ? "not_delivered_" : "delivered_") + (state == "foreground" ? "fg" : "bg"))
                    let msg = "LiveActivityManager: \(missed ? "NOT delivered" : "delivered") \"\(next.title)\" (\(state), round \(rounds), \(live.count) activity)"
                    if missed { appWarn(msg, category: "widget") } else { appLog(msg, category: "widget") }
                }
            }
            pending = nil
            sending = false
            endBackgroundWork(bgTask)
        }
    }

    /// Ends every live activity but one, so updates can't go to a hidden
    /// duplicate while a stale one stays on screen.
    private static func endExtras(_ live: [Activity<LumisoundActivityAttributes>]) -> [Activity<LumisoundActivityAttributes>] {
        guard live.count > 1 else {
            if let only = live.first { primaryID = only.id }
            return live
        }
        let keep = live.first(where: { $0.id == primaryID }) ?? live[0]
        primaryID = keep.id
        let extras = live.filter { $0.id != keep.id }
        appWarn("LiveActivityManager: ending \(extras.count) duplicate activity(s)", category: "widget")
        for a in extras { count("duplicate_ended"); endingByApp.insert(a.id) }
        Task {
            for a in extras { await a.end(a.content, dismissalPolicy: .immediate) }
        }
        return [keep]
    }

    // MARK: Watchdog

    private static var watchTask: Task<Void, Never>?
    private static var watchUntil: Date = .distantPast

    /// Re-checks the activities once a second for 12 s after every change.
    private static func watch() {
        watchUntil = Date().addingTimeInterval(12)
        guard watchTask == nil else { return }
        watchTask = Task { @MainActor in
            while Date() < watchUntil, !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                verify()
            }
            watchTask = nil
        }
    }

    // MARK: Background time

    private static func beginBackgroundWork() -> UIBackgroundTaskIdentifier {
        var id = UIBackgroundTaskIdentifier.invalid
        id = UIApplication.shared.beginBackgroundTask(withName: "LiveActivityUpdate") {
            UIApplication.shared.endBackgroundTask(id)
        }
        return id
    }

    private static func endBackgroundWork(_ id: UIBackgroundTaskIdentifier) {
        guard id != .invalid else { return }
        UIApplication.shared.endBackgroundTask(id)
    }

    private static func content(for snapshot: NowPlayingSnapshot) -> ActivityContent<LumisoundActivityAttributes.ContentState> {
        ActivityContent(
            state: LumisoundActivityAttributes.ContentState(now: snapshot),
            // The moment the track ends the system marks the activity stale
            // and redraws it. iOS ignores updates from this app while it plays
            // in the background, so that redraw is how an automatic advance or
            // crossfade reaches the Lock Screen: the view then reads the newer
            // snapshot the app has already saved in the App Group.
            // With a next track known, the stale moment is when it takes over
            // (`advanceAt`): the view then shows it on its own, on time.
            staleDate: snapshot.advanceAt ?? snapshot.endDate.map { $0.addingTimeInterval(0.75) },
            relevanceScore: snapshot.isPlaying ? 100 : 50
        )
    }

    static func end(reason: String = "stopped") {
        lastPublished = nil
        pending = nil
        primaryID = nil
        watchTask?.cancel()
        watchTask = nil
        let all = Activity<LumisoundActivityAttributes>.activities
        guard !all.isEmpty else { return }
        count("end." + reason)
        for a in all { endingByApp.insert(a.id) }
        Task {
            for a in all {
                var state = a.content.state
                state.now.isPlaying = false
                await a.end(ActivityContent(state: state, staleDate: nil), dismissalPolicy: .immediate)
            }
        }
    }
}
