import SwiftUI
import UIKit

extension NowPlayingView {

    // MARK: - Speed Row

    var speedRow: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Speed")
                    .font(.subheadline)
                    .foregroundStyle(AppTheme.textPrimary)
                Spacer()
                if editingSpeed {
                    TextField("", text: $speedInput)
                        .keyboardType(.decimalPad)
                        .multilineTextAlignment(.trailing)
                        .frame(width: 80)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit { applySpeedInput() }
                        .onAppear { speedInput = String(format: "%.2f", player.audioSettings.speed) }
                        .toolbar {
                            ToolbarItemGroup(placement: .keyboard) {
                                Spacer()
                                Button("Done") { applySpeedInput() }
                            }
                        }
                } else {
                    HStack(spacing: 4) {
                        Text(String(format: "%.2fx", player.audioSettings.speed))
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(AppTheme.textSecondary)
                            .onTapGesture { editingSpeed = true }
                        Image(systemName: "pencil")
                            .font(.caption2)
                            .foregroundStyle(AppTheme.textSecondary)
                            .onTapGesture { editingSpeed = true }
                    }
                }
            }
            HStack(spacing: 10) {
                // 0.05× steps — see the pitch row for why it isn't continuous.
                Slider(value: audioBinding(\.speed), in: 0.5...2.0, step: 0.05)
                    .tint(AppTheme.dynamicAccent)
                if player.audioSettings.speed != 1 {
                    Button {
                        var settings = player.audioSettings
                        settings.speed = 1
                        player.audioSettings = settings
                    } label: {
                        Image(systemName: "arrow.counterclockwise")
                            .font(.caption.weight(.bold))
                            .foregroundStyle(AppTheme.dynamicAccent)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Reset speed")
                }
            }
        }
    }
}
