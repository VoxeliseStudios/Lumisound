import SwiftUI

// MARK: - TVPlaylistsView (synced playlists — GET /user/playlists)
//
// A playlist's tracks come from whichever device(s) added them and may point
// at an on-device library item (`local_song_id`) instead of a stream URL —
// tvOS has no local file/media library access at all, so those entries are shown but not playable
// here. Entries backed by a Personal Cloud Library upload carry the bridge's
// own `/user/music/stream` URL as `track_url` and play like any other track.
//
// Create/rename/delete/add/remove all go through the bridge's dedicated
// playlist-mutation endpoints (not the wholesale `/user/sync` snapshot push,
// which would also overwrite the user's other settings) — see
// `TVBridgeClient`'s "Playlist mutations" section.

struct TVPlaylistsView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    @Environment(\.tvTheme) private var theme

    @State private var showNewPlaylist = false
    @State private var renamingPlaylist: TVPlaylist?

    var body: some View {
        GeometryReader { geo in
            ScrollView {
                if client.isLoadingPlaylists {
                    TVLoadingState(text: "Loading your playlists…")
                } else if let err = client.playlistsError {
                    TVEmptyState(systemImage: "exclamationmark.icloud",
                                 title: "Couldn't load playlists",
                                 message: err) {
                        Button {
                            Task { await client.fetchPlaylists(token: token) }
                        } label: {
                            TVPillLabel(title: "Try Again", systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                } else if client.playlists.isEmpty {
                    TVEmptyState(systemImage: "recordingtape",
                                 title: "No mixtapes yet",
                                 message: "Record one here, or on your iPhone — playlists sync across your devices.") {
                        Button {
                            showNewPlaylist = true
                        } label: {
                            TVPillLabel(title: "Record a Tape", systemImage: "plus")
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                } else {
                    content(width: geo.size.width)
                }
            }
        }
        .task {
            if client.playlists.isEmpty { await client.fetchPlaylists(token: token) }
        }
        .sheet(isPresented: $showNewPlaylist) {
            TVPlaylistNameSheet(title: "New Playlist", initialName: "") { name in
                _ = await client.createPlaylist(name: name, token: token)
            }
        }
        .sheet(item: $renamingPlaylist) { playlist in
            TVPlaylistNameSheet(title: "Rename Playlist", initialName: playlist.name) { name in
                _ = await client.renamePlaylist(id: playlist.id, name: name, token: token)
            }
        }
    }

    /// The tape wall: every playlist a cassette on the shelf, the blank tape
    /// first for recording a new one.
    private func content(width: CGFloat) -> some View {
        let count = 4
        let spacing: CGFloat = 44
        let usable = width - TVMetrics.margin * 2 - spacing * CGFloat(count - 1)
        let cell = max(200, usable / CGFloat(count))

        return VStack(alignment: .leading, spacing: 30) {
            HStack(alignment: .lastTextBaseline, spacing: 22) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("THE TAPE WALL")
                        .font(TVType.eyebrow)
                        .tracking(theme.tracking)
                        .foregroundStyle(TVPalette.neonAlt)
                    Text("Mixtapes")
                        .font(theme.display(58))
                }
                Text("\(client.playlists.count) \(client.playlists.count == 1 ? "tape" : "tapes")")
                    .font(TVType.meta)
                    .foregroundStyle(TVPalette.textSecondary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, TVMetrics.margin)
            .padding(.top, 20)

            LazyVGrid(
                columns: Array(repeating: GridItem(.fixed(cell), spacing: spacing), count: count),
                alignment: .leading,
                spacing: spacing + 10
            ) {
                newPlaylistCard(width: cell)
                ForEach(client.playlists) { playlist in
                    NavigationLink {
                        TVPlaylistDetailView(client: client, token: token, playlist: playlist)
                    } label: {
                        TVCassetteLabel(
                            name: playlist.name,
                            detail: tvSongCount(playlist.tracks.count),
                            artworkURLs: client.artworkURLs(for: playlist, token: token),
                            token: token,
                            width: cell
                        )
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                    .contextMenu { playlistMenu(playlist) }
                }
            }
            .padding(.horizontal, TVMetrics.margin)
        }
        .padding(.bottom, 80)
    }

    @ViewBuilder
    private func playlistMenu(_ playlist: TVPlaylist) -> some View {
        Button {
            renamingPlaylist = playlist
        } label: {
            Label("Rename", systemImage: "pencil")
        }
        Button(role: .destructive) {
            Task { await client.deletePlaylist(id: playlist.id, token: token) }
        } label: {
            Label("Delete", systemImage: "trash")
        }
    }

    private func newPlaylistCard(width: CGFloat) -> some View {
        Button {
            showNewPlaylist = true
        } label: {
            TVCassetteLabel(name: "Record a New Tape", detail: "Start from scratch",
                            artworkURLs: [], token: token, width: width, isBlank: true)
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
    }
}

// MARK: - Playlist detail

struct TVPlaylistDetailView: View {
    @ObservedObject var client: TVBridgeClient
    let token: String
    let playlist: TVPlaylist

    /// Re-reads the live copy out of `client.playlists` so a track removal
    /// (which mutates that array) is reflected here without a separate fetch.
    private var current: TVPlaylist {
        client.playlists.first(where: { $0.id == playlist.id }) ?? playlist
    }

    private func meta(playableCount: Int, totalCount: Int) -> String {
        if playableCount == totalCount { return tvSongCount(totalCount) }
        return "\(tvSongCount(totalCount)) · \(playableCount) playable on Apple TV"
    }

    var body: some View {
        // Computed ONCE per render and threaded through explicitly below —
        // `current`/`queue` were both computed `var`s read fresh by `meta`,
        // the Play button, AND once per row inside `row(for:)`'s `ForEach`,
        // so a K-track playlist's render recomputed the full `compactMap`
        // (itself O(K), and O(K · client.library.count) on a cache miss via
        // `client.playable(from:)`'s `libraryMatch` fallback) K+2 times —
        // O(K²) total for one screen draw. Same shape as the old
        // `smartTempoOrder` bug fixed on the main iOS app.
        let current = current
        let queue: [TVPlayable] = current.tracks.compactMap { client.playable(from: $0, token: token) }
        let urls = client.artworkURLs(for: current, token: token)
        TVGatefold(
            eyebrow: "Mixtape",
            title: current.name,
            subtitle: current.description,
            meta: meta(playableCount: queue.count, totalCount: current.tracks.count)
        ) {
            TVPlaylistArtwork(name: current.name, artworkURLs: urls, token: token, showsTitle: false)
        } actions: {
            if let first = queue.first {
                NavigationLink(value: TVPlayContext(queue: queue, startID: first.id)) {
                    TVPillLabel(title: "Press Play", systemImage: "play.fill")
                }
                .buttonStyle(.plain)
                .focusEffectDisabled()
            }
        } tracks: {
            if current.tracks.isEmpty {
                TVEmptyState(systemImage: "recordingtape",
                             title: "A blank tape",
                             message: "Press and hold Select on any song and choose Add to Playlist.")
            } else {
                TVTrackList {
                    ForEach(current.tracks) { track in
                        row(for: track, queue: queue)
                    }
                }
            }
        }
        .background(TVCollectionBackdrop(url: urls.first, token: token))
    }

    @ViewBuilder
    private func row(for track: TVPlaylistTrack, queue: [TVPlayable]) -> some View {
        let playable = client.playable(from: track, token: token)
        let label = TVTrackRow(
            artworkURL: playable?.artworkURL,
            token: token,
            title: track.title,
            artist: track.artist ?? "",
            detail: track.durationSeconds.flatMap { Double($0).tvDurationText },
            unavailableReason: playable == nil ? "Only on iPhone" : nil
        )

        Group {
            if let playable {
                NavigationLink(value: TVPlayContext(queue: queue, startID: playable.id)) {
                    label
                }
            } else {
                // Still focusable, so the context menu (Remove) can reach it —
                // a dead row you can see but not act on is worse than one that
                // explains itself.
                Button {} label: { label }
            }
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
        .contextMenu {
            Button(role: .destructive) {
                Task { await client.removeTrack(track.id, fromPlaylist: playlist.id, token: token) }
            } label: {
                Label("Remove from Playlist", systemImage: "minus.circle")
            }
        }
    }
}

// MARK: - TVPlaylistNameSheet
//
// Shared name-entry sheet for both create and rename. A plain `TextField` in
// a `List` (not a `TextField` embedded in `.alert`, which tvOS's on-screen
// keyboard flow doesn't drive reliably) — same pattern as the "New Playlist"
// row in `TVAddToPlaylistSheet`.

struct TVPlaylistNameSheet: View {
    let title: String
    let initialName: String
    let onSave: (String) async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var name = ""
    @State private var isSaving = false

    var body: some View {
        NavigationStack {
            List {
                TextField("Playlist name", text: $name)
            }
            .navigationTitle(title)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task {
                            isSaving = true
                            await onSave(name.trimmingCharacters(in: .whitespaces))
                            isSaving = false
                            dismiss()
                        }
                    }
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || isSaving)
                }
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .onAppear { name = initialName }
    }
}

// MARK: - Cassette

/// A playlist as a cassette tape: the shell, a label in the playlist's own
/// colour with its name and a strip of its covers, and the two reels in the
/// window — which spin while the tape has focus.
struct TVCassetteLabel: View {
    let name: String
    let detail: String
    let artworkURLs: [URL]
    let token: String
    let width: CGFloat
    var isBlank: Bool = false

    @Environment(\.isFocused) private var isFocused
    @Environment(\.tvTheme) private var theme
    @State private var spin = false

    private var height: CGFloat { width * 0.64 }

    /// The label colour, stable per name.
    private var labelColor: Color {
        let hues: [Double] = [0.0, 0.07, 0.13, 0.33, 0.5, 0.58, 0.72, 0.85, 0.93]
        var h: UInt32 = 2_166_136_261
        for s in name.unicodeScalars { h = (h ^ s.value) &* 16_777_619 }
        return Color(hue: hues[Int(h % UInt32(hues.count))], saturation: 0.55, brightness: 0.92)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            ZStack {
                // Shell
                RoundedRectangle(cornerRadius: width * 0.05, style: .continuous)
                    .fill(LinearGradient(colors: [Color(white: 0.2), Color(white: 0.09)],
                                         startPoint: .top, endPoint: .bottom))
                    .overlay {
                        RoundedRectangle(cornerRadius: width * 0.05, style: .continuous)
                            .strokeBorder(isFocused ? AnyShapeStyle(TVPalette.brand) : AnyShapeStyle(Color.white.opacity(0.12)),
                                          lineWidth: isFocused ? 3 : 1)
                    }
                // Corner screws
                ForEach(0..<4, id: \.self) { i in
                    Circle()
                        .fill(Color(white: 0.32))
                        .frame(width: width * 0.025, height: width * 0.025)
                        .position(x: i % 2 == 0 ? width * 0.05 : width * 0.95,
                                  y: i < 2 ? height * 0.07 : height * 0.93)
                }
                // Label
                VStack(spacing: 0) {
                    HStack(spacing: width * 0.03) {
                        Text(name)
                            .font(.system(size: max(16, width * 0.062), weight: .bold, design: theme.design))
                            .foregroundStyle(Color.black.opacity(0.82))
                            .lineLimit(1)
                            .minimumScaleFactor(0.6)
                        Spacer(minLength: 0)
                        Text(isBlank ? "C-90" : "A")
                            .font(.system(size: max(14, width * 0.05), weight: .black, design: .monospaced))
                            .foregroundStyle(Color.black.opacity(0.55))
                    }
                    .padding(.horizontal, width * 0.05)
                    .frame(height: height * 0.2)
                    Rectangle().fill(isBlank ? Color.gray.opacity(0.3) : Color.black.opacity(0.18)).frame(height: 2)
                    // Window with the reels
                    ZStack {
                        RoundedRectangle(cornerRadius: width * 0.03)
                            .fill(Color.black.opacity(0.82))
                        HStack(spacing: width * 0.22) {
                            reel
                            reel
                        }
                        // The tape between the reels.
                        Rectangle()
                            .fill(Color(red: 0.25, green: 0.15, blue: 0.1))
                            .frame(width: width * 0.22, height: height * 0.035)
                    }
                    .frame(width: width * 0.62, height: height * 0.3)
                    .padding(.vertical, height * 0.05)
                    // A strip of the tape's covers, like a hand-made J-card.
                    HStack(spacing: 2) {
                        ForEach(Array(artworkURLs.prefix(4).enumerated()), id: \.offset) { _, url in
                            TVAuthImage(url: url, token: token) { Color.black.opacity(0.2) }
                                .frame(width: height * 0.13, height: height * 0.13)
                                .clipped()
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, width * 0.05)
                    .frame(height: height * 0.13)
                }
                .padding(.vertical, height * 0.04)
                .background {
                    RoundedRectangle(cornerRadius: width * 0.03, style: .continuous)
                        .fill(isBlank ? Color(white: 0.9) : labelColor)
                }
                .padding(.horizontal, width * 0.07)
                .padding(.top, height * 0.1)
                .padding(.bottom, height * 0.2)
                // The trapezoid at the bottom of the shell.
                Path { p in
                    p.move(to: CGPoint(x: width * 0.22, y: height))
                    p.addLine(to: CGPoint(x: width * 0.28, y: height * 0.84))
                    p.addLine(to: CGPoint(x: width * 0.72, y: height * 0.84))
                    p.addLine(to: CGPoint(x: width * 0.78, y: height))
                }
                .fill(Color(white: 0.14))
            }
            .frame(width: width, height: height)
            .shadow(color: isFocused ? theme.primary.opacity(0.5) : .black.opacity(0.5),
                    radius: isFocused ? 30 : 12, y: isFocused ? 18 : 8)
            .scaleEffect(isFocused ? 1.06 : 1)
            .rotationEffect(.degrees(isFocused ? 0 : -1.2))

            Text(detail)
                .font(TVType.meta)
                .foregroundStyle(isFocused ? TVPalette.textSecondary : TVPalette.textTertiary)
                .frame(width: width, alignment: .leading)
        }
        .animation(.spring(response: 0.32, dampingFraction: 0.78), value: isFocused)
        .onChange(of: isFocused) { focused in
            spin = focused
        }
    }

    /// One reel: a white hub with six teeth. Spins (a transform, cheap) while
    /// the tape is focused.
    private var reel: some View {
        ZStack {
            Circle().fill(Color(red: 0.25, green: 0.15, blue: 0.1)).frame(width: height * 0.26, height: height * 0.26)
            Circle().fill(Color.white.opacity(0.9)).frame(width: height * 0.13, height: height * 0.13)
            ForEach(0..<6, id: \.self) { i in
                Rectangle()
                    .fill(Color.black.opacity(0.8))
                    .frame(width: height * 0.016, height: height * 0.04)
                    .offset(y: -height * 0.045)
                    .rotationEffect(.degrees(Double(i) * 60))
            }
        }
        .rotationEffect(.degrees(spin ? 360 : 0))
        .animation(spin ? .linear(duration: 2.2).repeatForever(autoreverses: false) : .easeOut(duration: 0.4), value: spin)
    }
}
