import SwiftUI
import UIKit

// MARK: - FolderMosaicService
//
// One finished bitmap per folder cover: the 2×2 artwork collage (or the
// folder's custom cover, or a single track's art), its rounded corners, the
// folder badge strip / hairline stroke and the drop shadow, all drawn once
// off the main thread and kept in memory.
//
// Why (screen recording 2026-11-14, Folders grid at 10–20 fps while flinging):
// each tile used to be four `ArtworkThumbnail`s — every one a UIKit
// `PanningArtwork` with an endless Core Animation slide for non-square art, a
// placeholder, a shimmer, a scrim and a continuous-corner clip — inside a
// GeometryReader, under a `.compositingGroup().shadow(…)`. With about a dozen
// tiles on screen that is ~48 animating, masked layers, and the shadow's
// offscreen pass re-ran every frame because the content under it kept moving.
// A tile is now a single static `Image` the render server only has to move.
// The folder detail header's 200 pt cover had the same shape and uses the
// `.header` style.
@MainActor
final class FolderMosaicService {
    static let shared = FolderMosaicService()

    enum Style: Hashable, Sendable {
        /// Folders grid tile: 12 pt corners, folder badge, soft shadow.
        case gridTile
        /// Folder detail header cover: 20 pt corners, hairline, deep shadow.
        case header

        var cornerRadius: CGFloat { self == .gridTile ? 12 : 20 }
        /// Room around the cover for the baked shadow, in points. The view
        /// shows the bitmap with this much negative padding.
        var margin: CGFloat { self == .gridTile ? 20 : 44 }
        fileprivate var shadow: (alpha: CGFloat, blur: CGFloat, y: CGFloat) {
            self == .gridTile ? (0.3, 14, 5) : (0.5, 32, 10)
        }
    }

    struct Key: Hashable {
        let folderID: String
        let songIDs: [String]
        let coverRevision: Int
        let pixelSide: Int
        var style: Style = .gridTile
    }

    private let cache: NSCache<KeyBox, UIImage> = {
        let c = NSCache<KeyBox, UIImage>()
        c.totalCostLimit = 48 * 1024 * 1024
        return c
    }()
    private var inFlight: [Key: Task<UIImage?, Never>] = [:]

    func cached(_ key: Key) -> UIImage? {
        cache.object(forKey: KeyBox(key))
    }

    /// The finished cover for `key`. Concurrent requests for the same key
    /// share one render.
    func mosaic(_ key: Key, songs: [Song], cover: UIImage?, sidePoints: CGFloat, scale: CGFloat) async -> UIImage? {
        if let hit = cached(key) { return hit }
        if let running = inFlight[key] { return await running.value }
        let surface = UIColor(AppTheme.surface)
        let accent = UIColor(AppTheme.dynamicAccent)
        let task = Task<UIImage?, Never> { @MainActor in
            // Quarter tiles at the thumbnail bucket that covers them; a
            // custom cover or a single track fills the whole cover.
            var art: [UIImage?] = []
            if cover == nil {
                let pixels = songs.count > 1 ? sidePoints * scale / 2 : sidePoints * scale
                for song in songs.prefix(4) {
                    if Task.isCancelled { return nil }
                    art.append(await ArtworkService.shared.loadThumbnail(for: song, pixelSize: pixels))
                }
            }
            let spec = RenderSpec(style: key.style, side: sidePoints, scale: scale, cover: cover, art: art,
                                  slots: cover != nil ? 1 : min(4, max(1, songs.count)),
                                  surface: surface, accent: accent)
            return await Task.detached(priority: .utility) { Self.render(spec) }.value
        }
        inFlight[key] = task
        let image = await task.value
        inFlight[key] = nil
        if let image {
            let px = image.size.width * image.scale
            cache.setObject(image, forKey: KeyBox(key), cost: Int(px * px * 4))
        }
        return image
    }

    // MARK: Rendering (any thread)

    private struct RenderSpec: @unchecked Sendable {
        let style: Style
        let side: CGFloat
        let scale: CGFloat
        let cover: UIImage?
        let art: [UIImage?]
        let slots: Int
        let surface: UIColor
        let accent: UIColor
    }

    nonisolated private static func render(_ s: RenderSpec) -> UIImage {
        let m = s.style.margin
        let canvas = CGSize(width: s.side + 2 * m, height: s.side + 2 * m)
        let format = UIGraphicsImageRendererFormat()
        format.scale = s.scale
        format.opaque = false
        format.preferredRange = .standard
        let tile = CGRect(x: m, y: m, width: s.side, height: s.side)
        let path = UIBezierPath(roundedRect: tile, cornerRadius: s.style.cornerRadius)
        return UIGraphicsImageRenderer(size: canvas, format: format).image { ctx in
            let cg = ctx.cgContext
            let shadow = s.style.shadow
            cg.saveGState()
            cg.setShadow(offset: CGSize(width: 0, height: shadow.y), blur: shadow.blur,
                         color: UIColor.black.withAlphaComponent(shadow.alpha).cgColor)
            s.surface.withAlphaComponent(1).setFill()
            path.fill()
            cg.restoreGState()

            cg.saveGState()
            path.addClip()
            s.surface.withAlphaComponent(1).setFill()
            UIRectFill(tile)
            if let cover = s.cover {
                drawFill(cover, in: tile)
            } else if s.slots == 1 {
                drawSlot(s.art.first ?? nil, in: tile, s)
            } else {
                let h = s.side / 2
                for i in 0..<min(4, s.art.count) {
                    let r = CGRect(x: m + CGFloat(i % 2) * h, y: m + CGFloat(i / 2) * h, width: h, height: h)
                    drawSlot(s.art[i], in: r, s)
                }
            }
            if s.style == .gridTile {
                // Folder badge strip along the bottom.
                let strip = CGRect(x: tile.minX, y: tile.maxY - 24, width: s.side, height: 24)
                UIColor.black.withAlphaComponent(0.35).setFill()
                UIRectFillUsingBlendMode(strip, .normal)
                let glyphConfig = UIImage.SymbolConfiguration(pointSize: 11, weight: .regular)
                if let glyph = UIImage(systemName: "folder.fill", withConfiguration: glyphConfig)?
                    .withTintColor(.white, renderingMode: .alwaysOriginal) {
                    glyph.draw(at: CGPoint(x: strip.minX + 6, y: strip.midY - glyph.size.height / 2))
                }
            }
            cg.restoreGState()

            if s.style == .header {
                UIColor.white.withAlphaComponent(0.12).setStroke()
                let stroke = UIBezierPath(roundedRect: tile.insetBy(dx: 0.5, dy: 0.5), cornerRadius: s.style.cornerRadius)
                stroke.lineWidth = 1
                stroke.stroke()
            }
        }
    }

    /// Aspect-fill, centered: a still frame of what `PanningArtwork` slides across.
    nonisolated private static func drawFill(_ image: UIImage, in rect: CGRect) {
        let w = image.size.width, h = image.size.height
        guard w > 0, h > 0, let ctx = UIGraphicsGetCurrentContext() else { return }
        let k = max(rect.width / w, rect.height / h)
        let size = CGSize(width: w * k, height: h * k)
        ctx.saveGState()
        UIRectClip(rect)
        image.draw(in: CGRect(x: rect.midX - size.width / 2, y: rect.midY - size.height / 2, width: size.width, height: size.height))
        ctx.restoreGState()
    }

    /// One track's art, or the same accent-tinted note placeholder the grid
    /// showed for a track without artwork.
    nonisolated private static func drawSlot(_ image: UIImage?, in rect: CGRect, _ s: RenderSpec) {
        if let image {
            drawFill(image, in: rect)
            return
        }
        guard let ctx = UIGraphicsGetCurrentContext() else { return }
        let colors = [s.accent.withAlphaComponent(0.35).cgColor, s.surface.withAlphaComponent(1).cgColor] as CFArray
        if let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1]) {
            ctx.saveGState()
            UIRectClip(rect)
            ctx.drawLinearGradient(gradient, start: rect.origin, end: CGPoint(x: rect.maxX, y: rect.maxY), options: [])
            ctx.restoreGState()
        }
        let config = UIImage.SymbolConfiguration(pointSize: rect.width * 0.32, weight: .medium)
        if let note = UIImage(systemName: "music.note", withConfiguration: config)?
            .withTintColor(s.accent.withAlphaComponent(0.85), renderingMode: .alwaysOriginal) {
            note.draw(at: CGPoint(x: rect.midX - note.size.width / 2, y: rect.midY - note.size.height / 2))
        }
    }
}

/// NSCache needs an object key.
private final class KeyBox: NSObject {
    let key: FolderMosaicService.Key
    init(_ key: FolderMosaicService.Key) { self.key = key }
    override var hash: Int { key.hashValue }
    override func isEqual(_ object: Any?) -> Bool { (object as? KeyBox)?.key == key }
}
