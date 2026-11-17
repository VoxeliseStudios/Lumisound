import Foundation
import SwiftUI

// MARK: - Presets (themes, sound effects, visualizer styles)
//
// Thin glue over the Rust core's presets module (rust/core/src/presets):
// it holds the bundled presets, checks and stores shared JSON presets under
// Documents/Presets/, and resolves every preset (shade-derived colours, EQ
// curve shapes, genre curves, tempo-synced tremolo, colour ramps). Swift only
// writes the resolved values into the settings the app already reads.
// (Replaced the Lua scripting layer.)

enum PresetPaths {
    static var documents: String {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].path
    }
}

// MARK: - Theme presets

@MainActor
final class ThemePresetEngine: ObservableObject {
    static let shared = ThemePresetEngine()

    /// Bundled id (`forest_calm`) or `user:<name>`; nil when no preset is applied.
    @Published private(set) var activePresetID: String?
    @Published private(set) var activePresetName: String?
    @Published private(set) var lastError: String?
    /// The active preset's accent, as hex, for the watch (see PhoneWatchSync).
    @Published private(set) var activeAccentHex: String?

    private enum Keys {
        static let active = "preset_active_theme"
        // Lua era: the old enum's raw value, or a user script path.
        static let legacyPreset = "lua_active_preset"
        static let legacyUserScript = "lua_active_user_script_path"
    }

    private init() {
        let d = UserDefaults.standard
        if d.string(forKey: Keys.active) == nil, let legacy = d.string(forKey: Keys.legacyPreset) {
            d.set(presetsCanonicalId(kind: .theme, id: legacy), forKey: Keys.active)
        }
        // Imported Lua theme scripts can't run any more; their colours stay as applied.
        d.removeObject(forKey: Keys.legacyPreset)
        d.removeObject(forKey: Keys.legacyUserScript)
        if let id = d.string(forKey: Keys.active) {
            // Re-resolve at launch so the preset's look stays reproducible even
            // if something else wrote one of the same settings in between.
            apply(id, persist: false)
        }
    }

    /// Resolves and applies a theme preset. On failure the current look is
    /// left alone and `lastError` says why.
    @discardableResult
    func apply(_ id: String, persist: Bool = true) -> Bool {
        do {
            let theme = try presetsTheme(documentsDir: PresetPaths.documents, id: id)
            applyToAppTheme(theme)
            activePresetID = theme.id
            activePresetName = theme.name
            activeAccentHex = theme.accent
            lastError = nil
            if persist { UserDefaults.standard.set(theme.id, forKey: Keys.active) }
            return true
        } catch {
            lastError = error.localizedDescription
            if !persist {
                // A preset that no longer resolves at launch (deleted file).
                UserDefaults.standard.removeObject(forKey: Keys.active)
            }
            return false
        }
    }

    /// Clears the preset marker and the two preset-only settings with no
    /// other UI (background override, layout scales). Individual colours and
    /// styles stay as the preset left them; Appearance → Reset restores those.
    func clearPreset() {
        UserDefaults.standard.removeObject(forKey: Keys.active)
        activePresetID = nil
        activePresetName = nil
        activeAccentHex = nil
        lastError = nil
        AppTheme.resetCustomBackgroundOverride()
        AppTheme.resetLayoutScales()
    }

    func deleted(_ id: String) {
        if activePresetID == id { clearPreset() }
    }

    /// Writes a resolved preset into the settings AppTheme, GlassSettings
    /// and the views already read. Settings the preset leaves out (nil) are
    /// not touched.
    private func applyToAppTheme(_ t: PresetTheme) {
        let d = UserDefaults.standard
        AppTheme.saveAccentColor(Color(hex: t.accent))
        AppTheme.saveAccentSecondaryColor(Color(hex: t.accentSecondary))
        AppTheme.saveCustomBackgroundOverride(background: t.background, surface: t.surface, elevatedSurface: t.elevatedSurface)

        if let v = t.fontStyle { AppTheme.saveFontStyle(AppFontStyle(rawValue: v) ?? .system) }
        if let v = t.panelMaterial { AppTheme.savePanelMaterialStyle(PanelMaterialStyle(rawValue: v) ?? .solid) }
        if let v = t.launchScreenStyle { d.set(v, forKey: "launch_screen_style") }
        if let v = t.tabTransitionStyle { d.set(v, forKey: "tab_transition_style") }
        if let v = t.artworkStyle { d.set(v, forKey: "nowPlaying_artworkStyle") }
        if let v = t.seekerStyle { d.set(v, forKey: "nowPlaying_seekerStyle") }
        if let v = t.cardStyle { d.set(v, forKey: "library_cardStyle") }
        if let v = t.panelOpacity { d.set(v, forKey: "panel_opacity") }
        if t.cornerRadiusScale != nil || t.spacingScale != nil {
            let current = AppTheme.presetLayoutScales
            AppTheme.saveLayoutScales(cornerRadius: t.cornerRadiusScale ?? current.cornerRadius,
                                      spacing: t.spacingScale ?? current.spacing)
        }

        let glass = GlassSettings.shared
        if let v = t.glassTintStrength { glass.tintStrength = v }
        if let v = t.glassTintHue { glass.tintHue = v }
        if let v = t.glassUseAccentTint { glass.useAccentTint = v }
        if let v = t.glassTranslucency { glass.translucency = v }

        if let v = t.reduceMotion { d.set(v, forKey: "app_reduce_motion") }
        if let v = t.showBpmBadges { d.set(v, forKey: PresetFeatureFlags.Keys.showBpmBadges) }
        if let v = t.aggressivePrefetch { d.set(v, forKey: PresetFeatureFlags.Keys.aggressivePrefetch) }
        if let v = t.hapticFeedback { d.set(v, forKey: PresetFeatureFlags.Keys.hapticFeedback) }

        if let v = t.libraryDefaultSort { d.set(v, forKey: "library_songs_sort") }
        if let v = t.pinFavoritesFirst { d.set(v, forKey: PresetFeatureFlags.Keys.pinFavoritesFirst) }
        if let v = t.libraryDefaultColumns { d.set(Int(v), forKey: "library_songs_columns") }

        if let v = t.layoutDefaultPanel { d.set(v, forKey: PresetFeatureFlags.Keys.layoutDefaultPanel) }
        if let v = t.layoutDefaultDisplayMode { d.set(v, forKey: PresetFeatureFlags.Keys.layoutDefaultDisplayMode) }
        if let v = t.layoutShowQueuePreview { d.set(v, forKey: "nowPlaying_showQueuePreview") }
        if let v = t.layoutEqExpanded { d.set(v, forKey: PresetFeatureFlags.Keys.layoutEqExpanded) }
    }
}

// MARK: - Feature flags a theme preset can set

enum PresetFeatureFlags {
    /// Stored keys kept from the Lua era so existing choices survive.
    enum Keys {
        static let showBpmBadges = "lua_flag_show_bpm_badges"
        static let aggressivePrefetch = "lua_flag_aggressive_prefetch"
        static let hapticFeedback = "lua_flag_haptic_feedback"
        static let pinFavoritesFirst = "lua_pin_favorites_first"
        static let layoutDefaultPanel = "lua_layout_default_panel"
        static let layoutDefaultDisplayMode = "lua_layout_default_display_mode"
        static let layoutEqExpanded = "lua_layout_eq_expanded"
    }

    /// BPM badge next to the artist on Now Playing (default on).
    static var showBpmBadges: Bool {
        UserDefaults.standard.object(forKey: Keys.showBpmBadges) as? Bool ?? true
    }

    /// Wider artwork prefetch window on the Songs tab (default off).
    static var aggressivePrefetch: Bool {
        UserDefaults.standard.object(forKey: Keys.aggressivePrefetch) as? Bool ?? false
    }

    /// Haptics on the mini player's buttons (default on).
    static var hapticFeedback: Bool {
        UserDefaults.standard.object(forKey: Keys.hapticFeedback) as? Bool ?? true
    }
}

// MARK: - Effect presets

enum EffectPresets {
    /// `AudioEffect.id` of a preset effect: `preset:<preset id>`. Older builds
    /// saved `lua:<script id>`; both resolve.
    static let idPrefix = "preset:"

    static func isPresetEffectID(_ effectID: String) -> Bool {
        effectID.hasPrefix(idPrefix) || effectID.hasPrefix("lua:")
    }

    static func presetID(fromEffectID effectID: String) -> String? {
        if effectID.hasPrefix(idPrefix) { return String(effectID.dropFirst(idPrefix.count)) }
        if effectID.hasPrefix("lua:") { return String(effectID.dropFirst(4)) }
        return nil
    }

    /// Resolves a preset into an `AudioEffect`, reacting to `track` (genre
    /// curves, tempo-synced tremolo) when one is playing.
    static func effect(presetID: String, track: Song?) throws -> AudioEffect {
        let context = track.map { PresetTrack(genre: $0.genre, bpm: $0.bpm) }
        let e = try presetsEffect(documentsDir: PresetPaths.documents, id: presetID, track: context)
        let special: AudioEffectSpecialMode
        switch e.special {
        case .rotation(let hz): special = .rotation(hz: hz)
        case .tremolo(let freq, let depth): special = .tremolo(freq: freq, depth: Float(depth))
        case .vibrato(let freq, let pitchDepth): special = .vibrato(freq: freq, depth: pitchDepth)
        case nil: special = .none
        }
        return AudioEffect(id: idPrefix + e.id, name: e.name, icon: e.icon, eqBands: e.eqBands, eqEnabled: e.eqEnabled,
                           speed: e.speed, pitchSemitones: e.pitchSemitones, specialMode: special)
    }

    /// Re-resolves a saved effect id (e.g. on every track start, so genre
    /// curves follow the music).
    static func effect(forEffectID effectID: String, track: Song?) -> AudioEffect? {
        guard let id = presetID(fromEffectID: effectID) else { return nil }
        return try? effect(presetID: id, track: track)
    }
}

// MARK: - Visualizer presets

@MainActor
final class VisualizerPresetStore: ObservableObject {
    static let shared = VisualizerPresetStore()

    @Published private(set) var activeConfig: PresetVisualizer?
    @Published private(set) var activePresetID: String?
    @Published private(set) var lastError: String?

    private static let key = "preset_active_visualizer"
    private static let legacyKey = "lua_active_visualizer_script"

    private init() {
        let d = UserDefaults.standard
        if d.string(forKey: Self.key) == nil, let legacy = d.string(forKey: Self.legacyKey) {
            d.set(legacy, forKey: Self.key)
        }
        d.removeObject(forKey: Self.legacyKey)
        if let id = d.string(forKey: Self.key) {
            apply(id, persist: false)
        }
    }

    @discardableResult
    func apply(_ id: String, persist: Bool = true) -> Bool {
        do {
            let config = try presetsVisualizer(documentsDir: PresetPaths.documents, id: id)
            activeConfig = config
            activePresetID = config.id
            lastError = nil
            if persist { UserDefaults.standard.set(config.id, forKey: Self.key) }
            return true
        } catch {
            lastError = error.localizedDescription
            if !persist { UserDefaults.standard.removeObject(forKey: Self.key) }
            return false
        }
    }

    func clear() {
        activeConfig = nil
        activePresetID = nil
        lastError = nil
        UserDefaults.standard.removeObject(forKey: Self.key)
    }

    func deleted(_ id: String) {
        if activePresetID == id { clear() }
    }
}
