import AppIntents
import SwiftUI
import WidgetKit

// MARK: - Up Next widget
//
// The playing track (small art, title, live bar, controls) over the next
// tracks in the queue. Each row is a button that skips ahead to that track
// (`PlayUpNextIntent`, run in the app's process). Same snapshot and provider
// as the Now Playing widget, so both always agree.

struct UpNextWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: LumisoundEntry
    private var s: NowPlayingSnapshot { entry.now }

    private var isLarge: Bool { family == .systemLarge }
    private var rows: [NowPlayingSnapshot.UpNextItem] {
        Array((s.upNext ?? []).prefix(isLarge ? NowPlayingSnapshot.upNextLimit : 2))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: isLarge ? 12 : 8) {
            header
            if isLarge {
                VStack(spacing: 4) {
                    NowPlayingProgressBar(snapshot: s, height: 4)
                    NowPlayingTimeRow(snapshot: s, font: .system(size: 10, weight: .medium))
                }
                NowPlayingControls(snapshot: s, size: 18, spacing: 26, showsFavorite: true)
                    .frame(maxWidth: .infinity)
            }
            upNextTitle
            if rows.isEmpty {
                Text(s.hasTrack ? "Nothing queued after this track." : "Play something in Lumisound.")
                    .font(.system(size: 12))
                    .foregroundStyle(.white.opacity(0.6))
                Spacer(minLength: 0)
            } else {
                VStack(spacing: isLarge ? 10 : 6) {
                    ForEach(rows) { item in
                        UpNextRow(item: item, artSize: isLarge ? 40 : 30, titleSize: isLarge ? 14 : 12)
                    }
                }
                Spacer(minLength: 0)
            }
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            NowPlayingArtwork(image: entry.artwork, snapshot: s, height: isLarge ? 56 : 44, cornerRadius: 10, square: true)
            VStack(alignment: .leading, spacing: 4) {
                NowPlayingLabels(snapshot: s, titleSize: isLarge ? 15 : 13, showsDetail: isLarge)
                if !isLarge {
                    NowPlayingProgressBar(snapshot: s, height: 3)
                }
            }
            Spacer(minLength: 0)
            if !isLarge {
                Button(intent: TogglePlaybackIntent()) {
                    Image(systemName: s.isPlaying ? "pause.fill" : "play.fill")
                        .font(.system(size: 13, weight: .bold))
                        .foregroundStyle(.black)
                        .frame(width: 28, height: 28)
                        .background(Circle().fill(s.tintColor))
                        .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var upNextTitle: some View {
        HStack {
            Text("UP NEXT")
                .font(.system(size: 10, weight: .bold))
                .tracking(0.8)
                .foregroundStyle(s.tintColor)
            Spacer()
            if let q = s.queueLabel {
                Text(q)
                    .font(.system(size: 10, weight: .medium))
                    .monospacedDigit()
                    .foregroundStyle(.white.opacity(0.55))
            }
        }
    }
}

struct UpNextWidget: Widget {
    let kind = "LumisoundUpNextWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: LumisoundWidgetProvider()) { entry in
            UpNextWidgetView(entry: entry)
                .containerBackground(for: .widget) { NowPlayingBackdrop(snapshot: entry.now, artwork: entry.artwork) }
        }
        .configurationDisplayName("Up Next")
        .description("What's playing and what's coming. Tap a track to skip to it.")
        .supportedFamilies([.systemMedium, .systemLarge])
    }
}
