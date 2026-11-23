import SwiftUI

// MARK: - Round 4: Active Sessions + Notifications
//
// Both are plain bridge REST (GET /auth/sessions, GET /user/notifications) —
// see the round-4 scope note in TVBridge.swift for what was deliberately
// left out (push registration, change-password/2FA/delete-account, the
// full friend-request flow, Listen Together).

/// Parses the bridge's ISO-8601 timestamps (with fractional seconds) into a
/// short relative-ish string. Falls back to the raw string if parsing fails
/// rather than showing nothing.
private func tvFormattedTimestamp(_ iso: String?) -> String {
    guard let iso else { return "" }
    let withFraction = ISO8601DateFormatter()
    withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let withoutFraction = ISO8601DateFormatter()
    withoutFraction.formatOptions = [.withInternetDateTime]
    guard let date = withFraction.date(from: iso) ?? withoutFraction.date(from: iso) else { return iso }

    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .short
    return formatter.localizedString(for: date, relativeTo: Date())
}

// MARK: - Active Sessions

struct TVSessionsView: View {
    @ObservedObject var client: TVBridgeClient
    @ObservedObject var account: TVAccount
    let token: String

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                TVScreenTitle(title: "The Rack",
                              detail: client.sessions.isEmpty ? nil : "\(client.sessions.count) signed in",
                              eyebrow: "Zone C · Sessions")

                if client.isLoadingSessions && client.sessions.isEmpty {
                    TVLoadingState(text: "Loading sessions…")
                } else if client.sessions.isEmpty {
                    TVEmptyState(systemImage: "rectangle.stack.badge.person.crop",
                                 title: "No active sessions")
                } else {
                    // An equipment rack: one 1U unit per signed-in device,
                    // between the rack's rails.
                    VStack(spacing: 6) {
                        ForEach(client.sessions) { session in
                            sessionRow(session)
                        }
                    }
                    .padding(.horizontal, 44)
                    .padding(.vertical, 20)
                    .background {
                        HStack {
                            TVRackRail()
                            Spacer()
                            TVRackRail()
                        }
                    }
                    .frame(maxWidth: 1400, alignment: .leading)
                    .padding(.horizontal, TVMetrics.margin)
                }
            }
            .padding(.bottom, 80)
        }
        .tvAmbientBackground()
        .task {
            if client.sessions.isEmpty { await client.fetchSessions(token: token) }
        }
    }

    private func sessionRow(_ session: TVSession) -> some View {
        HStack(spacing: 24) {
            // Power LED: green for this Apple TV, the theme's light otherwise.
            Circle()
                .fill(session.isCurrent ? Color(red: 0.3, green: 1, blue: 0.5) : TVPalette.violet)
                .frame(width: 12, height: 12)
                .shadow(color: session.isCurrent ? Color(red: 0.3, green: 1, blue: 0.5) : TVPalette.violet, radius: 6)
            Image(systemName: session.isCurrent ? "appletv.fill" : "iphone")
                .font(.system(size: 26, weight: .semibold))
                .foregroundStyle(.white.opacity(0.85))
                .frame(width: 46)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 12) {
                    Text(session.deviceName ?? "Unknown Device")
                        .font(.system(size: 24, weight: .bold, design: .monospaced))
                        .lineLimit(1)
                    if session.isCurrent {
                        Text("THIS UNIT")
                            .font(.system(size: 13, weight: .black, design: .monospaced))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(Capsule().fill(TVPalette.violet.opacity(0.35)))
                    }
                }
                Text("Online since \(tvFormattedTimestamp(session.createdAt))")
                    .font(.system(size: 17, weight: .medium, design: .monospaced))
                    .foregroundStyle(TVPalette.textTertiary)
            }
            Spacer(minLength: 20)
            Button(role: .destructive) {
                Task {
                    let wasCurrentSession = await client.revokeSession(session.tokenID, token: token)
                    if wasCurrentSession { account.logout() }
                }
            } label: {
                TVPillLabel(title: session.isCurrent ? "Power Off" : "Pull Unit",
                            systemImage: "power", style: .secondary)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()
        }
        .padding(.horizontal, 26)
        .padding(.vertical, 16)
        .background {
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(LinearGradient(colors: [Color(white: 0.2), Color(white: 0.11)], startPoint: .top, endPoint: .bottom))
        }
        .overlay(alignment: .leading) { rackScrews }
        .overlay(alignment: .trailing) { rackScrews }
    }

    private var rackScrews: some View {
        VStack(spacing: 30) {
            Circle().fill(Color(white: 0.4)).frame(width: 9, height: 9)
            Circle().fill(Color(white: 0.4)).frame(width: 9, height: 9)
        }
        .padding(.horizontal, 8)
    }
}

/// A rack rail: a vertical strip with mounting holes.
private struct TVRackRail: View, Equatable {
    var body: some View {
        VStack(spacing: 14) {
            ForEach(0..<40, id: \.self) { _ in
                RoundedRectangle(cornerRadius: 2).fill(Color.black.opacity(0.6)).frame(width: 10, height: 10)
            }
        }
        .frame(width: 30)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Color(white: 0.18))
        .clipped()
    }
}

// MARK: - Notifications

struct TVNotificationsView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String

    private var unreadCount: Int { client.notifications.filter(\.isUnread).count }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                HStack(alignment: .bottom) {
                    TVScreenTitle(title: "The Mailroom",
                                  detail: unreadCount > 0 ? "\(unreadCount) unread" : nil,
                                  eyebrow: "Zone B · Notifications")
                    if unreadCount > 0 {
                        Button {
                            Task { await client.markAllNotificationsRead(token: token) }
                        } label: {
                            TVPillLabel(title: "Mark All Read", systemImage: "checkmark", style: .secondary)
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                        .padding(.trailing, TVMetrics.margin)
                    }
                }

                if client.isLoadingNotifications && client.notifications.isEmpty {
                    TVLoadingState(text: "Loading notifications…")
                } else if client.notifications.isEmpty {
                    TVEmptyState(systemImage: "bell.slash",
                                 title: "You're all caught up",
                                 message: "Notifications from Lumisound and your friends show up here.")
                } else {
                    // Postcards, two to a row; select one to mark it read.
                    LazyVGrid(columns: [GridItem(.fixed(820), spacing: 40), GridItem(.fixed(820), spacing: 40)],
                              alignment: .leading, spacing: 40) {
                        ForEach(client.notifications) { note in
                            Button {
                                guard note.isUnread else { return }
                                Task { await client.markNotificationRead(note.id, token: token) }
                            } label: {
                                TVNotificationRowLabel(note: note)
                            }
                            .buttonStyle(.plain)
                            .focusEffectDisabled()
                        }
                    }
                    .padding(.horizontal, TVMetrics.margin)
                }
            }
            .padding(.bottom, 80)
        }
        .tvAmbientBackground()
        .task {
            if client.notifications.isEmpty { await client.fetchNotifications(token: token) }
        }
    }
}

/// A notification as a postcard: message on the left, a stamp top-right —
/// a fresh coloured stamp while unread, franked with a postmark once read.
private struct TVNotificationRowLabel: View {
    let note: TVNotification
    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme

    /// Tilt per card, stable per id.
    private var tilt: Double {
        var h: UInt32 = 2_166_136_261
        for s in note.id.description.unicodeScalars { h = (h ^ s.value) &* 16_777_619 }
        return Double(Int(h % 5) - 2) * 0.6
    }

    var body: some View {
        HStack(alignment: .top, spacing: 26) {
            VStack(alignment: .leading, spacing: 10) {
                Text(note.title ?? "Notification")
                    .font(.system(size: 26, weight: .bold, design: theme.design))
                    .foregroundStyle(Color.black.opacity(0.85))
                    .lineLimit(2)
                if let body = note.body, !body.isEmpty {
                    Text(body)
                        .font(.system(size: 21, design: theme.design))
                        .foregroundStyle(Color.black.opacity(0.65))
                        .lineLimit(4)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
                Text(tvFormattedTimestamp(note.createdAt))
                    .font(.system(size: 16, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Color.black.opacity(0.45))
            }
            Spacer(minLength: 0)
            ZStack {
                RoundedRectangle(cornerRadius: 4)
                    .fill(note.isUnread ? AnyShapeStyle(TVPalette.brand) : AnyShapeStyle(Color(white: 0.75)))
                    .frame(width: 92, height: 110)
                    .overlay {
                        RoundedRectangle(cornerRadius: 4)
                            .strokeBorder(Color.white, style: StrokeStyle(lineWidth: 4, dash: [3, 3]))
                    }
                Image(systemName: note.isUnread ? "bell.badge.fill" : "bell")
                    .font(.system(size: 34, weight: .semibold))
                    .foregroundStyle(.white)
                if !note.isUnread {
                    // Postmark
                    Circle()
                        .strokeBorder(Color.black.opacity(0.45), lineWidth: 3)
                        .frame(width: 96, height: 96)
                        .overlay(Text("READ").font(.system(size: 14, weight: .black, design: .monospaced)).foregroundStyle(Color.black.opacity(0.45)))
                        .offset(x: -30, y: 24)
                }
            }
        }
        .padding(28)
        .frame(width: 820, height: 250, alignment: .topLeading)
        .background {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color(red: 0.97, green: 0.95, blue: 0.9))
        }
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(isFocused ? AnyShapeStyle(TVPalette.brand) : AnyShapeStyle(Color.clear), lineWidth: 4)
        }
        .opacity(note.isUnread || isFocused ? 1 : 0.75)
        .shadow(color: isFocused ? TVPalette.violet.opacity(0.5) : .black.opacity(0.4), radius: isFocused ? 30 : 12, y: 10)
        .rotationEffect(.degrees(isFocused ? 0 : tilt))
        .scaleEffect(isFocused ? 1.04 : 1)
        .animation(.spring(response: 0.3, dampingFraction: 0.78), value: isFocused)
    }
}

// MARK: - Friends Listening Now
//
// Deliberately minimal — a passive "who's playing something right now" strip,
// not a full friends/social tab. A shared living-room screen showing a whole
// social graph is a worse fit than on a personal phone; this only ever renders
// when it has something to show (see TVBridgeClient.fetchFriendsListening).

struct TVFriendsListeningCard: View {
    let friendsListening: [TVFriendListening]

    var body: some View {
        if !friendsListening.isEmpty {
            VStack(spacing: 12) {
                ForEach(friendsListening) { entry in
                    HStack(spacing: 20) {
                        TVGeneratedArt(seed: entry.friend.name, systemImage: "person.fill")
                            .frame(width: 60, height: 60)
                            .clipShape(Circle())
                            .overlay {
                                Text(String(entry.friend.name.prefix(1)).uppercased())
                                    .font(.system(size: 26, weight: .heavy, design: TVTheme.current.design))
                            }
                        VStack(alignment: .leading, spacing: 3) {
                            Text(entry.friend.name)
                                .font(TVType.rowTitle)
                                .lineLimit(1)
                            if let title = entry.presence.nowPlayingTitle, !title.isEmpty {
                                Text("\(title)\(entry.presence.nowPlayingArtist.map { " — \($0)" } ?? "")")
                                    .font(TVType.rowDetail)
                                    .foregroundStyle(TVPalette.textSecondary)
                                    .lineLimit(1)
                            }
                        }
                        Spacer(minLength: 12)
                        TVPlayingMeter(isAnimating: entry.presence.isPlaying)
                    }
                    .padding(.horizontal, 22)
                    .padding(.vertical, 14)
                    .tvNeonCard(cornerRadius: 22)
                }
            }
        }
    }
}
