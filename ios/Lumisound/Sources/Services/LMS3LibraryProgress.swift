import Foundation

/// How much of the library is Lumisound Audio (LMS3) yet — header reads in
/// the core (`lms_library_status`), off the main thread. Refreshed by the
/// background conversion pass and when Settings shows it.
@MainActor
final class LMS3LibraryProgress: ObservableObject {
    static let shared = LMS3LibraryProgress()

    @Published private(set) var status: LmsLibraryStatus?
    private var refreshing = false

    private init() {}

    func refresh() {
        guard !refreshing, let library = LibraryManager.shared else { return }
        refreshing = true
        let paths = library.importedSongs.compactMap { $0.url }.filter(\.isFileURL).map(\.path)
        Task {
            let s = await Task.detached(priority: .utility) { lmsLibraryStatus(paths: paths) }.value
            status = s
            refreshing = false
        }
    }

    /// "4,210 of 5,731 tracks · 312 Premium" style summary.
    var summary: String {
        guard let s = status, s.total > 0 else { return "Checking…" }
        let f = NumberFormatter()
        f.numberStyle = .decimal
        let n = { (v: UInt32) in f.string(from: NSNumber(value: v)) ?? "\(v)" }
        var text = "\(n(s.lms3)) of \(n(s.total)) tracks"
        if s.premium > 0 { text += " · \(n(s.premium)) Premium" }
        return text
    }

    /// Converting in the background (paced, a batch every few minutes).
    var pending: UInt32 {
        guard let s = status else { return 0 }
        return s.legacy + s.plain
    }
}
