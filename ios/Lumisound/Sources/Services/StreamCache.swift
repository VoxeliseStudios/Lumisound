import Foundation

/// Streamed tracks' audio, cached by the core (`streamcache.rs`) under the
/// track's source id ("youtube:<id>"), not the URL: the stream ticket in the
/// URL rotates, which used to give the same track a new cache file every half
/// hour. Analysis and metadata readers (ReplayGain, BPM, artwork, ...) read a
/// streamed song from this local copy, never from its URL (each URL read was
/// a burst of range requests, each a yt-dlp run on the bridge).
enum StreamCache {
    /// Cache key for a stream URL (credentials stripped).
    static func key(for url: URL) -> String {
        streamSourceKey(url: url.absoluteString)
    }

    /// The downloaded copy of `url`, if there is one.
    static func cachedFile(for url: URL) -> URL? {
        streamCacheLookup(sourceKey: key(for: url)).map { URL(fileURLWithPath: $0) }
    }

    /// A URL that can be read without touching the network: `url` itself for
    /// a file, the cached download for a stream, nil when there is none yet.
    static func localURL(for url: URL) -> URL? {
        if url.isFileURL { return url }
        guard ["http", "https"].contains(url.scheme?.lowercased() ?? "") else { return url }
        return cachedFile(for: url)
    }

    /// Headers for fetching a song's stream (its own auth headers + a
    /// browser user agent CDNs accept).
    static func headers(for song: Song?) -> [HttpHeader] {
        var h = [HttpHeader(name: "User-Agent", value: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148")]
        for (field, value) in song?.httpHeaders ?? [:] { h.append(HttpHeader(name: field, value: value)) }
        return h
    }

    /// Where the core keeps cached streams (Caches: the system may purge it,
    /// which only costs a re-download). 1 GB, least recently used first.
    static func configure() {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        streamCacheConfigure(dir: caches.appendingPathComponent("LumiStreams", isDirectory: true).path,
                             capBytes: 1024 * 1024 * 1024)
    }
}
