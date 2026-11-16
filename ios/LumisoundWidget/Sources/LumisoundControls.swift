import AppIntents
import SwiftUI
import WidgetKit

// MARK: - Control Center / Lock Screen / Action button controls
//
// Lumisound turned off the system media player (no MPNowPlayingInfo, see
// ios/TODO.md, 2.0.0.27), so Control Center had no Lumisound controls at all.
// These controls give it back: Play/Pause and Favorite as toggles that show
// the real state (read from the shared now-playing snapshot), Next and
// Previous as buttons. Every action is an intent compiled into the app too,
// so it runs in the app's own process (launching it in the background if
// needed). The app reloads the controls whenever the snapshot changes.

/// The play state and track for Control Center, from the App Group snapshot.
struct NowPlayingControlProvider: ControlValueProvider {
    var previewValue: Bool { false }

    func currentValue() async throws -> Bool {
        NowPlayingSnapshot.load()?.isPlaying ?? false
    }
}

struct FavoriteControlProvider: ControlValueProvider {
    var previewValue: Bool { false }

    func currentValue() async throws -> Bool {
        guard let s = NowPlayingSnapshot.load(), s.hasTrack else { return false }
        return s.isFavorite
    }
}

struct LumisoundPlayPauseControl: ControlWidget {
    static let kind = "com.lumisound.ios.control.playpause"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: NowPlayingControlProvider()) { isPlaying in
            ControlWidgetToggle("Lumisound", isOn: isPlaying, action: SetPlayingIntent()) { on in
                Label(on ? "Playing" : "Paused", systemImage: on ? "pause.fill" : "play.fill")
            }
        }
        .displayName("Play / Pause")
        .description("Play or pause Lumisound.")
    }
}

struct LumisoundNextControl: ControlWidget {
    static let kind = "com.lumisound.ios.control.next"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind) {
            ControlWidgetButton(action: SkipNextIntent()) {
                Label("Next Track", systemImage: "forward.fill")
            }
        }
        .displayName("Next Track")
        .description("Skip to the next track in Lumisound.")
    }
}

struct LumisoundPreviousControl: ControlWidget {
    static let kind = "com.lumisound.ios.control.previous"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind) {
            ControlWidgetButton(action: SkipPreviousIntent()) {
                Label("Previous Track", systemImage: "backward.fill")
            }
        }
        .displayName("Previous Track")
        .description("Go back a track in Lumisound.")
    }
}

struct LumisoundFavoriteControl: ControlWidget {
    static let kind = "com.lumisound.ios.control.favorite"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: FavoriteControlProvider()) { isFavorite in
            ControlWidgetToggle("Favorite", isOn: isFavorite, action: SetFavoriteIntent()) { on in
                Label(on ? "Favorited" : "Favorite", systemImage: on ? "heart.fill" : "heart")
            }
            .tint(.pink)
        }
        .displayName("Favorite Track")
        .description("Favorite or unfavorite the track playing in Lumisound.")
    }
}
