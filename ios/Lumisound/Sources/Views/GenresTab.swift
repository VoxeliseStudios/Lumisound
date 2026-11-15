import SwiftUI
import MediaPlayer

// MARK: - Genres (2026-11-15 redesign)
//
// A wall of genre tiles instead of a plain list: each genre gets its own
// colour, a cover collage of its songs (one pre-rendered bitmap, see
// `CollageCover`), the song count and a play button. Tapping a tile opens the
// genre; the play button starts it. Biggest genres first.

struct GenresTab: View {
    @EnvironmentObject private var library: LibraryManager
    @EnvironmentObject private var player: AudioPlayerManager

    @AppStorage("library_genres_columns") private var genreColumns: Int = 2
    @State private var containerWidth: CGFloat = 0

    private var columns: Int { min(3, max(2, genreColumns)) }

    private var tileSide: CGFloat {
        let n = CGFloat(columns)
        return max(0, (containerWidth - 32 - 12 * (n - 1)) / n)
    }

    /// Genres ordered by size, computed off the body path only when the
    /// genre list itself changes.
    private struct GenreEntry: Identifiable, Equatable {
        let genre: String
        let count: Int
        var id: String { genre }
    }
    @State private var ordered: [GenreEntry] = []

    var body: some View {
        ScrollView {
            if library.genres.isEmpty {
                EmptyStateView(icon: "guitars.fill", title: "No genres", message: "Add music with genre tags to see them here.")
                    .padding(.top, 60)
            } else {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: columns), spacing: 14) {
                    ForEach(ordered) { item in
                        NavigationLink {
                            GenreDetailView(genre: item.genre)
                        } label: {
                            GenreTile(genre: item.genre, count: item.count, side: tileSide) {
                                let songs = library.songs(inGenre: item.genre)
                                player.setQueue(songs.shuffled(), startIndex: 0, autoplay: true)
                            }
                        }
                        .buttonStyle(PressableButtonStyle())
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 190)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { containerWidth = $0 }
        .background(Color.clear.ignoresSafeArea())
        .task(id: library.genres) {
            ordered = library.genres
                .map { GenreEntry(genre: $0, count: library.songs(inGenre: $0).count) }
                .sorted { $0.count != $1.count ? $0.count > $1.count : $0.genre.localizedCaseInsensitiveCompare($1.genre) == .orderedAscending }
        }
        .toolbar {
            ToolbarItem(placement: .navigationBarTrailing) {
                Button {
                    genreColumns = columns == 2 ? 3 : 2
                } label: {
                    Image(systemName: columns == 2 ? "square.grid.3x3" : "square.grid.2x2")
                }
                .tint(AppTheme.dynamicAccent)
                .accessibilityLabel(columns == 2 ? "Three columns" : "Two columns")
            }
        }
    }
}

private struct GenreTile: View {
    let genre: String
    let count: Int
    let side: CGFloat
    let onPlay: () -> Void

    @EnvironmentObject private var library: LibraryManager

    var body: some View {
        let hue = NameHue.color(genre)
        ZStack(alignment: .bottomLeading) {
            CollageCover(key: "genre|" + genre, songs: library.collageSongs(from: library.songs(inGenre: genre)), size: max(side, 1), icon: "guitars.fill", tint: hue)
            // The genre's own colour washes up from the bottom so the name
            // stays readable over any artwork.
            LinearGradient(colors: [.clear, hue.opacity(0.55), hue.opacity(0.92)], startPoint: .top, endPoint: .bottom)
            VStack(alignment: .leading, spacing: 2) {
                Text(genre)
                    .font(.system(size: side > 140 ? 20 : 15, weight: .bold, design: .serif))
                    .foregroundStyle(.white)
                    .lineLimit(2)
                    .minimumScaleFactor(0.8)
                Text("\(count) song\(count == 1 ? "" : "s")")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.white.opacity(0.85))
            }
            .padding(10)
            .padding(.trailing, 36)
        }
        .frame(width: max(side, 1), height: max(side, 1))
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(alignment: .bottomTrailing) {
            Button(action: onPlay) {
                Image(systemName: "play.fill")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(hue)
                    .frame(width: 32, height: 32)
                    .background(Circle().fill(.white))
            }
            .buttonStyle(PressableButtonStyle())
            .padding(8)
            .accessibilityLabel("Shuffle \(genre)")
        }
        .shadowBehind(RoundedRectangle(cornerRadius: 12, style: .continuous), opacity: 0.3, radius: 8, y: 5)
    }
}

// MARK: - Genre page

struct GenreDetailView: View {
    let genre: String
    @EnvironmentObject private var library: LibraryManager
    @EnvironmentObject private var player: AudioPlayerManager

    private var songs: [Song] { library.songs(inGenre: genre) }

    var body: some View {
        let hue = NameHue.color(genre)
        List {
            Section {
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 14) {
                        CollageCover(key: "genre|" + genre, songs: library.collageSongs(from: songs), size: 96, icon: "guitars.fill", tint: hue)
                            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                        VStack(alignment: .leading, spacing: 4) {
                            Text(genre)
                                .font(.system(.title, design: .serif).weight(.bold))
                                .foregroundStyle(AppTheme.textPrimary)
                                .lineLimit(2)
                            Text("\(songs.count) songs")
                                .font(.subheadline)
                                .foregroundStyle(AppTheme.textSecondary)
                        }
                    }
                    HStack(spacing: 10) {
                        Button {
                            player.setQueue(songs, startIndex: 0, autoplay: true)
                        } label: {
                            Label("Play", systemImage: "play.fill")
                                .font(.subheadline.weight(.bold))
                                .foregroundStyle(.white)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 12)
                                .background(Capsule().fill(hue.gradient))
                        }
                        Button {
                            player.setQueue(songs.shuffled(), startIndex: 0, autoplay: true)
                        } label: {
                            Label("Shuffle", systemImage: "shuffle")
                                .font(.subheadline.weight(.bold))
                                .foregroundStyle(hue)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 12)
                                .background(Capsule().fill(hue.opacity(0.16)))
                        }
                    }
                    .buttonStyle(PressableButtonStyle())
                }
                .padding(.vertical, 8)
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
            }
            Section {
                ForEach(songs) { song in
                    Button {
                        player.play(song: song, in: songs, uiSource: "genre")
                    } label: {
                        SongRow(song: song, isCurrent: player.currentSong?.id == song.id)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(AppTheme.surface.opacity(0.5))
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(AppTheme.background.ignoresSafeArea())
        .navigationTitle(genre)
        .navigationBarTitleDisplayMode(.inline)
    }
}
