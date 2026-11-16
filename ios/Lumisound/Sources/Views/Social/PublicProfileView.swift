import SwiftUI
import UIKit

// MARK: - PublicProfileView
//
// Read-only view of another user's profile: avatar, bio, their chosen
// main/sub accent colors, pinned favorite tracks, live online/offline +
// now-playing presence, and contextual friend actions (add / accept-decline
// / remove / block) based on the current relationship.
struct PublicProfileView: View {
    let userId: String
    /// Set when this screen is showing the signed-in user's own profile —
    /// either the Profile tab's default view or ProfileView's "View as
    /// Public" preview sheet. Defaults to `false` so every existing call
    /// site (`PublicProfileView(userId:)`) is unaffected. Suppresses the
    /// friend-request/block controls, which would otherwise render
    /// nonsensically self-referential ("Add Friend" pointed at yourself).
    var isSelfPreview: Bool = false
    /// Non-nil when this view IS the Profile tab's primary destination
    /// (rather than the "View as Public" preview sheet reached from inside
    /// the editor): renders an "Edit Profile" button in the header's action
    /// slot instead of the "this is how you look to others" hint, and is
    /// called when that button is tapped so the caller can push the editor.
    /// Only meaningful when `isSelfPreview` is true.
    var onEditProfile: (() -> Void)? = nil

    @EnvironmentObject private var social: SocialService
    @EnvironmentObject private var account: AccountService
    @EnvironmentObject private var player: AudioPlayerManager
    @EnvironmentObject private var library: LibraryManager
    @EnvironmentObject private var streaming: StreamingService
    @StateObject private var presenceService = PresenceService.shared
    @State private var stageTab: ProfileStageTab = .showcase
    @State private var showSendSong = false

    @State private var profile: PublicSocialProfile? = nil
    @State private var presence: SocialPresence? = nil
    @State private var bannerImage: UIImage? = nil
    @State private var isLoading = true
    @State private var isActing = false
    @State private var showBlockConfirm = false
    @State private var showRemoveConfirm = false
    @State private var comments: [ProfileComment] = []
    @State private var draftComment = ""
    @State private var isPostingComment = false
    @State private var compatibility: MusicCompatibility? = nil
    /// What this library actually sounds like. Friends-only, same as
    /// compatibility — see SonicFingerprintView for why it is treated that way.
    @State private var fingerprint: SonicFingerprint? = nil
    @State private var showBlendMix = false
    // MARK: Feature: profile-customization-3
    @State private var recentlyPlayedTogether: [SharedRecentTrack] = []

    private var mainAccentColor: Color { SocialAccentPalette.color(for: profile?.mainAccentHex) ?? AppTheme.dynamicAccent }
    private var subAccentColor: Color { SocialAccentPalette.color(for: profile?.subAccentHex) ?? AppTheme.accentSoft }

    /// For the self-preview case, "online" is answered locally (are you
    /// logged in and is the app actually in the foreground right now?)
    /// rather than by round-tripping through the same `/api/social/presence`
    /// GET a genuine visitor's client uses. That GET fires the instant this
    /// view appears, racing the heartbeat POST that reports you online in
    /// the first place (PresenceService.startHeartbeat, owned separately by
    /// ContentView) — on a fresh app-open -> Profile tab tap, or after any
    /// single transient network hiccup, the fetch can land before the
    /// server has a fresh row and this screen would show YOU as offline
    /// despite the app being open right in front of you. There's no such
    /// race for the local answer: it's already known without asking anyone.
    private var isOnline: Bool {
        if isSelfPreview {
            return account.isLoggedIn && UIApplication.shared.applicationState == .active
        }
        return presence?.online ?? false
    }

    /// What to show in the "Listening To" card, if anything. Was driven by
    /// `presence` unconditionally, which meant this NEVER showed in
    /// self-preview: the presence-polling `.task` below is deliberately
    /// skipped for self-preview (same reasoning as `isOnline` above — no
    /// need to round-trip through a remote GET, racy against your own
    /// heartbeat, to learn what YOUR OWN device is currently playing), so
    /// `presence` just stayed `nil` forever there and this card silently
    /// never appeared — on the Profile tab itself, or on the "View as
    /// Public" preview, even with "Share Now Playing" turned on. Local
    /// player state for self-preview (same source `ProfileView`'s own
    /// "Listening To" section already uses, unconditionally — matching that
    /// precedent rather than re-deriving the share-now-playing check here,
    /// since this is your own screen showing your own state, not a
    /// broadcast to someone else); server-provided, privacy-gated presence
    /// for an actual visitor.
    private var displayedNowPlaying: (title: String, artist: String?, song: Song?, artworkURL: URL?)? {
        if isSelfPreview {
            guard let song = player.currentSong, player.isPlaying else { return nil }
            return (song.title, song.artist.isEmpty ? nil : song.artist, song, nil)
        }
        guard presence?.online == true, presence?.isPlaying == true,
              let title = presence?.nowPlayingTitle else { return nil }
        // No `song` for a visitor's view (only this device's own local
        // player state has one) — but the presence payload does carry a
        // best-effort artwork URL now (see SocialPresence.nowPlayingArtworkURL),
        // so NowPlayingActivityRow still shows real artwork via AsyncImage
        // instead of always falling back to the placeholder icon.
        let artworkURL = presence?.nowPlayingArtworkURL.flatMap(URL.init(string:))
        return (title, presence?.nowPlayingArtist, nil, artworkURL)
    }

    /// An incoming pending request FROM this user, if any.
    private var incomingRequest: SocialFriendRequest? {
        social.incomingRequests.first { $0.userId == userId }
    }
    /// An outgoing pending request TO this user, if any.
    private var outgoingRequest: SocialFriendRequest? {
        social.outgoingRequests.first { $0.userId == userId }
    }

    var body: some View {
        ZStack(alignment: .top) {
            Color.clear.ignoresSafeArea()
            if profile != nil {
                ProfileAccentBackgroundGlow(
                    mainAccent: mainAccentColor, subAccent: subAccentColor,
                    intensity: ProfileGlowIntensity.from(profile?.accentGlowIntensity)
                )
            }

            if isLoading {
                ProgressView().tint(AppTheme.dynamicAccent)
            } else if let profile {
                ScrollView {
                    LazyVStack(spacing: 16, pinnedViews: [.sectionHeaders]) {
                        stageHeader(profile)
                        Section {
                            stageContent(profile)
                                .transition(.opacity)
                                .id(stageTab)
                        } header: {
                            ProfileStageTabPicker(selection: $stageTab, tint: mainAccentColor)
                        }
                    }
                    .padding(.bottom, 140)
                }
                .scrollIndicators(.hidden)
            } else {
                Text("This profile isn't available.")
                    .foregroundStyle(AppTheme.textSecondary)
            }
        }
        .navigationTitle(profile?.displayName ?? profile?.username ?? "Profile")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            await loadProfile()
            // Friend-request state only feeds `friendActionControl`, which is
            // never rendered in self-preview (see its `!isSelfPreview` guard
            // above) — fetching it there is a pure wasted round trip on the
            // single most-hit screen in the app (the Profile tab).
            if !isSelfPreview {
                await social.fetchFriendRequests()
            }
        }
        // Lightweight self-cancelling poll for this one profile's presence —
        // stops automatically when the view disappears, unlike the standing
        // friends-list timer in PresenceService (which only runs while
        // FriendsListView itself is on screen). Skipped for self-preview:
        // `isOnline` above answers itself locally and never reads `presence`.
        .task {
            guard !isSelfPreview else { return }
            while !Task.isCancelled {
                presence = await presenceService.fetchPresence(userId: userId, account: account)
                try? await Task.sleep(nanoseconds: UInt64(PresenceService.friendsPollInterval * 1_000_000_000))
            }
        }
        // Guestbook comments are readable regardless of relationship (the
        // bridge only blocks a blocked-either-direction pair, matching
        // profile visibility generally) — fetched unconditionally, unlike
        // compatibility below which the bridge hard-requires friendship for.
        .task { comments = await social.fetchProfileComments(userId: userId) }
        .task {
            guard !isSelfPreview else { return }
            compatibility = await social.fetchCompatibility(userId: userId)
        }
        .task {
            // Fetched for a self-preview too: seeing your own fingerprint is
            // the whole point of having one, and the endpoint allows it.
            fingerprint = await social.fetchSonicFingerprint(userId: userId)
        }
        // Extra feature #5 — same friends-only/self-preview-skip reasoning
        // as compatibility just above (the endpoint itself 400s on a
        // self-lookup and 403s on a non-friend, so there's no point firing
        // it in either case).
        .task {
            guard !isSelfPreview else { return }
            recentlyPlayedTogether = await social.fetchRecentlyPlayedTogether(userId: userId)
        }
        .confirmationDialog("Block this user?", isPresented: $showBlockConfirm, titleVisibility: .visible) {
            Button("Block", role: .destructive) {
                Task {
                    await social.blockUser(userId)
                    await loadProfile()
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("You'll no longer see each other's profiles, and any friendship or pending request will be removed.")
        }
        .confirmationDialog("Remove this friend?", isPresented: $showRemoveConfirm, titleVisibility: .visible) {
            Button("Remove Friend", role: .destructive) {
                Task {
                    await social.removeFriend(userId)
                    await loadProfile()
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .lumiSheet(isPresented: $showSendSong) {
            SendSongSheet(friendID: userId, friendName: profile?.displayName ?? profile?.username ?? "your friend")
        }
        .lumiSheet(isPresented: $showBlendMix) {
            NavigationStack {
                BlendMixView(friendUserID: userId, friendName: profile?.displayName ?? profile?.username ?? "your friend")
            }
        }
    }

    // MARK: - Stage (2026-11-15 redesign; see ProfileStage.swift)

    @ViewBuilder
    private func stageHeader(_ profile: PublicSocialProfile) -> some View {
        VStack(spacing: 14) {
            ProfileStageHero(
                mainAccent: mainAccentColor, subAccent: subAccentColor,
                bannerImage: bannerImage,
                avatarFrame: profile.avatarFrame, avatarDecoration: profile.avatarDecoration,
                profileEffect: profile.profileEffect, isOnline: isOnline
            ) {
                SocialAvatarView(userId: userId, size: 108, fallbackFill: .clear)
            }
            ProfileIdentityBlock(
                displayName: profile.displayName ?? profile.username, username: profile.username,
                pronouns: profile.pronouns, statusEmoji: profile.statusEmoji, statusText: profile.statusText,
                discordVerified: profile.discordVerified, discordUsername: profile.discordUsername,
                tint: mainAccentColor
            )
            actionRow(profile)
            ProfileStatStrip(stats: stats(for: profile), tint: mainAccentColor)
            if let nowPlaying = displayedNowPlaying {
                onAirCard(nowPlaying, profile: profile)
            }
        }
    }

    private func stats(for profile: PublicSocialProfile) -> [ProfileStat] {
        var out: [ProfileStat] = []
        if let streak = profile.listeningStreak {
            out.append(ProfileStat(value: "\(streak.currentStreakDays)", label: "day streak", icon: "flame.fill"))
            out.append(ProfileStat(value: "\(streak.longestStreakDays)", label: "best streak", icon: "trophy.fill"))
        }
        if !profile.badges.isEmpty {
            out.append(ProfileStat(value: "\(profile.badges.count)", label: profile.badges.count == 1 ? "badge" : "badges", icon: "rosette"))
        }
        if let visitors = profile.visitorCount {
            out.append(ProfileStat(value: visitors.formatted(.number.notation(.compactName)), label: "visits", icon: "eye.fill"))
        }
        if out.count < 4, let year = profile.memberSince.flatMap(parseServerDate).map({ Calendar.current.component(.year, from: $0) }) {
            out.append(ProfileStat(value: String(year), label: "member since", icon: "calendar"))
        }
        return Array(out.prefix(4))
    }

    @ViewBuilder
    private func actionRow(_ profile: PublicSocialProfile) -> some View {
        HStack(spacing: 10) {
            if isSelfPreview {
                if let onEditProfile {
                    Button(action: onEditProfile) {
                        Label("Edit Profile", systemImage: "pencil")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                            .background(Capsule().fill(mainAccentColor.gradient))
                    }
                    .buttonStyle(PressableButtonStyle())
                } else {
                    Label("This is how your profile looks to others", systemImage: "eye")
                        .font(.caption)
                        .foregroundStyle(AppTheme.textSecondary)
                        .frame(maxWidth: .infinity)
                }
            } else {
                friendActionControl
                    .font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .background(Capsule().fill(Color.white.opacity(0.08)))
                if profile.isFriend {
                    Button { showSendSong = true } label: {
                        Label("Send a Song", systemImage: "paperplane.fill")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                            .background(Capsule().fill(mainAccentColor.gradient))
                    }
                    .buttonStyle(PressableButtonStyle())
                }
            }
            ShareLink(item: "Find me on Lumisound — @\(profile.username)") {
                Image(systemName: "square.and.arrow.up")
                    .font(.subheadline.weight(.bold))
                    .foregroundStyle(AppTheme.textPrimary)
                    .frame(width: 46, height: 46)
                    .background(Circle().fill(Color.white.opacity(0.08)))
            }
        }
        .padding(.horizontal, 16)
    }

    private func onAirCard(_ nowPlaying: (title: String, artist: String?, song: Song?, artworkURL: URL?), profile: PublicSocialProfile) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 6) {
                Circle().fill(AppTheme.error).frame(width: 8, height: 8)
                Text("ON AIR")
                    .font(.caption2.weight(.heavy))
                    .kerning(1.2)
                    .foregroundStyle(AppTheme.error)
                Spacer()
            }
            NowPlayingActivityRow(title: nowPlaying.title, artist: nowPlaying.artist, tint: mainAccentColor, song: nowPlaying.song, artworkURL: nowPlaying.artworkURL)
            if profile.isFriend, !isSelfPreview {
                VibeBar(friendID: userId, title: nowPlaying.title, artist: nowPlaying.artist, tint: mainAccentColor)
            }
        }
        .padding(16)
        .cellSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous), fallback: mainAccentColor.opacity(0.14))
        .padding(.horizontal, 16)
    }

    @ViewBuilder
    private func stageContent(_ profile: PublicSocialProfile) -> some View {
        VStack(spacing: 16) {
            switch stageTab {
            case .showcase: showcaseTab(profile)
            case .music: musicTab(profile)
            case .wall: wallTab(profile)
            }
        }
        .padding(.top, 4)
    }

    @ViewBuilder
    private func showcaseTab(_ profile: PublicSocialProfile) -> some View {
        if let bio = profile.bio, !bio.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Image(systemName: "quote.opening")
                    .font(.title2.weight(.bold))
                    .foregroundStyle(mainAccentColor)
                bioText(bio)
                    .font(.system(.title3, design: .serif))
                    .foregroundStyle(AppTheme.textPrimary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 24)
        }
        if !profile.pinnedTracks.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("On repeat")
                    .font(.system(.headline, design: .serif))
                    .foregroundStyle(AppTheme.textPrimary)
                    .padding(.horizontal, 20)
                PinnedSleeveShelf(tracks: profile.pinnedTracks, mainAccent: mainAccentColor, subAccent: subAccentColor) { track in
                    DropPlayer(library: library, streaming: streaming, player: player)
                        .play(title: track.title, artist: track.artist, sourceTrackID: track.sourceTrackID)
                }
            }
        }
        if let featuredPlaylist = profile.featuredPlaylist {
            ProfilePanel(title: "Featured Playlist", icon: "music.note.list", tint: subAccentColor) {
                FeaturedPlaylistRow(playlist: featuredPlaylist, tint: subAccentColor)
            }
        }
        if !profile.badges.isEmpty {
            ProfilePanel(title: "Badges", icon: "rosette", tint: mainAccentColor, trailing: "\(profile.badges.count)") {
                ProfileBadgeWall(badges: profile.badges, tint: mainAccentColor)
            }
        }
        if (profile.bio ?? "").isEmpty, profile.pinnedTracks.isEmpty, profile.featuredPlaylist == nil, profile.badges.isEmpty {
            stageEmpty(isSelfPreview ? "Pin tracks, feature a playlist and write a bio in Edit Profile — they show up here." : "Nothing showcased yet.")
        }
    }

    @ViewBuilder
    private func musicTab(_ profile: PublicSocialProfile) -> some View {
        if let compatibility, profile.isFriend, !isSelfPreview {
            ProfilePanel(title: "Music Match", icon: "waveform.path.ecg", tint: mainAccentColor, trailing: "\(compatibility.score)%") {
                VStack(alignment: .leading, spacing: 12) {
                    MusicCompatibilityRow(compatibility: compatibility, tint: mainAccentColor)
                    MusicMatchReasons(compatibility: compatibility)
                    Button { showBlendMix = true } label: {
                        Label("Play Blend Mix", systemImage: "play.fill")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 11)
                            .background(Capsule().fill(mainAccentColor.gradient))
                    }
                    .buttonStyle(PressableButtonStyle())
                }
            }
        }
        if !profile.topArtists.isEmpty {
            ProfilePanel(title: "Top Artists", icon: "music.mic", tint: mainAccentColor) {
                TopArtistsStrip(artists: profile.topArtists, tint: mainAccentColor)
            }
        }
        if !profile.topGenres.isEmpty {
            ProfilePanel(title: "Top Genres", icon: "guitars.fill", tint: subAccentColor) {
                GenreChipCloud(genres: profile.topGenres, tint: subAccentColor)
            }
        }
        if let fingerprint, fingerprint.available {
            ProfilePanel(title: "Listening Fingerprint", icon: "waveform.badge.magnifyingglass", tint: subAccentColor) {
                SonicFingerprintView(fingerprint: fingerprint)
            }
        }
        if let streak = profile.listeningStreak {
            ProfilePanel(title: "Listening Streak", icon: "flame.fill", tint: mainAccentColor) {
                ListeningStreakRow(streak: streak, tint: mainAccentColor)
            }
        }
        if !recentlyPlayedTogether.isEmpty, profile.isFriend, !isSelfPreview {
            ProfilePanel(title: "Played Together", icon: "person.2.wave.2.fill", tint: subAccentColor) {
                SharedRecentTracksRow(tracks: recentlyPlayedTogether, tint: subAccentColor)
            }
        }
        if profile.topArtists.isEmpty, profile.topGenres.isEmpty, fingerprint?.available != true, profile.listeningStreak == nil {
            stageEmpty("Listening stats show up here once there's some history (and Show Listening Stats is on).")
        }
    }

    @ViewBuilder
    private func wallTab(_ profile: PublicSocialProfile) -> some View {
        if profile.showGuestbook || isSelfPreview {
            ProfilePanel(title: "Guestbook", icon: "bubble.left.and.bubble.right.fill", tint: subAccentColor, trailing: comments.isEmpty ? nil : "\(comments.count)") {
                guestbookContent
            }
        }
        if let visitorCount = profile.visitorCount {
            ProfilePanel(title: "Profile Visitors", icon: "eye.fill", tint: mainAccentColor) {
                ProfileVisitorStatsRow(visitorCount: visitorCount, recentVisitors: profile.recentVisitors, tint: mainAccentColor)
            }
        }
        ProfilePanel(title: "Member Since", icon: "calendar", tint: subAccentColor) {
            MemberSinceRow(memberSince: profile.memberSince)
        }
        if profile.isFriend, !isSelfPreview {
            Button(role: .destructive) { showBlockConfirm = true } label: {
                Label("Block User", systemImage: "hand.raised.fill")
                    .font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .background(Capsule().fill(AppTheme.error.opacity(0.14)))
            }
            .padding(.horizontal, 16)
        }
    }

    private func stageEmpty(_ text: String) -> some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(AppTheme.textSecondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 30)
            .padding(.horizontal, 24)
    }

    @ViewBuilder
    private var friendActionControl: some View {
        if isActing {
            HStack { Spacer(); ProgressView().tint(AppTheme.dynamicAccent); Spacer() }
        } else if let profile, profile.isFriend {
            Button(role: .destructive) { showRemoveConfirm = true } label: {
                Label("Remove Friend", systemImage: "person.fill.badge.minus")
            }
        } else if let incoming = incomingRequest {
            HStack {
                Button {
                    act { await social.acceptRequest(incoming.id) }
                } label: {
                    Label("Accept Request", systemImage: "person.fill.checkmark")
                        .foregroundStyle(AppTheme.success)
                }
                Spacer()
                Button {
                    act { await social.declineRequest(incoming.id) }
                } label: {
                    Label("Decline", systemImage: "xmark")
                        .foregroundStyle(AppTheme.error)
                }
            }
        } else if let outgoing = outgoingRequest {
            Button {
                act { await social.cancelRequest(outgoing.id) }
            } label: {
                Label("Cancel Request", systemImage: "clock.arrow.circlepath")
                    .foregroundStyle(AppTheme.textSecondary)
            }
        } else {
            Button {
                act { _ = await social.sendFriendRequest(toUserId: userId) }
            } label: {
                Label("Add Friend", systemImage: "person.badge.plus")
                    .foregroundStyle(AppTheme.dynamicAccent)
            }
        }
    }

    /// Guestbook contents: the message list (if any), a compose box for
    /// friends (posting is friends-only server-side — a non-friend visitor
    /// or self-preview just sees the read-only list), and an empty-state
    /// hint otherwise.
    @ViewBuilder
    private var guestbookContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            if comments.isEmpty {
                Text(isSelfPreview ? "No messages yet — friends can leave one here." : "No messages yet.")
                    .font(AppTheme.bodyFont(size: 13))
                    .foregroundStyle(AppTheme.textSecondary)
            } else {
                VStack(spacing: 10) {
                    ForEach(Array(comments.enumerated()), id: \.element.id) { index, comment in
                        ProfileCommentRow(
                            comment: comment,
                            tint: subAccentColor,
                            canDelete: isSelfPreview || comment.authorUserId == account.currentUser?.id
                        ) {
                            Task { await deleteComment(comment.id) }
                        }
                        if index < comments.count - 1 {
                            Divider().background(AppTheme.textSecondary.opacity(0.15))
                        }
                    }
                }
            }

            if let profile, profile.isFriend, !isSelfPreview {
                HStack(spacing: 8) {
                    TextField("Leave a message...", text: $draftComment, axis: .vertical)
                        .font(AppTheme.bodyFont(size: 13))
                        .lineLimit(1...3)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(AppTheme.textSecondary.opacity(0.1)))
                    Button {
                        Task { await postComment() }
                    } label: {
                        if isPostingComment {
                            ProgressView().tint(subAccentColor)
                        } else {
                            Image(systemName: "paperplane.fill")
                                .foregroundStyle(subAccentColor)
                        }
                    }
                    .disabled(isPostingComment || draftComment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }

    /// "Open up ALOT more customization to the profiles" — light Markdown
    /// (bold/italic/links only, no headers/lists/block quotes — those would
    /// look out of place in a short bio) instead of always-plain text.
    /// `AttributedString(markdown:)` (not `Text(LocalizedStringKey:)`, which
    /// would treat arbitrary user text as a LOCALIZATION KEY — format
    /// specifiers like a literal "%" in someone's bio could then be
    /// misinterpreted) falls back to the raw string on a parse failure
    /// rather than showing nothing.
    private func bioText(_ bio: String) -> Text {
        if let attributed = try? AttributedString(
            markdown: bio,
            options: AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        ) {
            return Text(attributed)
        }
        return Text(bio)
    }

    private func postComment() async {
        let text = draftComment.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isPostingComment else { return }
        isPostingComment = true
        defer { isPostingComment = false }
        if let posted = await social.postProfileComment(userId: userId, body: text) {
            comments.insert(posted, at: 0)
            draftComment = ""
        }
    }

    private func deleteComment(_ commentId: String) async {
        comments.removeAll { $0.id == commentId }
        await social.deleteProfileComment(commentId)
    }

    private func act(_ operation: @escaping () async -> Void) {
        guard !isActing else { return }
        isActing = true
        Task {
            defer { isActing = false }
            await operation()
        }
    }

    private func loadProfile() async {
        isLoading = true
        // Both are independent GETs — fire them concurrently instead of back
        // to back. Gate the screen only on the profile fetch (the essential
        // data); the banner is decorative and pops in via `bannerImage`'s
        // own `@State` update whenever it lands, same as any other async
        // image in this app.
        async let bannerTask = SocialService.loadBanner(userId: userId)
        profile = await social.fetchPublicProfile(userId: userId)
        isLoading = false
        bannerImage = await bannerTask
    }
}
