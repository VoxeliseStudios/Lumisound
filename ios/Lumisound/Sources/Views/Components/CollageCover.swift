import SwiftUI

/// A 2×2 cover built from up to four songs, rendered once into a single
/// bitmap by `FolderMosaicService` (`.collage`) — the cheap cover for any
/// grid of collections (genres, playlists). Falls back to an icon on a
/// gradient when there are no songs.
struct CollageCover: View {
    let key: String
    let songs: [Song]
    let size: CGFloat
    var icon: String = "music.note.list"
    var tint: Color = AppTheme.dynamicAccent

    @Environment(\.displayScale) private var displayScale
    @State private var image: UIImage?

    private var mosaicKey: FolderMosaicService.Key {
        let ids = songs.prefix(4).map(\.id)
        return FolderMosaicService.Key(
            folderID: "collage|" + key,
            songIDs: ids,
            coverRevision: 0,
            pixelSide: Int((size * displayScale).rounded()),
            style: .collage
        )
    }

    var body: some View {
        Group {
            if songs.isEmpty {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(LinearGradient(colors: [tint.opacity(0.55), tint.opacity(0.2)], startPoint: .topLeading, endPoint: .bottomTrailing))
                    .overlay(Image(systemName: icon).font(.system(size: size * 0.28, weight: .semibold)).foregroundStyle(.white.opacity(0.9)))
            } else if let image = image ?? FolderMosaicService.shared.cached(mosaicKey) {
                Image(uiImage: image).resizable().transition(.opacity)
            } else {
                RoundedRectangle(cornerRadius: 12, style: .continuous).fill(AppTheme.surface)
            }
        }
        .frame(width: size, height: size)
        .task(id: mosaicKey) {
            guard !songs.isEmpty, size >= 1 else { return }
            let k = mosaicKey
            if let hit = FolderMosaicService.shared.cached(k) { image = hit; return }
            let made = await FolderMosaicService.shared.mosaic(k, songs: Array(songs.prefix(4)), cover: nil, sidePoints: size, scale: displayScale)
            guard !Task.isCancelled, let made else { return }
            withAnimation(LumiMotion.glide(0.22)) { image = made }
        }
    }
}

/// A stable colour for a name (genre, playlist), so each keeps its own hue.
enum NameHue {
    static func color(_ name: String, saturation: Double = 0.62, brightness: Double = 0.72) -> Color {
        var hash = 5381
        for byte in name.utf8 { hash = ((hash << 5) &+ hash) &+ Int(byte) }
        return Color(hue: Double(abs(hash) % 360) / 360, saturation: saturation, brightness: brightness)
    }
}
