import SwiftUI

// MARK: - FriendsListView
//
// The main Friends hub, redesigned (2026-07-21) around a Discord-style
// segmented structure instead of one long flat scroll: Friends / Discover /
// Requests / Activity, switched via a horizontal chip picker rather than a
// stack of NavigationLink rows. Every capability from the original single-
// screen version is preserved — search/add by username and mutual-friend
// suggestions now live under "Discover", accept/decline/cancel requests
// under "Requests" (still badge-counted, now on the picker chip itself),
// the friends-only activity feed and blocked-users management are still
// reachable (Activity segment / toolbar menu) — plus five new features
// woven directly into the new structure: private nicknames, custom friend
// tags/groups (with filter chips), a weekly activity leaderboard,
// "listening together" (friends playing the exact same track right now),
// and friendiversary callouts. See `FriendsSegments.swift` for the
// Discover/Requests/Activity segment bodies.
//
// Owns the standing "poll my friends' presence" timer for as long as this
// screen is visible — PresenceService.startFriendsPolling/stopFriendsPolling
// is intentionally screen-scoped (not app-wide) since there's no reason to
// keep refreshing friends' online status when no friends UI is on screen.
struct FriendsListView: View {
    @EnvironmentObject private var social: SocialService
    @EnvironmentObject private var account: AccountService
    @EnvironmentObject private var library: LibraryManager
    @EnvironmentObject private var streaming: StreamingService
    @EnvironmentObject private var player: AudioPlayerManager
    @StateObject private var presenceService = PresenceService.shared
    @ObservedObject private var drops = SongDropsStore.shared

    @State private var showBlockedUsers = false
    @State private var sendSongTo: SocialFriend?

    // MARK: Presence

    private var presenceByUserId: [String: SocialPresence] {
        Dictionary(presenceService.friendsPresence.map { ($0.userId, $0) }, uniquingKeysWith: { first, _ in first })
    }

    private struct ListeningEntry: Identifiable {
        let friend: SocialFriend
        let presence: SocialPresence
        var id: String { friend.userId }
    }

    private var listeningNow: [ListeningEntry] {
        let byID = presenceByUserId
        return social.friends.compactMap { friend in
            guard let p = byID[friend.userId], p.online, p.isPlaying, p.nowPlayingTitle != nil else { return nil }
            return ListeningEntry(friend: friend, presence: p)
        }
    }

    private var onlineCount: Int {
        presenceService.friendsPresence.filter(\.online).count
    }

    /// Online first, then most recently seen, then by name — the hub shows the top of this.
    private var orderedFriends: [SocialFriend] {
        let byID = presenceByUserId
        func rank(_ f: SocialFriend) -> Int {
            guard let p = byID[f.userId] else { return 3 }
            if p.online && p.isPlaying { return 0 }
            if p.online { return 1 }
            return 2
        }
        return social.friends.sorted {
            let (a, b) = (rank($0), rank($1))
            if a != b { return a < b }
            return $0.effectiveName.localizedCaseInsensitiveCompare($1.effectiveName) == .orderedAscending
        }
    }

    // MARK: Body

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                header
                if !social.incomingRequests.isEmpty { requestsBanner }
                quickActions
                if !listeningNow.isEmpty { listeningNowSection }
                if !drops.received.isEmpty { dropsSection }
                if social.friends.isEmpty {
                    emptyState
                } else {
                    friendsSection
                }
                if social.leaderboard.count >= 2 { leaderboardSection }
            }
            .padding(.top, 8)
            .padding(.bottom, 150)
        }
        .scrollIndicators(.hidden)
        .background(AppTheme.background.ignoresSafeArea())
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                LumiMenu {
                    Button { showBlockedUsers = true } label: {
                        Label("Blocked Users", systemImage: "hand.raised")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .foregroundStyle(AppTheme.textPrimary)
                }
            }
        }
        .refreshable { await reload(force: true) }
        // `.onAppear` (not `.task`): Friends is a persistent tab, and it has
        // to refresh every time it comes back on screen, not only the first.
        .onAppear {
            presenceService.startFriendsPolling(account: account)
            Task { await reload(force: false) }
        }
        .onDisappear { presenceService.stopFriendsPolling() }
        .onChange(of: presenceService.friendsPresence) { _ in
            Task { await social.fetchListeningTogether() }
        }
        .sheet(isPresented: $showBlockedUsers) {
            NavigationStack { BlockedUsersView() }
        }
        .sheet(item: $sendSongTo) { friend in
            SendSongSheet(friendID: friend.userId, friendName: friend.effectiveName)
        }
    }

    private func reload(force: Bool) async {
        async let a: Void = social.fetchFriends()
        async let b: Void = social.fetchFriendRequests()
        async let c: Void = social.fetchLeaderboard()
        async let d: Void = drops.refresh(force: force)
        async let e: Void = social.fetchListeningTogether()
        _ = await (a, b, c, d, e)
    }

    // MARK: Header

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Friends")
                    .font(.system(size: 38, weight: .bold, design: .serif))
                    .foregroundStyle(AppTheme.textPrimary)
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(AppTheme.textSecondary)
            }
            Spacer()
            NavigationLink {
                DiscoverSegmentView().navigationTitle("Find People")
            } label: {
                Image(systemName: "person.badge.plus")
                    .font(.system(size: 17, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 44, height: 44)
                    .background(Circle().fill(AppTheme.dynamicAccentGradient))
            }
            .buttonStyle(PressableButtonStyle())
            .accessibilityLabel("Find people")
        }
        .padding(.horizontal, 20)
    }

    private var subtitle: String {
        if social.friends.isEmpty { return "Your people, and what they're playing" }
        let listening = listeningNow.count
        var parts = ["\(social.friends.count) friend\(social.friends.count == 1 ? "" : "s")"]
        if onlineCount > 0 { parts.append("\(onlineCount) online") }
        if listening > 0 { parts.append("\(listening) listening") }
        return parts.joined(separator: " · ")
    }

    // MARK: Requests

    private var requestsBanner: some View {
        NavigationLink {
            RequestsSegmentView().navigationTitle("Requests")
        } label: {
            HStack(spacing: 12) {
                HStack(spacing: -12) {
                    ForEach(social.incomingRequests.prefix(3)) { request in
                        SocialAvatarView(userId: request.userId, size: 36)
                            .overlay(Circle().stroke(AppTheme.background, lineWidth: 2))
                    }
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(social.incomingRequests.count) friend request\(social.incomingRequests.count == 1 ? "" : "s")")
                        .font(.subheadline.weight(.bold))
                        .foregroundStyle(AppTheme.textPrimary)
                    Text(social.incomingRequests.prefix(2).map { $0.displayName ?? $0.username }.joined(separator: ", "))
                        .font(.caption)
                        .foregroundStyle(AppTheme.textSecondary)
                        .lineLimit(1)
                }
                Spacer()
                Image(systemName: "chevron.right").font(.caption.weight(.bold)).foregroundStyle(AppTheme.textSecondary)
            }
            .padding(14)
            .cellSurface(in: RoundedRectangle(cornerRadius: 20, style: .continuous), fallback: AppTheme.dynamicAccent.opacity(0.16))
        }
        .buttonStyle(PressableButtonStyle())
        .padding(.horizontal, 16)
    }

    // MARK: Quick actions

    private var quickActions: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 10) {
                quickAction("Song Drops", icon: "paperplane.fill", badge: drops.unseenCount) {
                    SongDropsInboxView()
                }
                quickAction("Activity", icon: "chart.bar.xaxis", badge: 0) {
                    ActivitySegmentView().navigationTitle("Activity")
                }
                quickAction("All Friends", icon: "person.2.fill", badge: 0) {
                    FriendsSegmentView(presenceService: presenceService).navigationTitle("All Friends")
                }
                quickAction("Find People", icon: "sparkle.magnifyingglass", badge: 0) {
                    DiscoverSegmentView().navigationTitle("Find People")
                }
            }
            .padding(.horizontal, 16)
        }
    }

    private func quickAction<Destination: View>(_ title: String, icon: String, badge: Int, @ViewBuilder destination: @escaping () -> Destination) -> some View {
        NavigationLink {
            destination()
        } label: {
            HStack(spacing: 8) {
                Image(systemName: icon).font(.subheadline.weight(.bold)).foregroundStyle(AppTheme.dynamicAccent)
                Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(AppTheme.textPrimary)
                if badge > 0 {
                    Text("\(badge)")
                        .font(.caption2.weight(.heavy))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(AppTheme.error))
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .cellSurface(in: Capsule(), fallback: AppTheme.surface.opacity(0.7))
        }
        .buttonStyle(PressableButtonStyle())
    }

    // MARK: Listening now

    private var listeningNowSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionTitle("Listening now", icon: "dot.radiowaves.left.and.right")
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(spacing: 14) {
                    ForEach(listeningNow) { entry in
                        listeningCard(entry.friend, entry.presence)
                    }
                }
                .padding(.horizontal, 16)
            }
        }
    }

    private func listeningCard(_ friend: SocialFriend, _ presence: SocialPresence) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            NavigationLink {
                PublicProfileView(userId: friend.userId)
            } label: {
                VStack(alignment: .leading, spacing: 10) {
                    ZStack(alignment: .bottomLeading) {
                        Group {
                            if let url = presence.nowPlayingArtworkURL.flatMap(URL.init(string:)) {
                                CachedAsyncImage(url: url) { phase in
                                    switch phase {
                                    case .success(let image): image.resizable().aspectRatio(contentMode: .fill)
                                    default: artworkPlaceholder
                                    }
                                }
                            } else {
                                artworkPlaceholder
                            }
                        }
                        .frame(width: 210, height: 130)
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                        LinearGradient(colors: [.clear, .black.opacity(0.7)], startPoint: .center, endPoint: .bottom)
                            .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                        HStack(spacing: 8) {
                            SocialAvatarView(userId: friend.userId, size: 30)
                                .overlay(Circle().stroke(AppTheme.success, lineWidth: 2))
                            Text(friend.effectiveName)
                                .font(.caption.weight(.bold))
                                .foregroundStyle(.white)
                                .lineLimit(1)
                        }
                        .padding(10)
                    }
                    .frame(width: 210, height: 130)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(presence.nowPlayingTitle ?? "")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(AppTheme.textPrimary)
                            .lineLimit(1)
                        Text(presence.nowPlayingArtist ?? " ")
                            .font(.caption)
                            .foregroundStyle(AppTheme.textSecondary)
                            .lineLimit(1)
                    }
                }
            }
            .buttonStyle(PressableButtonStyle())
            VibeBar(friendID: friend.userId, title: presence.nowPlayingTitle ?? "", artist: presence.nowPlayingArtist, compact: true)
        }
        .frame(width: 210, alignment: .leading)
        .padding(12)
        .cellSurface(in: RoundedRectangle(cornerRadius: 24, style: .continuous), fallback: AppTheme.surface.opacity(0.6))
    }

    private var artworkPlaceholder: some View {
        LinearGradient(colors: [AppTheme.dynamicAccent.opacity(0.6), AppTheme.dynamicAccentSecondary.opacity(0.4)],
                       startPoint: .topLeading, endPoint: .bottomTrailing)
            .overlay(Image(systemName: "music.note").font(.largeTitle).foregroundStyle(.white.opacity(0.8)))
    }

    // MARK: Drops

    private var dropsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionTitle("Song drops", icon: "paperplane.fill", seeAll: AnyView(SongDropsInboxView()))
            VStack(spacing: 10) {
                ForEach(drops.received.prefix(3)) { drop in
                    SongDropCard(drop: drop, incoming: true) {
                        DropPlayer(library: library, streaming: streaming, player: player)
                            .play(title: drop.title, artist: drop.artist, sourceTrackID: drop.sourceTrackID)
                        Task { await drops.markSeen(drop) }
                    }
                }
            }
            .padding(.horizontal, 16)
        }
    }

    // MARK: Friends

    private var friendsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionTitle("Your friends", icon: "person.2.fill",
                         seeAll: social.friends.count > 8 ? AnyView(FriendsSegmentView(presenceService: presenceService).navigationTitle("All Friends")) : nil)
            VStack(spacing: 0) {
                let shown = Array(orderedFriends.prefix(8))
                ForEach(Array(shown.enumerated()), id: \.element.userId) { index, friend in
                    friendRow(friend)
                    if index < shown.count - 1 {
                        Divider().background(Color.white.opacity(0.06)).padding(.leading, 74)
                    }
                }
            }
            .padding(.vertical, 4)
            .cellSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous), fallback: AppTheme.surface.opacity(0.55))
            .padding(.horizontal, 16)
        }
    }

    private func friendRow(_ friend: SocialFriend) -> some View {
        let presence = presenceByUserId[friend.userId]
        return NavigationLink {
            PublicProfileView(userId: friend.userId)
        } label: {
            HStack(spacing: 12) {
                ZStack(alignment: .bottomTrailing) {
                    SocialAvatarView(userId: friend.userId, size: 46)
                    Circle()
                        .fill(presence?.online == true ? AppTheme.success : AppTheme.textSecondary.opacity(0.5))
                        .frame(width: 13, height: 13)
                        .overlay(Circle().stroke(AppTheme.background, lineWidth: 2.5))
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(friend.effectiveName)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(AppTheme.textPrimary)
                        .lineLimit(1)
                    Text(statusLine(presence))
                        .font(.caption)
                        .foregroundStyle(presence?.isPlaying == true ? AppTheme.dynamicAccent : AppTheme.textSecondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                Button { sendSongTo = friend } label: {
                    Image(systemName: "paperplane")
                        .font(.footnote.weight(.bold))
                        .foregroundStyle(AppTheme.dynamicAccent)
                        .frame(width: 34, height: 34)
                        .background(Circle().fill(AppTheme.dynamicAccent.opacity(0.14)))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Send \(friend.effectiveName) a song")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func statusLine(_ presence: SocialPresence?) -> String {
        guard let presence else { return "Offline" }
        if presence.online, presence.isPlaying, let title = presence.nowPlayingTitle {
            return "♫ \(title)" + (presence.nowPlayingArtist.map { " · \($0)" } ?? "")
        }
        if presence.online { return "Online" }
        if let seen = presence.lastSeenAt.flatMap(parseServerDate) {
            return "Seen \(seen.formatted(.relative(presentation: .named)))"
        }
        return "Offline"
    }

    // MARK: Leaderboard

    private var leaderboardSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            sectionTitle("This week's top listeners", icon: "trophy.fill", seeAll: AnyView(ActivitySegmentView().navigationTitle("Activity")))
            HStack(alignment: .bottom, spacing: 10) {
                let top = Array(social.leaderboard.prefix(3))
                // Podium order: 2nd, 1st, 3rd.
                ForEach([1, 0, 2].filter { $0 < top.count }, id: \.self) { i in
                    podiumColumn(top[i], place: i)
                }
            }
            .padding(16)
            .cellSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous), fallback: AppTheme.surface.opacity(0.55))
            .padding(.horizontal, 16)
        }
    }

    private func podiumColumn(_ entry: FriendLeaderboardEntry, place: Int) -> some View {
        let heights: [CGFloat] = [74, 52, 38]
        let medals = ["🥇", "🥈", "🥉"]
        return VStack(spacing: 6) {
            SocialAvatarView(userId: entry.userId, size: place == 0 ? 56 : 46)
            Text(entry.displayName ?? entry.username)
                .font(.caption.weight(.semibold))
                .foregroundStyle(AppTheme.textPrimary)
                .lineLimit(1)
            Text("\(entry.playCount) plays")
                .font(.caption2)
                .foregroundStyle(AppTheme.textSecondary)
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(place == 0 ? AnyShapeStyle(AppTheme.dynamicAccentGradient) : AnyShapeStyle(AppTheme.dynamicAccent.opacity(0.22)))
                .frame(height: heights[place])
                .overlay(Text(medals[place]).font(.title3))
        }
        .frame(maxWidth: .infinity)
    }

    // MARK: Empty

    private var emptyState: some View {
        VStack(spacing: 14) {
            Image(systemName: "person.2.wave.2.fill")
                .font(.system(size: 44, weight: .semibold))
                .foregroundStyle(AppTheme.dynamicAccentGradient)
            Text("Find your people")
                .font(.system(.title2, design: .serif).weight(.bold))
                .foregroundStyle(AppTheme.textPrimary)
            Text("See what friends are playing, send each other songs, react with a vibe, and match your taste.")
                .font(.subheadline)
                .foregroundStyle(AppTheme.textSecondary)
                .multilineTextAlignment(.center)
            NavigationLink {
                DiscoverSegmentView().navigationTitle("Find People")
            } label: {
                Label("Discover people", systemImage: "sparkle.magnifyingglass")
                    .font(.subheadline.weight(.bold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 22)
                    .padding(.vertical, 12)
                    .background(Capsule().fill(AppTheme.dynamicAccentGradient))
            }
            .buttonStyle(PressableButtonStyle())
        }
        .frame(maxWidth: .infinity)
        .padding(28)
        .cellSurface(in: RoundedRectangle(cornerRadius: 26, style: .continuous), fallback: AppTheme.surface.opacity(0.55))
        .padding(.horizontal, 16)
    }

    // MARK: Pieces

    private func sectionTitle(_ title: String, icon: String, seeAll: AnyView? = nil) -> some View {
        HStack(spacing: 8) {
            Image(systemName: icon).font(.footnote.weight(.bold)).foregroundStyle(AppTheme.dynamicAccent)
            Text(title)
                .font(.system(.title3, design: .serif).weight(.bold))
                .foregroundStyle(AppTheme.textPrimary)
            Spacer()
            if let seeAll {
                NavigationLink { seeAll } label: {
                    Text("See all").font(.subheadline.weight(.semibold)).foregroundStyle(AppTheme.dynamicAccent)
                }
            }
        }
        .padding(.horizontal, 20)
    }
}

// MARK: - Presence grouping (Friends segment)

/// Discord-style buckets for the Friends segment's grouped list — computed
/// purely from already-fetched presence data, no extra network calls.
private enum PresenceGroup: String, CaseIterable {
    case listeningNow = "Listening Now"
    case online = "Online"
    case recentlyActive = "Recently Active"
    case offline = "Offline"
}

private struct FriendGroupSection: Identifiable {
    let kind: PresenceGroup
    let friends: [SocialFriend]
    var id: String { kind.rawValue }
}

/// Shared — called for every friend row on every render.
private let relativeTimeFormatter: RelativeDateTimeFormatter = {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .abbreviated
    return formatter
}()

private func relativeTimeString(_ date: Date) -> String {
    relativeTimeFormatter.localizedString(for: date, relativeTo: Date())
}

/// Shared small-caps section header used across every Friends-tab segment
/// (this file and FriendsSegments.swift).
func friendsSectionHeader(_ text: String) -> some View {
    Text(text.uppercased())
        .font(AppTheme.bodyFont(size: 11))
        .foregroundStyle(AppTheme.textSecondary)
        .kerning(0.8)
}

// MARK: - FriendsSegmentView
//
// The redesigned core: grouped-by-presence friend list (Listening Now /
// Online / Recently Active / Offline), richer cards (nickname, tags, inline
// now-playing / last-seen), tag filter chips, an in-list search filter, and
// two new-feature banners up top (friendiversary callouts, listening
// together).
struct FriendsSegmentView: View {
    @EnvironmentObject private var social: SocialService
    @ObservedObject var presenceService: PresenceService

    @State private var searchText = ""
    @State private var selectedTag: String? = nil
    @State private var manageSheetFriend: SocialFriend? = nil

    private var presenceByUserId: [String: SocialPresence] {
        Dictionary(presenceService.friendsPresence.map { ($0.userId, $0) }, uniquingKeysWith: { first, _ in first })
    }

    private func groupKind(_ friend: SocialFriend) -> PresenceGroup {
        guard let presence = presenceByUserId[friend.userId] else { return .offline }
        if presence.online && presence.isPlaying { return .listeningNow }
        if presence.online { return .online }
        if let lastSeen = presence.lastSeenAt.flatMap(parseServerDate), Date().timeIntervalSince(lastSeen) < 86_400 {
            return .recentlyActive
        }
        return .offline
    }

    private var groupedFriends: [FriendGroupSection] {
        let trimmedQuery = searchText.trimmingCharacters(in: .whitespaces)
        let filtered = social.friends.filter { friend in
            (selectedTag == nil || friend.tags.contains(selectedTag!)) &&
            (trimmedQuery.isEmpty ||
             friend.effectiveName.localizedCaseInsensitiveContains(trimmedQuery) ||
             friend.username.localizedCaseInsensitiveContains(trimmedQuery))
        }
        let grouped = Dictionary(grouping: filtered, by: groupKind)
        return PresenceGroup.allCases.compactMap { kind in
            guard let items = grouped[kind], !items.isEmpty else { return nil }
            let sorted = items.sorted {
                $0.effectiveName.localizedCaseInsensitiveCompare($1.effectiveName) == .orderedAscending
            }
            return FriendGroupSection(kind: kind, friends: sorted)
        }
    }

    var body: some View {
        List {
            Section {
                friendiversaryBanner
                listeningTogetherBanner
            }
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)

            Section {
                tagFilterChips
                    .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                searchField
            }
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)

            if social.friends.isEmpty {
                Section {
                    Text("No friends yet — head to Discover to search for someone.")
                        .font(AppTheme.bodyFont(size: 13))
                        .foregroundStyle(AppTheme.textSecondary)
                }
                .listRowBackground(AppTheme.surface)
            } else if groupedFriends.isEmpty {
                Section {
                    Text("No friends match this filter.")
                        .font(AppTheme.bodyFont(size: 13))
                        .foregroundStyle(AppTheme.textSecondary)
                }
                .listRowBackground(AppTheme.surface)
            } else {
                ForEach(groupedFriends) { section in
                    Section {
                        ForEach(section.friends) { friend in
                            friendRow(friend)
                        }
                    } header: {
                        friendsSectionHeader("\(section.kind.rawValue) (\(section.friends.count))")
                    }
                    .listRowBackground(AppTheme.surface)
                }
            }
        }
        .scrollContentBackground(.hidden)
        .refreshable {
            await social.fetchFriends()
            await social.fetchFriendTagNames()
        }
        .sheet(item: $manageSheetFriend) { friend in
            FriendManageSheet(friend: friend)
        }
    }

    // MARK: New feature: friendiversary callouts

    @ViewBuilder
    private var friendiversaryBanner: some View {
        let matches = social.friends.compactMap { friend -> (SocialFriend, Int)? in
            friend.friendiversaryYears.map { (friend, $0) }
        }
        if !matches.isEmpty {
            ProfileInfoCard(title: "Friendiversary", icon: "gift.fill", tint: AppTheme.dynamicAccent) {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(matches, id: \.0.userId) { friend, years in
                        HStack(spacing: 8) {
                            SocialAvatarView(userId: friend.userId, size: 28)
                            Text("🎉 \(years) year\(years == 1 ? "" : "s") of friendship with \(friend.effectiveName) today!")
                                .font(AppTheme.bodyFont(size: 13))
                                .foregroundStyle(AppTheme.textPrimary)
                        }
                    }
                }
            }
        }
    }

    // MARK: New feature: listening together

    @ViewBuilder
    private var listeningTogetherBanner: some View {
        if !social.listeningTogether.isEmpty, let track = social.listeningTogetherTrack {
            ProfileInfoCard(title: "Listening Together", icon: "waveform", tint: AppTheme.dynamicAccent) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(
                        social.listeningTogether.count == 1
                            ? "1 friend is also listening to"
                            : "\(social.listeningTogether.count) friends are also listening to"
                    )
                    .font(AppTheme.bodyFont(size: 12))
                    .foregroundStyle(AppTheme.textSecondary)
                    Text([track.title, track.artist].compactMap { $0 }.joined(separator: " — "))
                        .font(AppTheme.bodyFont(size: 14).weight(.semibold))
                        .foregroundStyle(AppTheme.textPrimary)
                        .lineLimit(1)
                    HStack(spacing: -8) {
                        ForEach(social.listeningTogether.prefix(6)) { user in
                            NavigationLink(destination: PublicProfileView(userId: user.userId)) {
                                SocialAvatarView(userId: user.userId, size: 30)
                                    .overlay(Circle().stroke(AppTheme.background, lineWidth: 2))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    // MARK: Tag filter chips

    @ViewBuilder
    private var tagFilterChips: some View {
        if !social.friendTagNames.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    tagChip(label: "All", isSelected: selectedTag == nil) { selectedTag = nil }
                    ForEach(social.friendTagNames, id: \.self) { tag in
                        tagChip(label: tag, isSelected: selectedTag == tag) {
                            selectedTag = (selectedTag == tag) ? nil : tag
                        }
                    }
                }
            }
        }
    }

    private func tagChip(label: String, isSelected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(label)
                .font(AppTheme.bodyFont(size: 12).weight(isSelected ? .semibold : .regular))
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .foregroundStyle(isSelected ? .white : AppTheme.textPrimary)
                .background(Capsule().fill(isSelected ? AppTheme.dynamicAccent : AppTheme.surface))
        }
        .buttonStyle(.plain)
    }

    private var searchField: some View {
        HStack {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(AppTheme.textSecondary)
            TextField("Filter friends", text: $searchText)
                .autocorrectionDisabled()
                .foregroundStyle(AppTheme.textPrimary)
            if !searchText.isEmpty {
                Button {
                    searchText = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(AppTheme.textSecondary)
                }
                .buttonStyle(.plain)
            }
        }
    }

    // MARK: Friend row

    private func friendRow(_ friend: SocialFriend) -> some View {
        let presence = presenceByUserId[friend.userId]
        return HStack(spacing: 12) {
            NavigationLink(destination: PublicProfileView(userId: friend.userId)) {
                HStack(spacing: 12) {
                    SocialAvatarView(userId: friend.userId, size: 46)
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 6) {
                            Text(friend.effectiveName)
                                .font(AppTheme.bodyFont(size: 15).weight(.medium))
                                .foregroundStyle(AppTheme.textPrimary)
                            // Only show the raw @username alongside a
                            // nickname — when there's no nickname,
                            // effectiveName already IS the display/username.
                            if let nickname = friend.nickname, !nickname.isEmpty {
                                Text("@\(friend.username)")
                                    .font(AppTheme.bodyFont(size: 11))
                                    .foregroundStyle(AppTheme.textSecondary.opacity(0.7))
                            }
                        }
                        PresenceIndicatorView(presence: presence, dotSize: 8)
                        // Richer presence: a prominent relative "last seen"
                        // caption for offline/recently-active friends,
                        // beyond PresenceIndicatorView's plain "Offline" text.
                        if !(presence?.online ?? false), let lastSeen = presence?.lastSeenAt.flatMap(parseServerDate) {
                            Text("Last seen \(relativeTimeString(lastSeen))")
                                .font(AppTheme.bodyFont(size: 11))
                                .foregroundStyle(AppTheme.textSecondary.opacity(0.7))
                        }
                        if !friend.tags.isEmpty {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 4) {
                                    ForEach(friend.tags, id: \.self) { tag in
                                        Text(tag)
                                            .font(AppTheme.bodyFont(size: 10))
                                            .padding(.horizontal, 7)
                                            .padding(.vertical, 2)
                                            .foregroundStyle(AppTheme.dynamicAccent)
                                            .background(Capsule().fill(AppTheme.dynamicAccent.opacity(0.15)))
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .buttonStyle(.plain)
            Spacer()
            Button {
                manageSheetFriend = friend
            } label: {
                Image(systemName: "ellipsis.circle")
                    .foregroundStyle(AppTheme.textSecondary)
            }
            .buttonStyle(.plain)
        }
        .swipeActions(edge: .trailing) {
            Button(role: .destructive) {
                Task { await social.removeFriend(friend.userId) }
            } label: {
                Label("Remove", systemImage: "person.badge.minus")
            }
            Button {
                Task { await social.blockUser(friend.userId) }
            } label: {
                Label("Block", systemImage: "hand.raised")
            }
            .tint(.orange)
        }
    }
}

// MARK: - FriendManageSheet
//
// New feature UI: private nickname + custom tags/groups editor for one
// friend, reached via the "..." button on their row. Nicknames/tags are
// both scoped to the caller only (see the bridge's set_friend_nickname /
// add_friend_tag doc comments) — the friend being managed never sees this.
struct FriendManageSheet: View {
    @EnvironmentObject private var social: SocialService
    @Environment(\.dismiss) private var dismiss

    let friend: SocialFriend
    @State private var nicknameText: String
    @State private var newTagText = ""
    @State private var isSaving = false

    init(friend: SocialFriend) {
        self.friend = friend
        _nicknameText = State(initialValue: friend.nickname ?? "")
    }

    /// Reads live from the service rather than the sheet's own captured
    /// `friend` snapshot, so adding/removing a tag while the sheet is open
    /// updates this list immediately after each round trip.
    private var currentTags: [String] {
        social.friends.first(where: { $0.userId == friend.userId })?.tags ?? friend.tags
    }

    private var suggestedTags: [String] {
        social.friendTagNames.filter { !currentTags.contains($0) }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Nickname") {
                    TextField(friend.displayName ?? friend.username, text: $nicknameText)
                        .autocorrectionDisabled()
                    Text("Only visible to you — \(friend.displayName ?? friend.username) never sees this.")
                        .font(AppTheme.bodyFont(size: 11))
                        .foregroundStyle(AppTheme.textSecondary)
                }

                Section("Tags") {
                    if currentTags.isEmpty {
                        Text("No tags yet.")
                            .font(AppTheme.bodyFont(size: 13))
                            .foregroundStyle(AppTheme.textSecondary)
                    } else {
                        ForEach(currentTags, id: \.self) { tag in
                            HStack {
                                Text(tag).foregroundStyle(AppTheme.textPrimary)
                                Spacer()
                                Button {
                                    Task { await social.removeFriendTag(friend.userId, tagName: tag) }
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .foregroundStyle(AppTheme.textSecondary)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }

                    HStack {
                        TextField("New tag (e.g. Close Friends)", text: $newTagText)
                            .autocorrectionDisabled()
                        Button("Add") {
                            let trimmed = newTagText.trimmingCharacters(in: .whitespaces)
                            guard !trimmed.isEmpty else { return }
                            Task {
                                await social.addFriendTag(friend.userId, tagName: trimmed)
                                newTagText = ""
                            }
                        }
                        .disabled(newTagText.trimmingCharacters(in: .whitespaces).isEmpty)
                    }

                    if !suggestedTags.isEmpty {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 6) {
                                ForEach(suggestedTags, id: \.self) { tag in
                                    Button {
                                        Task { await social.addFriendTag(friend.userId, tagName: tag) }
                                    } label: {
                                        Text(tag)
                                            .font(AppTheme.bodyFont(size: 12))
                                            .padding(.horizontal, 10)
                                            .padding(.vertical, 5)
                                            .foregroundStyle(AppTheme.dynamicAccent)
                                            .background(Capsule().fill(AppTheme.dynamicAccent.opacity(0.15)))
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle("Manage \(friend.displayName ?? friend.username)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task {
                            isSaving = true
                            let trimmed = nicknameText.trimmingCharacters(in: .whitespaces)
                            await social.setFriendNickname(friend.userId, nickname: trimmed.isEmpty ? nil : trimmed)
                            isSaving = false
                            dismiss()
                        }
                    }
                    .disabled(isSaving)
                }
            }
        }
    }
}

// MARK: - BlockedUsersView

/// Simple management sheet for GET /api/social/block — presented modally
/// from FriendsListView's toolbar menu rather than pushed, since it's an
/// infrequent "moderation" surface rather than a primary navigation
/// destination.
struct BlockedUsersView: View {
    @EnvironmentObject private var social: SocialService
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            if social.blockedUsers.isEmpty {
                Text("You haven't blocked anyone.")
                    .font(AppTheme.bodyFont(size: 13))
                    .foregroundStyle(AppTheme.textSecondary)
            } else {
                ForEach(social.blockedUsers) { user in
                    HStack {
                        SocialAvatarView(userId: user.userId, size: 36)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(user.displayName ?? user.username).foregroundStyle(AppTheme.textPrimary)
                            Text("@\(user.username)")
                                .font(AppTheme.bodyFont(size: 12))
                                .foregroundStyle(AppTheme.textSecondary)
                        }
                        Spacer()
                        Button("Unblock") {
                            Task { await social.unblockUser(user.userId) }
                        }
                        .font(AppTheme.bodyFont(size: 13))
                        .foregroundStyle(AppTheme.dynamicAccent)
                    }
                }
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Blocked Users")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Done") { dismiss() }
            }
        }
        .task { await social.fetchBlockedUsers() }
    }
}

// MARK: - FriendsActivityFeedView

/// Full-screen friends-only feed of recent plays/favorites — see GET
/// /api/social/activity/friends. Reachable from the redesigned Activity
/// segment's "View Full Activity Feed" link. Distinct from the pre-existing
/// global DiscoverView feed (which surfaces ALL opted-in users, not just
/// friends).
struct FriendsActivityFeedView: View {
    @EnvironmentObject private var social: SocialService

    var body: some View {
        List {
            if social.friendsActivity.isEmpty {
                Text("No recent activity from friends yet.")
                    .font(AppTheme.bodyFont(size: 13))
                    .foregroundStyle(AppTheme.textSecondary)
                    .listRowBackground(AppTheme.surface)
            } else {
                ForEach(social.friendsActivity) { entry in
                    HStack(spacing: 12) {
                        SocialAvatarView(userId: entry.userId, size: 36)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 4) {
                                Text(entry.displayName ?? entry.username)
                                    .fontWeight(.medium)
                                    .foregroundStyle(AppTheme.textPrimary)
                                Image(systemName: entry.isPlayed ? "play.fill" : "heart.fill")
                                    .font(.system(size: 10))
                                    .foregroundStyle(entry.isPlayed ? AppTheme.dynamicAccent : AppTheme.error)
                            }
                            Text([entry.title, entry.artist].compactMap { $0 }.joined(separator: " — "))
                                .font(AppTheme.bodyFont(size: 13))
                                .foregroundStyle(AppTheme.textSecondary)
                                .lineLimit(1)
                        }
                    }
                    .listRowBackground(AppTheme.surface)
                }
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Friends Activity")
        .navigationBarTitleDisplayMode(.inline)
        .task { await social.fetchFriendsActivity() }
        .refreshable { await social.fetchFriendsActivity() }
    }
}
