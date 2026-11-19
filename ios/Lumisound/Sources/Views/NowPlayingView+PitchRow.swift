import SwiftUI
import UIKit

extension NowPlayingView {

    // MARK: - Pitch Row

    var pitchRow: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Pitch")
                    .font(.subheadline)
                    .foregroundStyle(AppTheme.textPrimary)
                Spacer()
                if editingPitch {
                    TextField("", text: $pitchInput)
                        .keyboardType(.numbersAndPunctuation)
                        .multilineTextAlignment(.trailing)
                        .frame(width: 80)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit { applyPitchInput() }
                        .onAppear { pitchInput = String(format: "%.1f", player.audioSettings.pitchSemitones) }
                        .toolbar {
                            ToolbarItemGroup(placement: .keyboard) {
                                Spacer()
                                Button("Done") { applyPitchInput() }
                            }
                        }
                } else {
                    HStack(spacing: 4) {
                        // One decimal: a whole-number label hid fractional
                        // pitches (−0.64 showed as "−1 st"), so a nudged
                        // slider went unnoticed.
                        Text(String(format: "%+.1f st", player.audioSettings.pitchSemitones))
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(AppTheme.textSecondary)
                            .onTapGesture { editingPitch = true }
                        Image(systemName: "pencil")
                            .font(.caption2)
                            .foregroundStyle(AppTheme.textSecondary)
                            .onTapGesture { editingPitch = true }
                    }
                }
            }
            HStack(spacing: 10) {
                // Half-semitone steps (same as Settings): a continuous slider
                // brushed while swiping between songs left values like −0.64
                // that were saved and replayed on every later track.
                Button {
                    pitchUnlocked.toggle()
                } label: {
                    Image(systemName: pitchUnlocked ? "lock.open.fill" : "lock.fill")
                        .font(.caption.weight(.bold))
                        .foregroundStyle(pitchUnlocked ? AppTheme.dynamicAccent : AppTheme.textSecondary)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(pitchUnlocked ? "Lock pitch" : "Unlock pitch")
                Slider(value: audioBinding(\.pitchSemitones), in: -12...12, step: 0.5)
                    .tint(AppTheme.dynamicAccent)
                    .disabled(!pitchUnlocked)
                    .opacity(pitchUnlocked ? 1 : 0.45)
                    // Locked: touches fall through, so a swipe across it moves the page, not the pitch.
                    .allowsHitTesting(pitchUnlocked)
                    // Re-locks 6 s after the last change (each change restarts the wait).
                    .task(id: "\(pitchUnlocked)|\(player.audioSettings.pitchSemitones)") {
                        guard pitchUnlocked else { return }
                        try? await Task.sleep(for: .seconds(6))
                        if !Task.isCancelled { pitchUnlocked = false }
                    }
                    .onDisappear { pitchUnlocked = false }
                if player.audioSettings.pitchSemitones != 0 {
                    Button {
                        var settings = player.audioSettings
                        settings.pitchSemitones = 0
                        player.audioSettings = settings
                    } label: {
                        Image(systemName: "arrow.counterclockwise")
                            .font(.caption.weight(.bold))
                            .foregroundStyle(AppTheme.dynamicAccent)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Reset pitch")
                }
            }
        }
    }
}
