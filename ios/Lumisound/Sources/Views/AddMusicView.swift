import SwiftUI

struct AddMusicView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var library: LibraryManager

    @State private var showFilePicker = false
    @State private var importSuccess: String? = nil

    var body: some View {
        NavigationStack {
            // 2026-11-15 redesign: one big primary action, then the other
            // ways in as cards, instead of a settings-style list.
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    VStack(alignment: .leading, spacing: 6) {
                        Image(systemName: "music.note.house.fill")
                            .font(.system(size: 34, weight: .semibold))
                            .foregroundStyle(AppTheme.dynamicAccentGradient)
                        Text("Bring your music")
                            .font(.system(size: 30, weight: .bold, design: .serif))
                            .foregroundStyle(AppTheme.textPrimary)
                        Text(library.allSongs.isEmpty
                             ? "Your library is empty — start with a few files."
                             : "\(library.allSongs.count.formatted()) songs in your library so far.")
                            .font(.subheadline)
                            .foregroundStyle(AppTheme.textSecondary)
                    }
                    .padding(.horizontal, 4)

                    Button { showFilePicker = true } label: {
                        VStack(alignment: .leading, spacing: 14) {
                            HStack {
                                Image(systemName: "doc.badge.plus")
                                    .font(.system(size: 22, weight: .bold))
                                    .frame(width: 48, height: 48)
                                    .background(Circle().fill(.white.opacity(0.22)))
                                Spacer()
                                Image(systemName: "arrow.up.right")
                                    .font(.headline.weight(.bold))
                            }
                            VStack(alignment: .leading, spacing: 4) {
                                Text("Import from Files")
                                    .font(.title2.weight(.bold))
                                Text("Downloads, iCloud Drive, a USB drive — pick as many songs as you like.")
                                    .font(.subheadline)
                                    .opacity(0.9)
                            }
                            HStack(spacing: 6) {
                                ForEach(["MP3", "M4A", "FLAC", "WAV", "OPUS", "ALAC"], id: \.self) { format in
                                    Text(format)
                                        .font(.caption2.weight(.heavy))
                                        .padding(.horizontal, 8)
                                        .padding(.vertical, 4)
                                        .background(Capsule().fill(.white.opacity(0.2)))
                                }
                            }
                        }
                        .foregroundStyle(.white)
                        .padding(20)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 26, style: .continuous).fill(AppTheme.dynamicAccentGradient))
                    }
                    .buttonStyle(PressableButtonStyle())

                    NavigationLink {
                        UploadMusicView()
                    } label: {
                        addCard(icon: "icloud.and.arrow.up", title: "Cloud Backup & Upload",
                                detail: "Keep tracks in your Lumisound account and restore them on any device.")
                    }
                    .buttonStyle(PressableButtonStyle())

                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 12) {
                            iconBadge("laptopcomputer", color: Color(white: 0.42))
                            Text("From a computer")
                                .font(.headline)
                                .foregroundStyle(AppTheme.textPrimary)
                        }
                        transferStep(1, "Connect your iPhone (USB or the same Wi‑Fi).")
                        transferStep(2, "Mac: Finder → your iPhone → Files. Windows: the Apple Devices app → Files.")
                        transferStep(3, "Drag songs or whole folders onto Lumisound — folders become Folders in your library.")
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .cellSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous), fallback: AppTheme.surface.opacity(0.7))

                    #if DEBUG
                    Button {
                        if let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first {
                            library.scanSpecificDirectory(docs)
                            importSuccess = "Scanning app storage…"
                        }
                    } label: {
                        addCard(icon: "folder.fill", title: "Scan App Storage (debug)",
                                detail: "Rescans files transferred into the app's own folder.")
                    }
                    .buttonStyle(PressableButtonStyle())
                    #endif

                    if let msg = importSuccess {
                        Label(msg, systemImage: "checkmark.circle.fill")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(AppTheme.success)
                            .padding(.horizontal, 4)
                    }
                }
                .padding(20)
            }
            .background(EditionScreenBackground())
            .navigationTitle("Add Music")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }.tint(AppTheme.dynamicAccent)
                }
            }
            .sheet(isPresented: $showFilePicker) {
                DocumentPicker(mode: .audioFiles) { urls in
                    showFilePicker = false
                    guard !urls.isEmpty else { return }
                    library.importFiles(urls: urls)
                    // importFiles runs asynchronously (and can fail — surfaced via
                    // library.errorMessage elsewhere), so claiming "imported" here
                    // would be premature; match the in-progress phrasing used above
                    // for the other two import paths.
                    importSuccess = "Importing \(urls.count) file\(urls.count == 1 ? "" : "s")…"
                }
                .ignoresSafeArea()
            }
        }
    }

    private func addCard(icon: String, title: String, detail: String) -> some View {
        HStack(spacing: 14) {
            iconBadge(icon, color: AppTheme.dynamicAccent)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.headline).foregroundStyle(AppTheme.textPrimary)
                Text(detail).font(.caption).foregroundStyle(AppTheme.textSecondary).multilineTextAlignment(.leading)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right").font(.caption.weight(.bold)).foregroundStyle(AppTheme.textSecondary)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .cellSurface(in: RoundedRectangle(cornerRadius: 22, style: .continuous), fallback: AppTheme.surface.opacity(0.7))
    }

    private func transferStep(_ n: Int, _ text: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text("\(n)")
                .font(.caption.weight(.heavy))
                .foregroundStyle(.white)
                .frame(width: 22, height: 22)
                .background(Circle().fill(AppTheme.dynamicAccent))
            Text(text)
                .font(.subheadline)
                .foregroundStyle(AppTheme.textPrimary.opacity(0.9))
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func importRow(icon: String, color: Color, title: String, subtitle: String) -> some View {
        HStack(spacing: 14) {
            iconBadge(icon, color: color)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).fontWeight(.medium).foregroundStyle(AppTheme.textPrimary)
                Text(subtitle).font(.caption).foregroundStyle(AppTheme.textSecondary)
            }
        }
    }

    /// Consistent rounded-square icon badge used by every Add Music row, so the
    /// rows share one visual language instead of mixing bare glyphs and badges.
    private func iconBadge(_ systemName: String, color: Color) -> some View {
        ZStack {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(color)
                .frame(width: 40, height: 40)
            Image(systemName: systemName)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(.white)
        }
    }

    private func sectionLabel(_ text: String) -> some View {
        Text(text.uppercased())
            .font(AppTheme.bodyFont(size: 11))
            .foregroundStyle(AppTheme.textSecondary)
            .kerning(0.8)
    }
}
