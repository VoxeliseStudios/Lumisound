import SwiftUI
import MediaPlayer
import UIKit

// MARK: - Folders Tab
//
// Groups imported songs by the first subdirectory of "Imported Music" that
// contains them. Unlike the Albums tab, grouping is done by filesystem path —
// every song inside a folder shows up here regardless of its album metadata tag.
// The actual grouping logic lives in `MusicFolderService.localFolderGroups(from:)`
// so `LibraryHubView`'s speed-dial folder tiles use the exact same folder set.

private typealias FolderEntry = MusicFolderService.LocalFolderGroup

struct FoldersTab: View {
    @EnvironmentObject private var library: LibraryManager
    @AppStorage("library_folders_columns") private var columns: Int = 2

    @State private var folders: [FolderEntry] = []
    /// Which folders' inline "peek" is expanded — owned here, not by each
    /// `FolderGridCell`, since the toggle button has to live OUTSIDE the
    /// `NavigationLink`'s label as a sibling overlay (see the ForEach below):
    /// a `Button` nested inside a `NavigationLink`'s label is an unreliable
    /// pattern on iOS — the outer link's tap recognizer can swallow the
    /// inner button's taps — so the actual toggle control can't live inside
    /// `FolderGridCell` itself the way a first draft of this had it.
    @State private var expandedFolderIDs: Set<String> = []
    /// Width of the scroll view, measured once here so each tile knows its
    /// side without a GeometryReader of its own.
    @State private var containerWidth: CGFloat = 0

    /// One column's width: the grid's 16 pt side insets and 12 pt spacing.
    private var tileSide: CGFloat {
        let n = CGFloat(max(1, columns))
        return max(0, (containerWidth - 32 - 12 * (n - 1)) / n)
    }

    private var gridColumns: [GridItem] {
        Array(repeating: GridItem(.flexible(), spacing: 12), count: columns)
    }

    var body: some View {
        ScrollView {
            if folders.isEmpty {
                EmptyStateView(
                    icon: "folder",
                    title: "No folders",
                    message: "Create subfolders inside \"Imported Music\" in the Files app to organise your tracks."
                )
                .padding(.top, 60)
            } else {
                LazyVGrid(columns: gridColumns, spacing: 16) {
                    ForEach(folders) { folder in
                        ZStack(alignment: .topTrailing) {
                            NavigationLink {
                                LocalFolderDetailView(folderName: folder.id, folderURL: folder.dirURL)
                            } label: {
                                FolderGridCell(folder: folder, side: tileSide, isExpanded: expandedFolderIDs.contains(folder.id))
                            }
                            .buttonStyle(.plain)

                            // Drawn on top as a sibling (not inside the
                            // NavigationLink's label) so it reliably gets its
                            // own taps instead of the link swallowing them —
                            // see `expandedFolderIDs`'s doc comment.
                            Button {
                                withAnimation(LumiMotion.snap) {
                                    if expandedFolderIDs.contains(folder.id) {
                                        expandedFolderIDs.remove(folder.id)
                                    } else {
                                        expandedFolderIDs.insert(folder.id)
                                    }
                                }
                            } label: {
                                Image(systemName: "chevron.down")
                                    .font(.caption2.weight(.bold))
                                    .foregroundStyle(AppTheme.textPrimary)
                                    .rotationEffect(.degrees(expandedFolderIDs.contains(folder.id) ? 180 : 0))
                                    .padding(6)
                                    // Flat color, not `adaptiveGlass` — this
                                    // is a real backdrop blur on iOS 26+, and
                                    // paying for one on every single grid
                                    // cell's decorative chevron (20-50+
                                    // visible/near-visible at once) is a real,
                                    // measured contributor to scroll stutter
                                    // and sustained GPU/thermal load. A flat
                                    // backing reads the same for a glyph this
                                    // small.
                                    .background(Color.black.opacity(0.4), in: Circle())
                            }
                            .buttonStyle(.plain)
                            .padding(8)
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                // Extra clearance below the last row — see SongsTab's
                // identical fix.
                .padding(.bottom, 190)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { containerWidth = $0 }
        .background(Color.clear.ignoresSafeArea())
        .task(id: library.allSongs.count) {
            // Grouping is O(n) over the whole library and, for the big
            // libraries this tab exists for, easily large enough to be a
            // visible main-thread stall — see `rebuildAllSongs()`'s identical
            // off-actor pattern. `allSongs.count` changes once per rebuild
            // *during* a scan (song-by-song, every ~0.1s — see
            // `rebuildAllSongs`'s debounce), so without this, a long import
            // repeatedly re-blocked the main thread doing a full re-group,
            // which is what made scrolling here freeze and snap back to the
            // top while a scan was still running.
            let songs = library.allSongs
            let grouped = await Task.detached(priority: .userInitiated) {
                MusicFolderService.localFolderGroups(from: songs)
            }.value
            guard !Task.isCancelled else { return }
            folders = grouped
        }
        .toolbar {
            // Mirrors SongsTab's column-toggle buttons (rather than a Menu) so
            // switching the Folders layout works the same, directly-tappable way.
            ToolbarItemGroup(placement: .navigationBarTrailing) {
                Button {
                    columns = 1
                } label: {
                    Image(systemName: "rectangle.grid.1x2")
                        .foregroundStyle(columns == 1 ? AppTheme.dynamicAccent : AppTheme.textSecondary)
                }
                .buttonStyle(.plain)

                Button {
                    columns = 2
                } label: {
                    Image(systemName: "square.grid.2x2")
                        .foregroundStyle(columns == 2 ? AppTheme.dynamicAccent : AppTheme.textSecondary)
                }
                .buttonStyle(.plain)

                Button {
                    columns = 3
                } label: {
                    Image(systemName: "square.grid.3x3")
                        .foregroundStyle(columns == 3 ? AppTheme.dynamicAccent : AppTheme.textSecondary)
                }
                .buttonStyle(.plain)
            }
        }
    }
}

private struct FolderGridCell: View {
    let folder: FolderEntry
    /// Tile side in points, from `FoldersTab.tileSide`.
    let side: CGFloat
    /// Owned by `FoldersTab` (see `expandedFolderIDs`'s doc comment there) —
    /// this view only reads it to decide whether to show the inline peek;
    /// the actual toggle button lives outside this view entirely.
    let isExpanded: Bool

    @Environment(\.displayScale) private var displayScale
    /// The finished tile from `FolderMosaicService` (collage or custom cover,
    /// corners, badge and shadow in one bitmap).
    @State private var tile: UIImage? = nil
    // Observed so a cover change made elsewhere (LocalFolderDetailView)
    // re-renders this tile — see `FolderCoverArtService.changeCount`.
    @ObservedObject private var coverService = FolderCoverArtService.shared

    private var trackCount: Int { folder.songs.count }

    private var mosaicKey: FolderMosaicService.Key {
        FolderMosaicService.Key(
            folderID: folder.id,
            songIDs: folder.songs.prefix(4).map(\.id),
            coverRevision: coverService.changeCount,
            pixelSide: Int((side * displayScale).rounded())
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // One static image per tile; the memory-cache read keeps a tile
            // that scrolls back into view from flashing its placeholder.
            let image = tile ?? FolderMosaicService.shared.cached(mosaicKey)
            Color.clear
                .frame(maxWidth: .infinity)
                .aspectRatio(1, contentMode: .fit)
                .overlay {
                    if let image {
                        // The bitmap carries `Style.gridTile.margin` of shadow on each side.
                        Image(uiImage: image)
                            .resizable()
                            .padding(-FolderMosaicService.Style.gridTile.margin)
                            .transition(.opacity)
                    } else {
                        RoundedRectangle(cornerRadius: FolderMosaicService.Style.gridTile.cornerRadius, style: .continuous)
                            .fill(AppTheme.surface)
                    }
                }
                .task(id: mosaicKey) {
                    guard side >= 1 else { return }
                    let key = mosaicKey
                    if let hit = FolderMosaicService.shared.cached(key) {
                        tile = hit
                        return
                    }
                    let cover = await FolderCoverArtService.shared.cover(for: folder.dirURL)
                    guard !Task.isCancelled else { return }
                    let made = await FolderMosaicService.shared.mosaic(
                        key, songs: Array(folder.songs.prefix(4)), cover: cover,
                        sidePoints: side, scale: displayScale
                    )
                    guard !Task.isCancelled, let made else { return }
                    withAnimation(LumiMotion.glide(0.22)) { tile = made }
                }

            VStack(alignment: .leading, spacing: 2) {
                Text(folder.id)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(AppTheme.textPrimary)
                    .lineLimit(2)

                Text("\(trackCount) \(trackCount == 1 ? "track" : "tracks")")
                    .font(.caption2)
                    .foregroundStyle(AppTheme.textSecondary)
            }
            .padding(.horizontal, 2)

            if isExpanded {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(folder.songs.prefix(4)) { song in
                        Text(song.displayName)
                            .font(.caption2)
                            .foregroundStyle(AppTheme.textSecondary)
                            .lineLimit(1)
                    }
                    if trackCount > 4 {
                        Text("+ \(trackCount - 4) more")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(AppTheme.dynamicAccent)
                    }
                }
                .padding(.horizontal, 2)
                .padding(.top, 2)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
    }
}
