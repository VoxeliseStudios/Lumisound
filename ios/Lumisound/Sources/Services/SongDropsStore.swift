import Foundation
import SwiftUI

// MARK: - Song Drops & Vibes
//
// A "song" drop is a track sent to a friend with an optional short note; a
// "vibe" is an emoji reaction to what a friend is playing right now. The
// bridge owns the rules (friends-only, block-aware, rate limited, notifies the
// recipient — see rust/bridge/src/routes/social.rs `send_drop`); this store
// only fetches, sends and keeps the inbox for the Friends hub and profiles.

struct SongDrop: Decodable, Identifiable, Equatable {
    let id: String
    let kind: String
    let title: String
    let artist: String?
    let sourceTrackID: String?
    let trackURL: String?
    let note: String?
    let emoji: String?
    let reaction: String?
    let createdAt: String?
    let seenAt: String?
    /// The other person: the sender on a received drop, the recipient on a sent one.
    let person: SocialUserRef

    var isVibe: Bool { kind == "vibe" }
    var isUnseen: Bool { seenAt == nil }
    var date: Date? { createdAt.flatMap(parseServerDate) }

    enum CodingKeys: String, CodingKey {
        case id, kind, title, artist, note, emoji, reaction, from, to
        case sourceTrackID = "source_track_id"
        case trackURL = "track_url"
        case createdAt = "created_at"
        case seenAt = "seen_at"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = (try? c.decode(String.self, forKey: .kind)) ?? "song"
        title = try c.decode(String.self, forKey: .title)
        artist = try c.decodeIfPresent(String.self, forKey: .artist)
        sourceTrackID = try c.decodeIfPresent(String.self, forKey: .sourceTrackID)
        trackURL = try c.decodeIfPresent(String.self, forKey: .trackURL)
        note = try c.decodeIfPresent(String.self, forKey: .note)
        emoji = try c.decodeIfPresent(String.self, forKey: .emoji)
        reaction = try c.decodeIfPresent(String.self, forKey: .reaction)
        createdAt = try c.decodeIfPresent(String.self, forKey: .createdAt)
        seenAt = try c.decodeIfPresent(String.self, forKey: .seenAt)
        let ref = try c.decodeIfPresent(DropPerson.self, forKey: .from) ?? c.decode(DropPerson.self, forKey: .to)
        person = SocialUserRef(userId: ref.user_id, username: ref.username, displayName: ref.display_name, avatarURL: ref.avatar_url)
    }

    private struct DropPerson: Decodable {
        let user_id: String
        let username: String
        let display_name: String?
        let avatar_url: String?
    }

    var personName: String {
        if let d = person.displayName, !d.isEmpty { return d }
        return person.username
    }
}

private struct SongDropsResponse: Decodable {
    let received: [SongDrop]
    let sent: [SongDrop]
    let unseen: Int
}

@MainActor
final class SongDropsStore: ObservableObject {
    static let shared = SongDropsStore()

    /// The emoji the bridge accepts for vibes and drop reactions (`VIBES`).
    static let vibes = ["🔥", "❤️", "😮", "🎧", "💃", "😭", "🤯", "🫶"]

    @Published private(set) var received: [SongDrop] = []
    @Published private(set) var sent: [SongDrop] = []
    @Published private(set) var unseenCount = 0
    @Published private(set) var isLoading = false

    private var account: AccountService? { AccountService.shared }
    private var lastFetch: Date?

    /// Refreshes the inbox. `force` skips the 20 s reuse window (pull to refresh).
    func refresh(force: Bool = false) async {
        guard let account, account.isLoggedIn else { return }
        if !force, let lastFetch, Date().timeIntervalSince(lastFetch) < 20 { return }
        isLoading = true
        defer { isLoading = false }
        do {
            let data = try await account.makeRequest("/api/social/drops?limit=60")
            let r = try JSONDecoder().decode(SongDropsResponse.self, from: data)
            received = r.received
            sent = r.sent
            unseenCount = r.unseen
            lastFetch = Date()
        } catch {
            // Silent: the hub just shows what it had.
        }
    }

    private struct SendBody: Encodable {
        let to_user_id: String
        let kind: String
        let title: String
        let artist: String?
        let source_track_id: String?
        let note: String?
        let emoji: String?
    }

    /// Sends `song` to a friend with an optional note. Returns whether it went.
    @discardableResult
    func sendSong(to userId: String, title: String, artist: String?, sourceTrackID: String?, note: String?) async -> Bool {
        await send(SendBody(to_user_id: userId, kind: "song", title: title, artist: artist,
                            source_track_id: sourceTrackID, note: note, emoji: nil))
    }

    /// Reacts to what a friend is playing right now.
    @discardableResult
    func sendVibe(to userId: String, title: String, artist: String?, emoji: String) async -> Bool {
        await send(SendBody(to_user_id: userId, kind: "vibe", title: title, artist: artist,
                            source_track_id: nil, note: nil, emoji: emoji))
    }

    private func send(_ body: SendBody) async -> Bool {
        guard let account, account.isLoggedIn else { return false }
        do {
            let data = try await account.makeRequest("/api/social/drops", method: "POST", body: body)
            if let drop = try? JSONDecoder().decode(SongDrop.self, from: data) {
                sent.removeAll { $0.id == drop.id }
                sent.insert(drop, at: 0)
            }
            return true
        } catch {
            ToastCenter.shared.show(error.localizedDescription, category: .error)
            return false
        }
    }

    func markSeen(_ drop: SongDrop) async {
        guard drop.isUnseen, let account, account.isLoggedIn else { return }
        _ = try? await account.makeRequest("/api/social/drops/\(drop.id)/seen", method: "POST")
        await refresh(force: true)
    }

    func react(_ drop: SongDrop, emoji: String) async {
        guard let account, account.isLoggedIn else { return }
        struct Body: Encodable { let emoji: String }
        _ = try? await account.makeRequest("/api/social/drops/\(drop.id)/react", method: "POST", body: Body(emoji: emoji))
        await refresh(force: true)
    }

    func delete(_ drop: SongDrop) async {
        guard let account, account.isLoggedIn else { return }
        received.removeAll { $0.id == drop.id }
        sent.removeAll { $0.id == drop.id }
        _ = try? await account.makeRequest("/api/social/drops/\(drop.id)", method: "DELETE")
    }
}
