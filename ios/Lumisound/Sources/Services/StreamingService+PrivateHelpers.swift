import Foundation
import UIKit

extension StreamingService {

    // MARK: - Private helpers

    func makeRequest(_ path: String) -> URLRequest? {
        // Strip trailing slash from bridgeURL, ensure path starts with /
        let base = bridgeURL.trimmingCharacters(in: .init(charactersIn: "/"))
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        guard let url = URL(string: base + normalizedPath) else { return nil }
        // Lets the core send a literal '+' in this bridge's queries as %2B
        // (the bridge decodes a raw '+' as a space).
        if registeredBridgeBase != base {
            clientRegisterBridge(baseUrl: base)
            registeredBridgeBase = base
        }

        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        if !apiKey.isEmpty {
            request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        }
        // The bridge's shared-service routes (search, stream, download, lyrics,
        // trending, ...) need the signed-in session too: the app key ships
        // inside the IPA, so on its own it no longer authorizes anything.
        let accountToken = AccountService.shared?.token
        if let accountToken, !accountToken.isEmpty {
            request.setValue(accountToken, forHTTPHeaderField: "X-Account-Token")
        }
        // Core-side bridge calls (prefetch, retry queue, shared blend
        // profiles) use the same session.
        let session = "\(base)|\(apiKey)|\(accountToken ?? "")"
        if Self.coreSession != session {
            Self.coreSession = session
            bridgeSessionSet(baseUrl: base, apiKey: apiKey, accountToken: accountToken)
        }
        return request
    }

    /// Last session handed to the core (`bridgeSessionSet`).
    nonisolated(unsafe) static var coreSession = ""
}
