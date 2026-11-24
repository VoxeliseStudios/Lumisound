import SwiftUI

// MARK: - Aria Lyrics sheet ("Generate with Aria")
//
// The whole run lives in the Rust core (`AriaLyricsJob`, core aria_lyrics.rs):
// upload or cloud path, following the bridge job, progress, the result and
// saving it. This file only draws it:
//
//   working  — Aria's orb, a three-step checklist (separate → listen → time),
//              the overall bar, and a live caption of the words she just heard;
//   finished — the headline, a preview of every line with its time, and
//              "Use these lyrics" (saved next to the track AND for the account,
//              so the Apple TV and Android see them) / Discard / Try again;
//   failed   — the reason in plain words, and Try again.
//
// Nothing is saved until "Use these lyrics": a bad transcription is
// discarded, never written over lyrics that were fine.

/// Bridges the core's progress callbacks (any thread) onto the main actor.
final class AriaLyricsSession: ObservableObject, AriaLyricsObserver, @unchecked Sendable {
    enum Phase {
        case working
        case finished(AriaLyricsResult)
        case failed(String)
    }

    @Published var phase: Phase = .working
    @Published var progress: AriaLyricsProgress?
    /// The last few phrases Aria heard, newest last.
    @Published var heard: [String] = []

    private var job: AriaLyricsJob?

    func onProgress(progress: AriaLyricsProgress) {
        DispatchQueue.main.async {
            self.progress = progress
            if let h = progress.heard, h != self.heard.last {
                self.heard = Array((self.heard + [h]).suffix(3))
            }
        }
    }

    @MainActor
    func start(song: Song, base: String, token: String, hint: String?) {
        phase = .working
        progress = nil
        heard = []
        let job = AriaLyricsJob(baseUrl: base, token: token)
        self.job = job
        let title = song.title.isEmpty ? song.displayName : song.title
        let artist = song.artistName
        Task { @MainActor in
            do {
                let result: AriaLyricsResult
                if let url = song.url, url.isFileURL {
                    // A locked (.lms) file is uploaded as its unlocked view.
                    let file = LumisoundExclusiveExtensionService.playableURL(for: url)
                    result = try await job.runUpload(filePath: file.path, title: title, artist: artist, hint: hint, observer: self)
                } else if let url = song.url, let cloud = ariaLyricsCloudPath(url: url.absoluteString) {
                    result = try await job.runCloud(cloudPath: cloud, title: title, artist: artist, hint: hint, observer: self)
                } else {
                    phase = .failed("Aria can only write lyrics for songs in your library or your cloud library.")
                    return
                }
                phase = .finished(result)
            } catch let CoreError.Network(_, _, cancelled) where cancelled {
                return
            } catch let e as CoreError {
                // The core's messages are written for people; show them as-is.
                let message: String
                switch e {
                case let .Http(_, m): message = m
                case let .Network(m, _, _): message = m
                case let .Invalid(m): message = m
                case let .Io(m): message = m
                case .Unauthorized: message = "Your session expired. Sign in again to use Aria."
                }
                appWarn("Aria lyrics failed for \"\(title)\": \(message)", category: "network")
                phase = .failed(message)
            } catch {
                appWarn("Aria lyrics failed for \"\(title)\": \(error.localizedDescription)", category: "network")
                phase = .failed(error.localizedDescription)
            }
        }
    }

    func cancel() {
        guard let job else { return }
        Task { await job.cancel() }
    }

    /// Stores accepted lyrics for the account (best effort; the local save is
    /// what the player reads).
    func share(title: String, artist: String, result: AriaLyricsResult) {
        guard let job else { return }
        Task {
            do { try await job.share(title: title, artist: artist, result: result) }
            catch { appWarn("Aria lyrics share failed: \(error.localizedDescription)", category: "network") }
        }
    }
}

struct AriaLyricsSheet: View {
    let song: Song
    /// Untimed text already on screen: Aria aligns to it instead of writing
    /// the words from scratch.
    let hint: String?
    /// Called with the accepted result; returns false if saving failed.
    let onUse: (AriaLyricsResult) -> Bool

    @EnvironmentObject private var account: AccountService
    @Environment(\.dismiss) private var dismiss
    @StateObject private var session = AriaLyricsSession()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 22) {
                    header
                    switch session.phase {
                    case .working: working
                    case .finished(let result): finished(result)
                    case .failed(let message): failed(message)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 30)
            }
            .background(AppTheme.background.ignoresSafeArea())
            .navigationTitle("Aria Lyrics")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(isWorking ? "Cancel" : "Close") {
                        if isWorking { session.cancel() }
                        dismiss()
                    }
                }
            }
        }
        .onAppear(perform: start)
        .onDisappear { if isWorking { session.cancel() } }
    }

    private var isWorking: Bool {
        if case .working = session.phase { return true }
        return false
    }

    private func start() {
        guard let token = account.token, let streaming = StreamingService.shared else {
            session.phase = .failed("Sign in to let Aria write lyrics.")
            return
        }
        session.start(song: song, base: streaming.bridgeURL, token: token, hint: hint)
    }

    // MARK: Header

    private var header: some View {
        VStack(spacing: 10) {
            AriaLyricsOrb(isActive: isWorking, pulse: session.heard.count)
                .frame(width: 112, height: 112)
                .padding(.top, 12)
            Text(song.title.isEmpty ? song.displayName : song.title)
                .font(.title3.weight(.bold))
                .foregroundStyle(AppTheme.textPrimary)
                .multilineTextAlignment(.center)
                .lineLimit(2)
            if !song.artistName.isEmpty {
                Text(song.artistName)
                    .font(.subheadline)
                    .foregroundStyle(AppTheme.textSecondary)
            }
        }
    }

    // MARK: Working

    private var working: some View {
        let p = session.progress
        return VStack(spacing: 20) {
            VStack(spacing: 8) {
                HStack {
                    Text(p?.title ?? "Getting ready")
                        .font(.headline)
                        .foregroundStyle(AppTheme.textPrimary)
                    Spacer()
                    Text("\(Int(((p?.fraction ?? 0) * 100).rounded()))%")
                        .font(.subheadline.monospacedDigit().weight(.semibold))
                        .foregroundStyle(AppTheme.dynamicAccent)
                }
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(AppTheme.textSecondary.opacity(0.15))
                        Capsule()
                            .fill(LinearGradient(colors: [AppTheme.dynamicAccent, AppTheme.dynamicAccentSecondary],
                                                 startPoint: .leading, endPoint: .trailing))
                            .frame(width: max(8, geo.size.width * (p?.fraction ?? 0)))
                    }
                }
                .frame(height: 8)
                .animation(.easeOut(duration: 0.6), value: p?.fraction)
                HStack {
                    Text(p?.detail ?? "Connecting to Aria")
                    Spacer()
                    Text(elapsedLabel(p?.elapsedSecs ?? 0)).monospacedDigit()
                }
                .font(.caption)
                .foregroundStyle(AppTheme.textSecondary)
            }

            VStack(alignment: .leading, spacing: 0) {
                AriaLyricsStepRow(icon: "waveform.path", title: "Separate the vocals", step: 1, current: p?.step ?? 0)
                AriaLyricsStepRow(icon: "ear", title: "Listen to every phrase", step: 2, current: p?.step ?? 0)
                AriaLyricsStepRow(icon: "metronome", title: "Time each line to the music", step: 3, current: p?.step ?? 0)
            }
            .padding(14)
            .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(AppTheme.surface))

            if !session.heard.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Aria hears", systemImage: "quote.opening")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(AppTheme.dynamicAccent)
                    ForEach(Array(session.heard.enumerated()), id: \.offset) { index, phrase in
                        Text(phrase)
                            .font(.system(.body, design: .serif).italic())
                            .foregroundStyle(AppTheme.textPrimary.opacity(index == session.heard.count - 1 ? 1 : 0.4))
                            .transition(.opacity.combined(with: .move(edge: .bottom)))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(AppTheme.surface))
                .animation(.easeOut(duration: 0.35), value: session.heard)
            }

            Text("Aria runs on the Lumisound server — you can leave this open or close it to stop.")
                .font(.caption2)
                .foregroundStyle(AppTheme.textSecondary.opacity(0.8))
                .multilineTextAlignment(.center)
        }
    }

    // MARK: Finished

    @ViewBuilder
    private func finished(_ result: AriaLyricsResult) -> some View {
        VStack(spacing: 16) {
            Text(result.headline)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(AppTheme.textPrimary)
                .multilineTextAlignment(.center)
            if let aligned = result.aligned {
                Text("Matched \(Int((aligned * 100).rounded()))% of the known lyrics to the music")
                    .font(.caption)
                    .foregroundStyle(AppTheme.textSecondary)
            }
            if !result.synced && !result.lines.isEmpty {
                Text("Aria heard the words but couldn't time them, so they'll show as a static block.")
                    .font(.caption)
                    .foregroundStyle(AppTheme.textSecondary)
                    .multilineTextAlignment(.center)
            }

            if !result.lines.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(result.lines.enumerated()), id: \.offset) { _, line in
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            if let label = line.timeLabel {
                                Text(label)
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(AppTheme.dynamicAccent)
                                    .frame(width: 40, alignment: .trailing)
                            }
                            Text(line.text)
                                .font(.body)
                                .foregroundStyle(AppTheme.textPrimary)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(AppTheme.surface))

                Button {
                    if onUse(result) {
                        session.share(title: song.title.isEmpty ? song.displayName : song.title,
                                      artist: song.artistName, result: result)
                        dismiss()
                    }
                } label: {
                    Label("Use these lyrics", systemImage: "checkmark")
                        .font(.headline)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 14)
                        .background(Capsule().fill(AppTheme.dynamicAccent))
                        .foregroundStyle(.white)
                }
                .buttonStyle(.plain)
            }

            HStack(spacing: 12) {
                Button("Try again", action: start)
                    .buttonStyle(.bordered)
                Button(result.lines.isEmpty ? "Close" : "Discard") { dismiss() }
                    .buttonStyle(.bordered)
            }
            .tint(AppTheme.dynamicAccent)
        }
    }

    // MARK: Failed

    private func failed(_ message: String) -> some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.bubble")
                .font(.system(size: 34))
                .foregroundStyle(AppTheme.textSecondary)
            Text(message)
                .font(.body)
                .foregroundStyle(AppTheme.textPrimary)
                .multilineTextAlignment(.center)
            Button("Try again", action: start)
                .buttonStyle(.borderedProminent)
                .tint(AppTheme.dynamicAccent)
        }
        .padding(.top, 10)
    }

    private func elapsedLabel(_ secs: UInt64) -> String {
        String(format: "%d:%02d", Int(secs / 60), Int(secs % 60))
    }
}

/// One row of the checklist: done (check), current (lit, pulsing), or ahead.
private struct AriaLyricsStepRow: View {
    let icon: String
    let title: String
    let step: UInt32
    let current: UInt32

    var body: some View {
        let done = current > step
        let active = current == step
        HStack(spacing: 14) {
            ZStack {
                Circle()
                    .fill(done || active ? AppTheme.dynamicAccent.opacity(active ? 0.22 : 0.9) : AppTheme.textSecondary.opacity(0.12))
                    .frame(width: 34, height: 34)
                Image(systemName: done ? "checkmark" : icon)
                    .font(.system(size: 14, weight: .bold))
                    .foregroundStyle(done ? Color.white : (active ? AppTheme.dynamicAccent : AppTheme.textSecondary))
            }
            .scaleEffect(active ? 1.06 : 1)
            .animation(active ? .easeInOut(duration: 0.9).repeatForever(autoreverses: true) : .default, value: active)
            Text(title)
                .font(.subheadline.weight(active ? .semibold : .regular))
                .foregroundStyle(done || active ? AppTheme.textPrimary : AppTheme.textSecondary)
            Spacer()
        }
        .padding(.vertical, 7)
    }
}

/// Aria's orb: a slowly turning ring of the accent colours around a soft core.
/// Transform/opacity animation only (60 fps, cheap); it swells briefly each
/// time she hears a new phrase.
private struct AriaLyricsOrb: View {
    let isActive: Bool
    let pulse: Int

    @State private var spin = false
    @State private var swell = false

    var body: some View {
        ZStack {
            Circle()
                .fill(RadialGradient(colors: [AppTheme.dynamicAccent.opacity(0.45), .clear],
                                     center: .center, startRadius: 4, endRadius: 60))
                .scaleEffect(swell ? 1.15 : 1)
            Circle()
                .strokeBorder(AngularGradient(colors: [AppTheme.dynamicAccent, AppTheme.dynamicAccentSecondary,
                                                       AppTheme.dynamicAccent.opacity(0.2), AppTheme.dynamicAccent],
                                              center: .center), lineWidth: 5)
                .rotationEffect(.degrees(spin ? 360 : 0))
                .padding(10)
            Image(systemName: "sparkles")
                .font(.system(size: 30, weight: .semibold))
                .foregroundStyle(AppTheme.dynamicAccent)
                .opacity(isActive ? 1 : 0.85)
        }
        .onAppear {
            withAnimation(.linear(duration: 6).repeatForever(autoreverses: false)) { spin = true }
        }
        .onChange(of: pulse) { _, _ in
            withAnimation(.easeOut(duration: 0.25)) { swell = true }
            withAnimation(.easeIn(duration: 0.45).delay(0.25)) { swell = false }
        }
    }
}
