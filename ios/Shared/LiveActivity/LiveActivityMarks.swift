import Foundation

/// What the Live Activity view did on its own, written where the app can read
/// it: the widget extension has no network and doesn't link the Rust core, so
/// it appends "<unix seconds> <kind> <key>" lines to a file in the App Group
/// and the app folds them into its `live_activity_summary`
/// (core `live_activity_ingest_marks`). Each kind+key is written once per
/// extension process (views re-render often), and the file is capped.
enum LiveActivityMarks {
    static let fileName = "LiveActivityMarks.log"
    private static let maxBytes: UInt64 = 64 * 1024
    private static let lock = NSLock()
    nonisolated(unsafe) private static var written = Set<String>()

    static var fileURL: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
            .appendingPathComponent(fileName)
    }

    static func mark(_ kind: String, key: String) {
        let id = kind + " " + key
        guard lock.withLock({ written.insert(id).inserted }), let url = fileURL else { return }
        let line = Data("\(Int(Date().timeIntervalSince1970)) \(id)\n".utf8)
        if let handle = try? FileHandle(forWritingTo: url) {
            defer { try? handle.close() }
            guard let end = try? handle.seekToEnd(), end < maxBytes else { return }
            try? handle.write(contentsOf: line)
        } else {
            try? line.write(to: url, options: .atomic)
        }
    }
}
