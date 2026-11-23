import SwiftUI

// MARK: - TVSettingsView
//
// Playback and display preferences, reached from the Account screen.
//
// Aria Lumi has no row here. That is deliberate and stated on the screen rather
// than left as a silent omission — a missing switch reads as an oversight, and
// someone looking for it should find the answer instead of wondering. The only
// Aria-related control is whether her spoken handover line is DISPLAYED, which
// is a presentation choice and does not turn her off.
struct TVSettingsView: View {
    @ObservedObject private var settings = TVAudioSettings.shared
    @ObservedObject private var aria = TVAria.shared
    @ObservedObject private var themeStore = TVThemeStore.shared
    @AppStorage("tv.nowPlaying.artworkStyle") private var artworkStyleRaw = TVArtworkStyle.classic.rawValue
    @Environment(\.tvTheme) private var theme

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 34) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("THE DESK")
                        .font(TVType.eyebrow)
                        .tracking(theme.tracking)
                        .foregroundStyle(TVPalette.neonAlt)
                    Text("Settings")
                        .font(theme.display(58))
                }
                .padding(.horizontal, TVMetrics.margin)
                .padding(.top, 36)

                appearanceStrip

                // The desk: one channel strip per group, side by side.
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(alignment: .top, spacing: 26) {
                        strip("Playback", number: 1) {
                            toggleRow(
                                title: "Skip Silent Intros",
                                detail: "Starts a track at its first real sound, skipping a silent or fading-in lead-in.",
                                systemImage: "scissors",
                                isOn: $settings.skipSilentIntros
                            )
                            toggleRow(
                                title: "Crossfade",
                                detail: "Overlaps the end of one track with the start of the next.",
                                systemImage: "arrow.triangle.merge",
                                isOn: $settings.crossfadeEnabled
                            )
                            if settings.crossfadeEnabled {
                                toggleRow(
                                    title: "Smart Crossfade",
                                    detail: "Plans each transition from both songs: starts on a downbeat, runs whole bars, swaps the basslines.",
                                    systemImage: "waveform.path.ecg",
                                    isOn: $settings.autoCrossfade
                                )
                            }
                        }
                        strip("Sound", number: 2) {
                            toggleRow(
                                title: "LumiEQ",
                                detail: "The phone's equalizer, running in the Lumisound engine on this Apple TV.",
                                systemImage: "slider.vertical.3",
                                isOn: $settings.eqEnabled
                            )
                            if settings.eqEnabled { eqPresetRow }
                            toggleRow(
                                title: "Night Mode",
                                detail: "Evens out loud and quiet passages so late, low listening stays clear.",
                                systemImage: "moon.stars",
                                isOn: $settings.nightMode
                            )
                            roomRow
                        }
                        strip("Aria Lumi", number: 3) {
                            ariaNotice
                            toggleRow(
                                title: "Show Track Intros",
                                detail: "Displays her one-line handover as a new track begins. Aria keeps running either way.",
                                systemImage: "text.bubble",
                                isOn: $settings.djTransitions
                            )
                        }
                        strip("Library", number: 4) {
                            columnsRow
                        }
                    }
                    .padding(.horizontal, TVMetrics.margin)
                    .padding(.vertical, 20)
                }
            }
            .padding(.bottom, 80)
        }
        .tvAmbientBackground()
    }

    // MARK: Appearance

    /// The lighting rig: every theme as a swatch (select to switch the whole
    /// app), then the Now Playing artwork styles.
    private var appearanceStrip: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 14) {
                Text("LIGHTING")
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text("\(themeStore.theme.displayName) — \(themeStore.theme.tagline)")
                    .font(TVType.rowDetail)
                    .foregroundStyle(TVPalette.textSecondary)
                    .lineLimit(1)
            }
            .padding(.horizontal, TVMetrics.margin)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 24) {
                    ForEach(TVTheme.allCases) { t in
                        Button {
                            withAnimation(.easeInOut(duration: 0.4)) { themeStore.theme = t }
                        } label: {
                            TVThemeCardLabel(theme: t, isSelected: themeStore.theme == t)
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                }
                .padding(.horizontal, TVMetrics.margin)
                .padding(.vertical, 18)
            }

            Text("NOW PLAYING STYLE")
                .font(TVType.eyebrow)
                .tracking(theme.tracking)
                .foregroundStyle(TVPalette.neonAlt)
                .padding(.horizontal, TVMetrics.margin)
                .padding(.top, 6)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 16) {
                    ForEach(TVArtworkStyle.allCases) { style in
                        Button {
                            artworkStyleRaw = style.rawValue
                            TVRemoteLogger.log(category: "settings", event: "artwork_style_changed",
                                               detail: ["style": style.rawValue])
                        } label: {
                            TVChip(title: style.displayName, isSelected: artworkStyleRaw == style.rawValue,
                                   systemImage: style.systemImage)
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                }
                .padding(.horizontal, TVMetrics.margin)
                .padding(.vertical, 14)
            }
        }
        .focusSection()
    }

    // MARK: Pieces

    /// A channel strip: a numbered fader-scribble at the top, then the
    /// group's controls, on a tall panel.
    private func strip<Content: View>(_ title: String, number: Int,
                                      @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Text(String(format: "%02d", number))
                    .font(.system(size: 15, weight: .black, design: .monospaced))
                    .foregroundStyle(theme.ground)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(RoundedRectangle(cornerRadius: 5).fill(TVPalette.brand))
                Text(title.uppercased())
                    .font(.system(size: 18, weight: .heavy, design: theme.design))
                    .tracking(2)
                    .foregroundStyle(.white)
            }
            .padding(.horizontal, 12)
            .padding(.top, 8)
            // The scribble strip: masking tape across the top of the channel.
            Rectangle()
                .fill(Color(red: 0.93, green: 0.9, blue: 0.8).opacity(0.85))
                .frame(height: 6)
                .padding(.horizontal, 12)
            VStack(spacing: 6) {
                content()
            }
        }
        .padding(12)
        .frame(width: 540, alignment: .topLeading)
        .tvGlassPanel(cornerRadius: 30)
        .focusSection()
    }

    /// Says plainly why there is no on/off switch, rather than leaving a gap.
    private var ariaNotice: some View {
        HStack(alignment: .top, spacing: 16) {
            TVAriaMark(isAnimating: true, diameter: 30)
            VStack(alignment: .leading, spacing: 6) {
                Text("Aria is always on")
                    .font(.system(size: 23, weight: .semibold))
                Text("She picks a track for you each day and introduces what's coming up. There's no switch for her.")
                    .font(TVType.rowDetail)
                    .foregroundStyle(.white.opacity(0.5))
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 18)
        .background {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(TVPalette.violet.opacity(0.16))
        }
    }

    private func toggleRow(title: String, detail: String, systemImage: String,
                           isOn: Binding<Bool>) -> some View {
        Button {
            isOn.wrappedValue.toggle()
            TVRemoteLogger.log(category: "settings", event: "setting_toggled",
                               detail: ["setting": title, "value": isOn.wrappedValue])
        } label: {
            TVSettingRowLabel(title: title, detail: detail,
                              systemImage: systemImage, isOn: isOn.wrappedValue)
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
    }

    /// The preset families from the core's catalogue as chips; the chosen
    /// preset's tagline underneath.
    private var eqPresetRow: some View {
        let presets = eqPresets().filter { $0.category == "Signature" || $0.category == "Genre" }
        let current = presets.first { $0.id == settings.eqPresetID }
        return VStack(alignment: .leading, spacing: 10) {
            Text("Preset · \(current?.name ?? "Custom")")
                .font(.system(size: 23, weight: .semibold))
            Text(current?.tagline ?? "")
                .font(TVType.rowDetail)
                .foregroundStyle(TVPalette.textTertiary)
                .lineLimit(2)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 14) {
                    ForEach(presets, id: \.id) { p in
                        Button {
                            settings.eqPresetID = p.id
                            TVRemoteLogger.log(category: "settings", event: "eq_preset_changed", detail: ["preset": p.id])
                        } label: {
                            TVChip(title: p.name, isSelected: settings.eqPresetID == p.id)
                        }
                        .buttonStyle(.plain)
                        .focusEffectDisabled()
                    }
                }
                .padding(.vertical, 8)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
    }

    private var roomRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Room")
                .font(.system(size: 23, weight: .semibold))
            Text("A touch of space around the music, from the engine's reverb.")
                .font(TVType.rowDetail)
                .foregroundStyle(TVPalette.textTertiary)
            HStack(spacing: 14) {
                ForEach(Array(["Off", "Room", "Hall"].enumerated()), id: \.offset) { i, name in
                    Button {
                        settings.room = i
                    } label: {
                        TVChip(title: name, isSelected: settings.room == i)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
            .padding(.top, 6)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
    }

    /// Column count as a chip row: three fixed choices are faster to hit
    /// directly than to step through.
    private var columnsRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Cards per row")
                .font(.system(size: 23, weight: .semibold))
            Text("Artists, genres and search results. Fewer columns means larger artwork.")
                .font(TVType.rowDetail)
                .foregroundStyle(TVPalette.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 14) {
                ForEach([2, 3, 4], id: \.self) { count in
                    Button {
                        settings.gridColumns = count
                        TVRemoteLogger.log(category: "settings", event: "grid_columns_changed",
                                           detail: ["columns": count])
                    } label: {
                        TVChip(title: "\(count)", isSelected: settings.gridColumns == count)
                    }
                    .buttonStyle(.plain)
                    .focusEffectDisabled()
                }
            }
            .padding(.top, 6)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
    }
}

/// A theme as a lighting-rig card: its swatch, name, and a lit lamp when it's
/// the one in use.
private struct TVThemeCardLabel: View {
    let theme: TVTheme
    let isSelected: Bool

    @Environment(\.isFocused) private var isFocused

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TVThemeSwatch(theme: theme, size: 190)
                .overlay(alignment: .topTrailing) {
                    Circle()
                        .fill(isSelected ? theme.primary : Color.black.opacity(0.4))
                        .frame(width: 16, height: 16)
                        .shadow(color: isSelected ? theme.primary : .clear, radius: 8)
                        .padding(14)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 190 * 0.22, style: .continuous)
                        .strokeBorder(isFocused ? Color.white : (isSelected ? theme.primary : Color.white.opacity(0.1)),
                                      lineWidth: isFocused || isSelected ? 3 : 1)
                }
                .shadow(color: theme.primary.opacity(isFocused ? 0.6 : 0.2), radius: isFocused ? 26 : 10, y: 8)
            Text(theme.displayName)
                .font(theme.display(21, .bold))
                .foregroundStyle(isFocused || isSelected ? Color.white : Color.white.opacity(0.65))
                .frame(width: 190, alignment: .leading)
                .lineLimit(1)
        }
        .scaleEffect(isFocused ? 1.07 : 1)
        .animation(.spring(response: 0.3, dampingFraction: 0.78), value: isFocused)
    }
}

/// A settings row's appearance, split out so `@Environment(\.isFocused)` is read
/// inside the button's label — the pattern every custom control in this port
/// uses.
private struct TVSettingRowLabel: View {
    let title: String
    let detail: String
    let systemImage: String
    let isOn: Bool

    @Environment(\.isFocused) private var isFocused

    var body: some View {
        HStack(alignment: .center, spacing: 22) {
            Image(systemName: systemImage)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 56, height: 56)
                .background {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .fill(isOn ? AnyShapeStyle(TVPalette.brand)
                              : AnyShapeStyle(Color.white.opacity(0.1)))
                }

            VStack(alignment: .leading, spacing: 5) {
                Text(title)
                    .font(.system(size: 25, weight: .semibold))
                Text(detail)
                    .font(TVType.rowDetail)
                    .foregroundStyle(isFocused ? TVPalette.textSecondary : TVPalette.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 24)

            // A desk switch rather than SwiftUI's `Toggle` (on tvOS a Toggle
            // inside a custom row brings its own focus behaviour): an LED that
            // lights in the theme colour above a rocker.
            VStack(spacing: 8) {
                Circle()
                    .fill(isOn ? TVPalette.violet : Color.white.opacity(0.12))
                    .frame(width: 14, height: 14)
                    .shadow(color: isOn ? TVPalette.violet : .clear, radius: 8)
                ZStack(alignment: isOn ? .top : .bottom) {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Color.black.opacity(0.45))
                        .frame(width: 40, height: 62)
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(LinearGradient(colors: [Color(white: 0.85), Color(white: 0.55)],
                                             startPoint: .top, endPoint: .bottom))
                        .frame(width: 32, height: 30)
                        .padding(4)
                }
                .frame(width: 40, height: 62)
            }
            .animation(.spring(response: 0.28, dampingFraction: 0.75), value: isOn)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
        .tvNeonCard(cornerRadius: 22, isFocused: isFocused, isProminent: false)
    }
}
