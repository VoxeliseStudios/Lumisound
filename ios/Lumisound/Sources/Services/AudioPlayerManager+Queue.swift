@preconcurrency import AVFoundation
import AudioToolbox
import Foundation
import MediaPlayer
import UIKit

extension AudioPlayerManager {

    // MARK: - Queue Management

    /// Drag-reorder support (e.g. List onMove) — operates on absolute indices
    /// across the WHOLE queue. Kept for backward compatibility / callers that
    /// genuinely want a whole-array move; the reworked `QueueView` uses the
    /// section-scoped `moveManualQueueItem`/`moveAutoQueueItem` below instead,
    /// since those map cleanly onto SwiftUI's per-`Section` `.onMove` indices.
    func moveQueueItem(from source: IndexSet, to destination: Int) {
        // Capture the current song id before mutation so we can re-anchor currentIndex.
        let currentSongID = currentSong?.id
        queue.move(fromOffsets: source, toOffset: destination)
        if let id = currentSongID,
           let newIndex = queue.firstIndex(where: { $0.id == id }) {
            currentIndex = newIndex
        }
        gaplessScheduled = false
        pendingNextIndex = nil
    }

    /// Reorders within the manually-queued block only — `source`/`destination`
    /// are indices local to that sub-list, exactly what SwiftUI's `List`
    /// reports from a single `Section`'s `ForEach.onMove`. Recomputes the
    /// block's absolute range fresh each call rather than trusting a
    /// previously-captured one, so it stays correct even if the queue changed
    /// out from under the view between renders.
    func moveManualQueueItem(from source: IndexSet, to destination: Int) {
        let range = manualBlockRange()
        var block = Array(queue[range])
        guard !block.isEmpty else { return }
        block.move(fromOffsets: source, toOffset: destination)
        queue.replaceSubrange(range, with: block)
    }

    /// Reorders within the auto-continuation tail only (everything after the
    /// manually-queued block) — same local-index contract as
    /// `moveManualQueueItem`. Lets users re-order upcoming tracks from the
    /// loaded playlist/album without promoting them to "manually queued".
    func moveAutoQueueItem(from source: IndexSet, to destination: Int) {
        let start = manualBlockRange().upperBound
        guard start < queue.count else { return }
        let range = start..<queue.count
        var block = Array(queue[range])
        block.move(fromOffsets: source, toOffset: destination)
        queue.replaceSubrange(range, with: block)
        // Moving within this tail can change which song is now immediately
        // next; invalidate any stashed pending-next resolution so gapless/
        // crossfade re-resolves it instead of handing off to a stale song.
        pendingNextIndex = nil
        gaplessScheduled = false
    }

    /// Swipe-to-delete support — removes by absolute `IndexSet` across the
    /// whole queue.
    func removeFromQueue(at offsets: IndexSet) {
        let currentSongID = currentSong?.id
        queue.remove(atOffsets: offsets)
        guard !queue.isEmpty else { stop(); return }
        if let id = currentSongID,
           let newIndex = queue.firstIndex(where: { $0.id == id }) {
            currentIndex = newIndex
        } else {
            // Current song was removed; clamp index and move on.
            currentIndex = min(currentIndex, queue.count - 1)
            currentSong = queue[currentIndex]
            gaplessScheduled = false
            pendingNextIndex = nil
            playCurrent(from: 0)
        }
    }

    /// Removes a single queue entry by its stable song id — used by
    /// swipe-to-remove in the reworked Queue UI, which (now that the queue is
    /// split into "Manually Queued" and "Up Next" sections) can no longer
    /// hand a single IndexSet local to the whole array. Removes only the
    /// FIRST matching instance, matching `removeFromQueue`'s IndexSet
    /// semantics for the (rare) case the same song appears twice in queue.
    func removeSong(id: Song.ID) {
        guard let offset = queue.firstIndex(where: { $0.id == id }) else { return }
        removeFromQueue(at: IndexSet(integer: offset))
    }

    /// Insert a song to play immediately after the current track — the
    /// "Play Next" action. Tagging it `.manual` (the default) is what makes
    /// it show up in the Queue UI's "Manually Queued" section instead of
    /// blending into the auto-continuation tail. Repeated calls each land
    /// right after `currentIndex`, so the most recently "played next" song
    /// plays soonest — matching the mental model of stacking picks in front
    /// of whatever's already lined up.
    func insertNext(song: Song, source: QueueSource = .manual) {
        var song = song
        song.queueSource = source
        let insertionIndex = currentIndex + 1
        if insertionIndex >= queue.count {
            queue.append(song)
        } else {
            queue.insert(song, at: insertionIndex)
        }
    }

    /// Append a song without affecting current playback — the "Add to Queue"
    /// action. Unlike a plain array append, this lands at the END of the
    /// manually-queued block (i.e. after any earlier "Play Next"/"Add to
    /// Queue" picks, but BEFORE the auto-continuation tail resumes), so
    /// explicit user picks always play before Auto-Radio or the rest of the
    /// loaded playlist/album gets a turn — not buried after however many
    /// tracks the context queue happens to have left.
    ///
    /// `source` defaults to `.manual` for every existing "Add to Queue" call
    /// site (menus, streaming search, Discover Mix, On This Day); Auto-Radio's
    /// own continuation append (see `LumisoundApp`) explicitly passes
    /// `.autoContinuation` instead so it correctly lands in the "Up Next"
    /// section rather than "Manually Queued".
    func appendToQueue(song: Song, source: QueueSource = .manual) {
        var song = song
        song.queueSource = source
        if source == .manual {
            let insertionIndex = manualBlockRange().upperBound
            queue.insert(song, at: insertionIndex)
        } else {
            queue.append(song)
        }
    }

    /// Clears the auto-radio seed after the LumisoundApp observer has handled it.
    func clearAutoRadioSeed() {
        autoRadioSeed = nil
    }

    // MARK: - Manual vs. Auto-Continuation Grouping

    /// Absolute indices of the contiguous manually-queued block that begins
    /// right after the current track (see `insertNext`/`appendToQueue`).
    /// Walking forward from `currentIndex + 1` while each entry resolves to
    /// `.manual` is what keeps this correct across shuffles/moves without any
    /// separate bookkeeping — a `Song`'s `queueSource` travels with the value
    /// itself, so this always reflects the queue's actual current contents.
    func manualBlockRange() -> Range<Int> {
        let start = currentIndex + 1
        guard start <= queue.count else { return queue.count..<queue.count }
        var end = start
        while end < queue.count && queue[end].resolvedQueueSource == .manual {
            end += 1
        }
        return start..<end
    }

    /// Songs after the current one that were explicitly queued by the user
    /// ("Play Next"/"Add to Queue"), in play order. Empty while
    /// `repeatMode == .one`, since nothing else is "up next" in that mode.
    var manuallyQueuedUpNext: [Song] {
        guard repeatMode != .one, !queue.isEmpty else { return [] }
        let range = manualBlockRange()
        guard range.upperBound <= queue.count else { return [] }
        return Array(queue[range])
    }

    /// Songs that will play next as part of the natural playback context —
    /// the loaded playlist/album/library list, or Auto-Radio's continuation —
    /// AFTER the manually-queued block is exhausted. Mirrors
    /// `resolveNextIndex`'s repeat-mode semantics (including the repeat-all
    /// wraparound to the start of the queue) so this only ever lists songs
    /// that will actually play, not ones playback would never reach.
    var autoContinuationUpNext: [Song] {
        guard repeatMode != .one, queue.count > 1 else { return [] }
        let start = manualBlockRange().upperBound
        guard start <= queue.count else { return [] }
        var result = Array(queue[start...])
        if repeatMode == .all {
            result += queue[0..<min(currentIndex, queue.count)]
        }
        return result
    }

    /// `manuallyQueuedUpNext.count + autoContinuationUpNext.count`, without
    /// building either array — both copy the rest of the queue (the whole
    /// library, when playing from Songs), and badges only need the number.
    var upNextCount: Int {
        guard repeatMode != .one, !queue.isEmpty else { return 0 }
        let after = max(queue.count - (currentIndex + 1), 0)
        let wrap = (repeatMode == .all && queue.count > 1) ? min(max(currentIndex, 0), queue.count) : 0
        return after + wrap
    }

    /// Human-readable "Playing from X" context label — the playlist name if
    /// the current queue was started from one, else a generic fallback.
    /// Takes `LibraryManager` as a parameter (mirroring the existing weak
    /// `libraryManager` reference already used for BPM lookups) rather than
    /// this file owning a lookup into playlist storage itself.
    func playingFromContextLabel(library: LibraryManager) -> String {
        if let id = currentPlaylistID,
           let playlist = library.playlists.first(where: { $0.id == id }) {
            return "Playing from \(playlist.name)"
        }
        return "Playing from Queue"
    }

    // MARK: - Queue Helpers

    /// Resolves which queue index plays after `currentIndex`, WITHOUT mutating any state.
    /// Shuffle reorders `queue` itself when enabled (see `shuffleQueue`), so the
    /// upcoming index is always just the next sequential slot — keeping this in sync
    /// with what the Up Next/Queue UI displays.
    func resolveNextIndex() -> Int? {
        guard !queue.isEmpty else { return nil }
        if repeatMode == .one { return currentIndex }
        let nextIndex = currentIndex + 1
        if nextIndex < queue.count { return nextIndex }
        if repeatMode == .all { return 0 }
        return nil
    }

    func peekNextSong() -> Song? {
        guard let nextIndex = resolveNextIndex() else {
            pendingNextIndex = nil
            return nil
        }
        guard queue.indices.contains(nextIndex) else {
            pendingNextIndex = nil
            return nil
        }
        pendingNextIndex = nextIndex
        return queue[nextIndex]
    }

    /// Downloads the upcoming streamed track into the core's stream cache
    /// (by source id, so `downloadAndSchedule` and the Rust engine's gapless /
    /// Smart Crossfade find it), and asks the bridge to resolve the track
    /// after that, so both switches start without the resolve + download wait.
    func prefetchUpcomingStreamIfNeeded() {
        guard let nextIndex = resolveNextIndex(), queue.indices.contains(nextIndex) else { return }
        let nextSong = queue[nextIndex]
        guard nextSong.id != currentSong?.id, let url = nextSong.url,
              ["http", "https"].contains(url.scheme?.lowercased() ?? "") else { return }
        streamCachePrefetch(items: [StreamPrefetch(sourceKey: StreamCache.key(for: url), url: url.absoluteString,
                                                   headers: StreamCache.headers(for: nextSong))])
        let after = nextIndex + 1
        if queue.indices.contains(after), let id = queue[after].sourceTrackID?.split(separator: ":", maxSplits: 1),
           id.count == 2, queue[after].url?.isFileURL == false {
            streamPrefetchUrls(tracks: [PrefetchTrack(id: String(id[1]), source: String(id[0]), url: nil)],
                               engine: Self.rustEngineEnabled)
        }
    }

    func advanceIndex() {
        guard !queue.isEmpty else { return }
        if repeatMode == .one { return }
        // Prefer the index that was actually resolved (and scheduled/crossfaded to)
        // by the most recent `peekNextSong()` — re-resolving here would let shuffle
        // mode land on a different song than the audio that's already playing.
        let nextIndex: Int
        if let pending = pendingNextIndex, queue.indices.contains(pending) {
            nextIndex = pending
        } else if let resolved = resolveNextIndex() {
            nextIndex = resolved
        } else {
            return
        }
        currentIndex = nextIndex
        currentSong = queue[currentIndex]
        pendingNextIndex = nil
    }
}

// MARK: - Song Queue-Source Convenience

extension Song {
    /// `queueSource` is `nil` for any `Song` set up before this field existed
    /// (older persisted snapshots, songs restored from cloud sync before the
    /// backend supports round-tripping this — see `AccountService+QueueSync`)
    /// or for a plain library/search-result `Song` never placed in a queue.
    /// Reading through this instead of the raw optional treats all of those
    /// consistently as "part of the auto-continuation context", which is the
    /// correct default: only an explicit "Play Next"/"Add to Queue" action
    /// should ever mark something `.manual`.
    var resolvedQueueSource: QueueSource {
        queueSource ?? .autoContinuation
    }

    /// Convenience for `resolvedQueueSource == .manual`.
    var isManuallyQueued: Bool {
        resolvedQueueSource == .manual
    }
}
