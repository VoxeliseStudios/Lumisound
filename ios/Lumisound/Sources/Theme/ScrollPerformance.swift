import SwiftUI

// MARK: - Scroll performance helpers
//
// Shared by every scrolling list/grid of cards (Library tabs, hub, folder
// and album pages) so they keep 60 fps while flinging. See `cellSurface` in
// GlassEffectCompat.swift for the background half.

extension View {
    /// A drop shadow cast by a plain `shape` drawn BEHIND the view, instead
    /// of `.shadow` on the view itself.
    ///
    /// `.shadow` on content that keeps changing (a cover that pans, artwork
    /// fading in, a glass card) derives the shadow from the content's alpha
    /// in an offscreen pass, redone every frame the content changes, for
    /// every visible cell. A shadow from a static shape is rendered once and
    /// then only moves with the cell. `shape` must match the view's outline
    /// (it is filled opaque black and hidden by the view).
    func shadowBehind<S: Shape>(_ shape: S, opacity: Double = 0.35, radius: CGFloat = 10, y: CGFloat = 6) -> some View {
        background {
            shape
                .fill(Color.black)
                .shadow(color: .black.opacity(opacity), radius: radius, y: y)
        }
    }
}
