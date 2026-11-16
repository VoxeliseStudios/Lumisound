import Foundation

/// Exports a time range of a local track's audio as a standalone `.m4a`
/// file — for a ringtone (via Finder/GarageBand once saved out; there's no
/// public "set as ringtone" API for a non-App-Store app to call directly),
/// a sample, a voice-memo intro, whatever. Trims via `AVMutableComposition`
/// + `AVAssetExportSession` (Apple's own audio-only M4A preset) rather than
/// re-encoding by hand. Local files only, same restriction as
/// `SilenceTrimAnalyzer` — exporting a remote stream would mean
/// downloading the whole thing first just to trim it.
enum ClipExportError: LocalizedError {
    case notLocalFile
    case noAudioTrack
    case invalidRange
    case exportFailed(String)

    var errorDescription: String? {
        switch self {
        case .notLocalFile: return "Only downloaded/local tracks can be exported as a clip."
        case .noAudioTrack: return "Couldn't read this track's audio."
        case .invalidRange: return "The loop's end must come after its start."
        case .exportFailed(let message): return message
        }
    }
}

enum ClipExportService {
    /// Exports `url`'s audio between `start` and `end` (seconds) to a
    /// temporary `.m4a` and returns its URL (the caller presents it, e.g.
    /// via `ShareLink`). Lumi clip: read by the Rust core in place (LMS3,
    /// no unlocked copy); lossless for AAC LMS3 tracks.
    static func exportClip(from url: URL, start: TimeInterval, end: TimeInterval, title: String) async throws -> URL {
        guard url.isFileURL else { throw ClipExportError.notLocalFile }
        guard end > start else { throw ClipExportError.invalidRange }
        let sanitizedTitle = title
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }
            .joined(separator: "_")
        do {
            return try await LumiClipService.clip(from: url, start: start, end: end, baseName: sanitizedTitle.isEmpty ? "clip" : sanitizedTitle)
        } catch LumiClipError.unreadable {
            throw ClipExportError.noAudioTrack
        } catch {
            throw ClipExportError.exportFailed(error.localizedDescription)
        }
    }
}
