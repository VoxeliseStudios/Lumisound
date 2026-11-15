import SwiftUI

// MARK: - Vibe bar
//
// One-tap emoji reactions to what a friend is playing right now. Used on the
// Friends hub's "Listening now" cards and on a friend's profile.

struct VibeBar: View {
    let friendID: String
    let title: String
    let artist: String?
    var tint: Color = AppTheme.dynamicAccent
    var compact = false

    @State private var sentEmoji: String?

    var body: some View {
        HStack(spacing: compact ? 4 : 6) {
            ForEach(SongDropsStore.vibes.prefix(compact ? 4 : 6), id: \.self) { emoji in
                Button {
                    sentEmoji = emoji
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    Task {
                        if await SongDropsStore.shared.sendVibe(to: friendID, title: title, artist: artist, emoji: emoji) {
                            ToastCenter.shared.show("Vibe sent \(emoji)", category: .success, icon: "sparkles", duration: 1.6)
                        }
                    }
                } label: {
                    Text(emoji)
                        .font(.system(size: compact ? 15 : 18))
                        .frame(width: compact ? 30 : 36, height: compact ? 30 : 36)
                        .background(
                            Circle().fill(sentEmoji == emoji ? tint.opacity(0.35) : Color.white.opacity(0.08))
                        )
                        .scaleEffect(sentEmoji == emoji ? 1.12 : 1)
                        .animation(LumiMotion.snap, value: sentEmoji)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Send \(emoji)")
            }
        }
    }
}

// MARK: - Drop playback

/// Plays a dropped track: the listener's own copy when the library has it
/// (same source id, else same title + artist), otherwise the YouTube stream
/// the sender's copy came from.
@MainActor
struct DropPlayer {
    let library: LibraryManager
    let streaming: StreamingService
    let player: AudioPlayerManager

    func play(title: String, artist: String?, sourceTrackID: String?) {
        if let local = localMatch(title: title, artist: artist, sourceTrackID: sourceTrackID) {
            player.play(song: local, in: [local], uiSource: "song_drop")
            return
        }
        guard let sourceTrackID, sourceTrackID.hasPrefix("youtube:") else {
            ToastCenter.shared.show("“\(title)” isn't in your library yet — search for it to play it.", category: .info)
            return
        }
        let vid = String(sourceTrackID.dropFirst("youtube:".count))
        let track = StreamTrack(
            id: vid, title: title, artist: artist ?? "", durationSeconds: 0,
            thumbnailURL: "https://i.ytimg.com/vi/\(vid)/hqdefault.jpg", source: "youtube",
            youtubeURL: "https://www.youtube.com/watch?v=\(vid)"
        )
        Task {
            do {
                let url = try await streaming.streamURL(for: track)
                let song = streaming.toSong(track: track, streamURL: url)
                player.play(song: song, in: [song], uiSource: "song_drop")
            } catch {
                ToastCenter.shared.show(error.localizedDescription, category: .error)
            }
        }
    }

    func localMatch(title: String, artist: String?, sourceTrackID: String?) -> Song? {
        let songs = library.allSongs
        if let sourceTrackID, let hit = songs.first(where: { $0.sourceTrackID == sourceTrackID }) {
            return hit
        }
        let t = title.lowercased()
        let a = (artist ?? "").lowercased()
        return songs.first { $0.title.lowercased() == t && (a.isEmpty || $0.artist.lowercased() == a) }
    }
}

// MARK: - Drop card

/// One received (or sent) drop: who, what, their note, play and react.
struct SongDropCard: View {
    let drop: SongDrop
    let incoming: Bool
    var onPlay: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            ZStack(alignment: .bottomTrailing) {
                SocialAvatarView(userId: drop.person.userId, size: 44)
                Text(drop.isVibe ? (drop.emoji ?? "🎧") : "🎵")
                    .font(.system(size: 14))
                    .frame(width: 22, height: 22)
                    .background(Circle().fill(AppTheme.background))
                    .offset(x: 4, y: 4)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(headline)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(AppTheme.textSecondary)
                    .lineLimit(1)
                Text(drop.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(AppTheme.textPrimary)
                    .lineLimit(2)
                if let artist = drop.artist, !artist.isEmpty {
                    Text(artist)
                        .font(.caption)
                        .foregroundStyle(AppTheme.textSecondary)
                        .lineLimit(1)
                }
                if let note = drop.note, !note.isEmpty {
                    Text("“\(note)”")
                        .font(.callout.italic())
                        .foregroundStyle(AppTheme.textPrimary.opacity(0.9))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(AppTheme.dynamicAccent.opacity(0.12)))
                        .padding(.top, 2)
                }
                if !drop.isVibe {
                    reactionRow
                }
            }
            Spacer(minLength: 0)
            if !drop.isVibe {
                Button(action: onPlay) {
                    Image(systemName: "play.fill")
                        .font(.system(size: 14, weight: .bold))
                        .foregroundStyle(.white)
                        .frame(width: 38, height: 38)
                        .background(AppTheme.dynamicAccentGradient, in: Circle())
                }
                .buttonStyle(PressableButtonStyle())
                .accessibilityLabel("Play \(drop.title)")
            }
        }
        .padding(12)
        .cellSurface(
            in: RoundedRectangle(cornerRadius: 18, style: .continuous),
            fallback: incoming && drop.isUnseen ? AppTheme.dynamicAccent.opacity(0.14) : AppTheme.surface.opacity(0.6)
        )
        .contextMenu {
            Button(role: .destructive) {
                Task { await SongDropsStore.shared.delete(drop) }
            } label: {
                Label(incoming ? "Remove" : "Unsend", systemImage: "trash")
            }
        }
    }

    private var headline: String {
        let when = drop.date.map { $0.formatted(.relative(presentation: .named)) } ?? ""
        let who = drop.personName
        if drop.isVibe {
            return incoming ? "\(who) vibed with your track · \(when)" : "You vibed with \(who) · \(when)"
        }
        return incoming ? "\(who) sent you a song · \(when)" : "Sent to \(who) · \(when)"
    }

    @ViewBuilder
    private var reactionRow: some View {
        if incoming {
            HStack(spacing: 4) {
                ForEach(SongDropsStore.vibes.prefix(5), id: \.self) { emoji in
                    Button {
                        Task { await SongDropsStore.shared.react(drop, emoji: emoji) }
                    } label: {
                        Text(emoji)
                            .font(.system(size: 14))
                            .frame(width: 28, height: 28)
                            .background(Circle().fill(drop.reaction == emoji ? AppTheme.dynamicAccent.opacity(0.35) : Color.white.opacity(0.06)))
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.top, 2)
        } else if let reaction = drop.reaction {
            Text("They reacted \(reaction)")
                .font(.caption2.weight(.semibold))
                .foregroundStyle(AppTheme.dynamicAccent)
        }
    }
}

// MARK: - Send a song

/// Pick a track from your library (the one playing is offered first), add a
/// short note, send it to a friend.
struct SendSongSheet: View {
    let friendID: String
    let friendName: String

    @EnvironmentObject private var library: LibraryManager
    @EnvironmentObject private var player: AudioPlayerManager
    @Environment(\.dismiss) private var dismiss

    @State private var query = ""
    @State private var selected: Song?
    @State private var note = ""
    @State private var isSending = false

    private var results: [Song] {
        let q = query.trimmingCharacters(in: .whitespaces)
        if q.isEmpty {
            var picks: [Song] = []
            if let current = player.currentSong { picks.append(current) }
            picks += library.allSongs.prefix(30).filter { $0.id != player.currentSong?.id }
            return picks
        }
        return Array(library.allSongs.lazy.filter {
            $0.title.localizedCaseInsensitiveContains(q) || $0.artist.localizedCaseInsensitiveContains(q)
        }.prefix(60))
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let selected {
                    selectedCard(selected)
                        .padding(16)
                        .transition(.move(edge: .top).combined(with: .opacity))
                }
                List(results) { song in
                    Button {
                        withAnimation(LumiMotion.snap) { selected = song }
                    } label: {
                        HStack(spacing: 12) {
                            ArtworkThumbnail(song: song, size: 44, showsScrim: false)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(song.displayName).font(.subheadline.weight(.semibold)).foregroundStyle(AppTheme.textPrimary).lineLimit(1)
                                Text(song.artistName).font(.caption).foregroundStyle(AppTheme.textSecondary).lineLimit(1)
                            }
                            Spacer()
                            if selected?.id == song.id {
                                Image(systemName: "checkmark.circle.fill").foregroundStyle(AppTheme.dynamicAccent)
                            }
                        }
                    }
                    .listRowBackground(Color.clear)
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
                .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search your library")
            }
            .background(AppTheme.background.ignoresSafeArea())
            .navigationTitle("Send to \(friendName)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        send()
                    } label: {
                        if isSending { ProgressView() } else { Text("Send").bold() }
                    }
                    .disabled(selected == nil || isSending)
                }
            }
            .onAppear { if selected == nil { selected = player.currentSong } }
        }
    }

    private func selectedCard(_ song: Song) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                ArtworkThumbnail(song: song, size: 56, showsScrim: false)
                VStack(alignment: .leading, spacing: 2) {
                    Text(song.displayName).font(.headline).foregroundStyle(AppTheme.textPrimary).lineLimit(1)
                    Text(song.artistName).font(.caption).foregroundStyle(AppTheme.textSecondary).lineLimit(1)
                }
            }
            TextField("Add a note (optional)", text: $note, axis: .vertical)
                .lineLimit(1...3)
                .padding(10)
                .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color.white.opacity(0.07)))
                .onChange(of: note) { v in if v.count > 140 { note = String(v.prefix(140)) } }
        }
        .padding(14)
        .cellSurface(in: RoundedRectangle(cornerRadius: 20, style: .continuous), fallback: AppTheme.dynamicAccent.opacity(0.12))
    }

    private func send() {
        guard let song = selected else { return }
        isSending = true
        Task {
            let trimmed = note.trimmingCharacters(in: .whitespacesAndNewlines)
            let ok = await SongDropsStore.shared.sendSong(
                to: friendID, title: song.title, artist: song.artist.isEmpty ? nil : song.artist,
                sourceTrackID: song.sourceTrackID, note: trimmed.isEmpty ? nil : trimmed
            )
            isSending = false
            if ok {
                ToastCenter.shared.show("Sent “\(song.displayName)” to \(friendName)", category: .success, icon: "paperplane.fill")
                dismiss()
            }
        }
    }
}

// MARK: - Inbox

struct SongDropsInboxView: View {
    @EnvironmentObject private var library: LibraryManager
    @EnvironmentObject private var streaming: StreamingService
    @EnvironmentObject private var player: AudioPlayerManager
    @ObservedObject private var store = SongDropsStore.shared
    @State private var showSent = false

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 12) {
                Picker("", selection: $showSent) {
                    Text("Received").tag(false)
                    Text("Sent").tag(true)
                }
                .pickerStyle(.segmented)
                .padding(.bottom, 4)

                let drops = showSent ? store.sent : store.received
                if drops.isEmpty {
                    ContentUnavailableView(
                        showSent ? "Nothing sent yet" : "No drops yet",
                        systemImage: "paperplane",
                        description: Text(showSent ? "Send a friend a song from their profile or the Friends tab." : "When a friend sends you a song it lands here.")
                    )
                    .padding(.top, 40)
                } else {
                    ForEach(drops) { drop in
                        SongDropCard(drop: drop, incoming: !showSent) {
                            DropPlayer(library: library, streaming: streaming, player: player)
                                .play(title: drop.title, artist: drop.artist, sourceTrackID: drop.sourceTrackID)
                            Task { await store.markSeen(drop) }
                        }
                    }
                }
            }
            .padding(16)
            .padding(.bottom, 120)
        }
        .background(Color.clear)
        .navigationTitle("Song Drops")
        .navigationBarTitleDisplayMode(.inline)
        .task { await store.refresh(force: true) }
        .refreshable { await store.refresh(force: true) }
    }
}
