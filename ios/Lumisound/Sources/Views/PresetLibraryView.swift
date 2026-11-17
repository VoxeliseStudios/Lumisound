import SwiftUI
import UIKit

// MARK: - Preset screens
//
// One list screen for theme, effect and visualizer presets. The rows,
// import checks, file storage and sharing format all come from the Rust core
// (`presetsList` / `presetsImport` / `presetsSource` / `presetsDelete`); each
// screen below only says what "apply" means for its kind.

struct ThemePresetsView: View {
    @ObservedObject private var engine = ThemePresetEngine.shared

    var body: some View {
        PresetLibraryView(
            kind: .theme,
            title: "Theme Presets",
            subtitle: "One-tap looks you can share.",
            icon: "paintpalette.fill",
            tint: .orange,
            intro: "A preset sets the app's colours, fonts, panel and glass style, a few behaviour flags and the Library's default sort in one tap. Applying one overwrites the individual choices in Appearance, Liquid Glass and the Library sort chips. Share a preset as JSON, or import one someone sent you.",
            activeID: engine.activePresetID,
            lastError: engine.lastError,
            apply: { engine.apply($0) ? nil : engine.lastError },
            clearLabel: engine.activePresetID == nil ? nil : "Clear Active Preset",
            clear: {
                engine.clearPreset()
                ToastCenter.shared.show("Theme preset cleared", category: .info, icon: "arrow.counterclockwise")
            },
            clearNote: "Clearing removes the preset marker and the background/layout scale it set. Colours, fonts and styles stay as the preset left them; use Appearance → Reset to Default to restore everything.",
            onDelete: { engine.deleted($0) }
        )
    }
}

struct EffectPresetsView: View {
    @EnvironmentObject private var player: AudioPlayerManager

    var body: some View {
        PresetLibraryView(
            kind: .effect,
            title: "Effect Presets",
            subtitle: "Computed EQ curves and motion.",
            icon: "slider.horizontal.3",
            tint: .purple,
            intro: "Each preset computes its own EQ curve, speed/pitch and optional rotation, tremolo or vibrato, and can follow the playing track: genre curves pick an EQ from the genre tag, and a tremolo can pulse on the beat. Share one as JSON, or import one someone sent you.",
            activeID: EffectPresets.presetID(fromEffectID: player.audioSettings.activeEffectID).map { presetsCanonicalId(kind: .effect, id: $0) },
            lastError: nil,
            apply: { id in
                do {
                    let effect = try EffectPresets.effect(presetID: id, track: player.currentSong)
                    player.applyEffect(effect)
                    return nil
                } catch {
                    return error.localizedDescription
                }
            },
            clearLabel: nil,
            clear: {},
            clearNote: nil,
            onDelete: { _ in }
        )
    }
}

struct VisualizerPresetsView: View {
    @ObservedObject private var store = VisualizerPresetStore.shared

    var body: some View {
        PresetLibraryView(
            kind: .visualizer,
            title: "Visualizer Styles",
            subtitle: "Colours and motion for Live Spectrum.",
            icon: "waveform.path.ecg.rectangle",
            tint: .pink,
            intro: "Restyles the \u{201C}Live Spectrum\u{201D} Now Playing artwork style: gradient colours, sensitivity, bar spacing and rounding, mirroring. Select \u{201C}Live Spectrum\u{201D} under artwork styles to see it.",
            activeID: store.activePresetID,
            lastError: store.lastError,
            apply: { store.apply($0) ? nil : store.lastError },
            clearLabel: store.activePresetID == nil ? nil : "Use Default Style",
            clear: {
                store.clear()
                ToastCenter.shared.show("Visualizer style cleared", category: .info, icon: "arrow.counterclockwise")
            },
            clearNote: nil,
            onDelete: { store.deleted($0) }
        )
    }
}

// MARK: - Shared list

struct PresetLibraryView: View {
    let kind: PresetKind
    let title: String
    let subtitle: String
    let icon: String
    let tint: Color
    let intro: String
    let activeID: String?
    let lastError: String?
    /// Applies the preset with this id; returns an error message on failure.
    let apply: (String) -> String?
    let clearLabel: String?
    let clear: () -> Void
    let clearNote: String?
    let onDelete: (String) -> Void

    @State private var entries: [PresetEntry] = []
    @State private var showImport = false
    @State private var importText = ""
    @State private var importName = ""
    @State private var importError: String?
    @State private var shareItem: PresetShareItem?

    private var bundled: [PresetEntry] { entries.filter(\.bundled) }
    private var imported: [PresetEntry] { entries.filter { !$0.bundled } }

    var body: some View {
        LKPage(title: title, subtitle: subtitle, icon: icon, tint: tint) {
            LKSection {
                Text(intro)
                    .font(AppTheme.bodyFont(size: 13))
                    .foregroundStyle(AppTheme.textSecondary)
            }

            if let lastError {
                LKSection {
                    Label(lastError, systemImage: "exclamationmark.triangle.fill")
                        .font(AppTheme.bodyFont(size: 13))
                        .foregroundStyle(AppTheme.error)
                }
            }

            LKSection {
                ForEach(bundled, id: \.id) { row($0) }
            } header: {
                header("Built In")
            }

            LKSection {
                if imported.isEmpty {
                    Text("No imported presets yet. Tap the import button to paste one someone shared with you.")
                        .font(AppTheme.bodyFont(size: 13))
                        .foregroundStyle(AppTheme.textSecondary)
                } else {
                    ForEach(imported, id: \.id) { row($0) }
                }
            } header: {
                header("Imported")
            }

            if let clearLabel {
                LKSection {
                    Button(role: .destructive, action: clear) {
                        Label(clearLabel, systemImage: "xmark.circle")
                    }
                    .foregroundStyle(AppTheme.dynamicAccent)
                }
                if let clearNote {
                    LKSection {
                        Text(clearNote)
                            .font(AppTheme.bodyFont(size: 12))
                            .foregroundStyle(AppTheme.textSecondary)
                    }
                }
            }
        }
        .background(Color.clear.ignoresSafeArea())
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .navigationBarTrailing) {
                Button {
                    importText = UIPasteboard.general.string ?? ""
                    importName = ""
                    importError = nil
                    showImport = true
                } label: {
                    Image(systemName: "square.and.arrow.down")
                }
                .accessibilityLabel("Import preset")
            }
        }
        .task { reload() }
        .lumiSheet(isPresented: $showImport) { importSheet }
        .lumiSheet(item: $shareItem) { item in
            PresetShareSheet(item: item)
        }
    }

    private func reload() {
        entries = presetsList(kind: kind, documentsDir: PresetPaths.documents)
    }

    private func header(_ text: String) -> some View {
        Text(text)
            .font(AppTheme.bodyFont())
            .foregroundStyle(AppTheme.textSecondary)
    }

    private func row(_ entry: PresetEntry) -> some View {
        let isActive = activeID == entry.id
        return Button {
            if let error = entry.error {
                ToastCenter.shared.show(error, category: .error, icon: "exclamationmark.triangle.fill")
                return
            }
            if let error = apply(entry.id) {
                ToastCenter.shared.show(error, category: .error, icon: "exclamationmark.triangle.fill")
            } else {
                ToastCenter.shared.show("Applied \u{201C}\(entry.name)\u{201D}", category: .success, icon: entry.icon)
            }
        } label: {
            HStack(spacing: 14) {
                PresetSwatch(kind: kind, entry: entry)
                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.name)
                        .font(AppTheme.headlineFont(size: 15))
                        .foregroundStyle(AppTheme.textPrimary)
                    Text(entry.error ?? entry.subtitle)
                        .font(AppTheme.bodyFont(size: 12))
                        .foregroundStyle(entry.error == nil ? AppTheme.textSecondary : AppTheme.error)
                        .lineLimit(2)
                }
                Spacer()
                if isActive {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(AppTheme.dynamicAccent)
                }
            }
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            if entry.error == nil {
                Button {
                    share(entry)
                } label: {
                    Label("Share as JSON", systemImage: "square.and.arrow.up")
                }
            }
            if !entry.bundled {
                Button(role: .destructive) {
                    do {
                        try presetsDelete(kind: kind, documentsDir: PresetPaths.documents, id: entry.id)
                        onDelete(entry.id)
                        reload()
                    } catch {
                        ToastCenter.shared.show(error.localizedDescription, category: .error, icon: "exclamationmark.triangle.fill")
                    }
                } label: {
                    Label("Delete", systemImage: "trash")
                }
            }
        }
    }

    private func share(_ entry: PresetEntry) {
        do {
            let json = try presetsSource(kind: kind, documentsDir: PresetPaths.documents, id: entry.id)
            shareItem = PresetShareItem(name: entry.name, json: json)
        } catch {
            ToastCenter.shared.show(error.localizedDescription, category: .error, icon: "exclamationmark.triangle.fill")
        }
    }

    private var importSheet: some View {
        NavigationStack {
            LKScroll {
                LKSection("Preset Name") {
                    TextField("Optional — the preset's own name is used otherwise", text: $importName)
                        .autocorrectionDisabled()
                }
                LKSection("Preset (JSON)") {
                    TextEditor(text: $importText)
                        .font(.system(.footnote, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .frame(minHeight: 220)
                }
                if let importError {
                    LKSection {
                        Label(importError, systemImage: "exclamationmark.triangle.fill")
                            .font(.caption)
                            .foregroundStyle(AppTheme.error)
                    }
                }
                LKSection {
                    Text("Paste a preset someone shared with you. To see the format, long-press a built-in preset and choose Share as JSON; it can be edited and imported back.")
                        .font(.caption)
                        .foregroundStyle(AppTheme.textSecondary)
                }
            }
            .navigationTitle("Import Preset")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { showImport = false }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import") {
                        do {
                            let entry = try presetsImport(kind: kind, documentsDir: PresetPaths.documents, source: importText,
                                                          suggestedName: importName.trimmingCharacters(in: .whitespaces))
                            reload()
                            showImport = false
                            ToastCenter.shared.show("Imported \u{201C}\(entry.name)\u{201D}", category: .success, icon: "square.and.arrow.down")
                        } catch {
                            importError = error.localizedDescription
                        }
                    }
                    .disabled(importText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

struct PresetShareItem: Identifiable {
    let id = UUID()
    let name: String
    let json: String
}

private struct PresetShareSheet: View {
    let item: PresetShareItem
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            LKScroll {
                LKSection {
                    Text(item.json)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                LKSection {
                    Button {
                        UIPasteboard.general.string = item.json
                        ToastCenter.shared.show("Copied", category: .success, icon: "doc.on.doc")
                    } label: {
                        Label("Copy JSON", systemImage: "doc.on.doc")
                    }
                    ShareLink(item: item.json, subject: Text(item.name)) {
                        Label("Share…", systemImage: "square.and.arrow.up")
                    }
                }
            }
            .navigationTitle(item.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

private struct PresetSwatch: View {
    let kind: PresetKind
    let entry: PresetEntry

    var body: some View {
        switch kind {
        case .theme where entry.swatch.count >= 2:
            ZStack {
                Circle().fill(Color(hex: entry.swatch[0])).frame(width: 40, height: 40)
                Circle().fill(Color(hex: entry.swatch[1])).frame(width: 18, height: 18).offset(x: 10, y: 10)
            }
            .overlay(Circle().strokeBorder(.white.opacity(0.12), lineWidth: 1).frame(width: 40, height: 40))
        case .visualizer where !entry.swatch.isEmpty:
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(LinearGradient(colors: entry.swatch.map { Color(hex: $0) }, startPoint: .leading, endPoint: .trailing))
                .frame(width: 40, height: 40)
                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.white.opacity(0.12), lineWidth: 1))
        default:
            Image(systemName: entry.icon)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(AppTheme.dynamicAccent)
                .frame(width: 40, height: 40)
                .background(Circle().fill(AppTheme.dynamicAccent.opacity(0.14)))
        }
    }
}
