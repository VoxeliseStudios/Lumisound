import Foundation
import UIKit

extension StreamingService {

    // MARK: - Server Library URLs

    /// `&ticket=…` when a stream ticket is cached: the bridge now requires a
    /// session or ticket for the server library, and players and image views
    /// do not carry our headers.
    fileprivate static var ticketQuery: String {
        guard let t = freshCachedTicket.map({ urlComponent(s: $0) }) else { return "" }
        return "&ticket=\(t)"
    }

    /// Returns the direct stream URL for a server library track.
    func serverStreamURL(for track: ServerTrack) -> URL? {
        guard let encoded = (urlComponent(s: track.serverPath) as String?) else {
            return nil
        }
        let base = bridgeURL.trimmingCharacters(in: .init(charactersIn: "/"))
        return URL(string: "\(base)/api/library/server/stream?path=\(encoded)\(Self.ticketQuery)")
    }

    /// Returns the artwork URL for a server library track.
    func serverArtworkURL(for track: ServerTrack) -> URL? {
        guard track.hasArtwork,
              let encoded = (urlComponent(s: track.serverPath) as String?) else {
            return nil
        }
        let base = bridgeURL.trimmingCharacters(in: .init(charactersIn: "/"))
        return URL(string: "\(base)/api/library/server/artwork?path=\(encoded)\(Self.ticketQuery)")
    }
}
