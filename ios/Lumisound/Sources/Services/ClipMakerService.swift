import Foundation

enum ClipMakerError: Error {
    case exportFailed
}

/// Trims a segment out of a local track and exports it as its own `.m4a`.
/// Lumi clip (LumiClipService): the track is read by the Rust core in place
/// — LMS3 included, no unlocked copy — lossless for AAC LMS3 tracks,
/// otherwise rendered and AAC-encoded.
enum ClipMakerService {
    static func exportClip(from url: URL, start: TimeInterval, end: TimeInterval) async throws -> URL {
        do {
            return try await LumiClipService.clip(from: url, start: max(0, start), end: max(start + 0.5, end))
        } catch {
            throw ClipMakerError.exportFailed
        }
    }
}
