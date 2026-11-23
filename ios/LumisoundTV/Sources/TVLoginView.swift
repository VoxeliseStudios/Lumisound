import SwiftUI

// MARK: - TVLoginView — "Box Office"
//
// Signing in is buying your ticket: an admission ticket for the show on the
// left (the mark, the name, the night's features, a serial number and a
// perforated stub), and the box-office window with the form on the right.
struct TVLoginView: View {
    @ObservedObject var account: TVAccount
    @Environment(\.tvTheme) private var theme

    @State private var username = ""
    @State private var password = ""
    @State private var glow = false

    var body: some View {
        ZStack {
            TVAmbientBackground()

            HStack(spacing: 0) {
                brandSide
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                formSide
                    .frame(width: 760)
                    .frame(maxHeight: .infinity)
                    .focusSection()
            }
        }
    }

    private var brandSide: some View {
        ZStack {
            RadialGradient(colors: [TVPalette.violet.opacity(glow ? 0.55 : 0.35), .clear],
                           center: .init(x: 0.4, y: 0.4), startRadius: 0, endRadius: 800)
            ticket
                .rotationEffect(.degrees(-4))
        }
        .animation(.easeInOut(duration: 3).repeatForever(autoreverses: true), value: glow)
        .onAppear { glow = true }
    }

    /// ADMIT ONE: body and stub divided by a perforation.
    private var ticket: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 22) {
                HStack(spacing: 18) {
                    TVBrandMark(height: 64)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("ADMIT ONE")
                            .font(.system(size: 18, weight: .black, design: .monospaced))
                            .tracking(4)
                            .foregroundStyle(Color.black.opacity(0.6))
                        Text("Lumisound")
                            .font(theme.display(64))
                            .foregroundStyle(Color.black.opacity(0.88))
                    }
                }
                Text("Your music, on the big screen.")
                    .font(.system(size: 26, weight: .semibold, design: theme.design))
                    .foregroundStyle(Color.black.opacity(0.7))
                HStack(spacing: 12) {
                    feature("square.stack.fill", "Cloud library")
                    feature("music.note.list", "Mixtapes")
                    feature("quote.bubble.fill", "Live lyrics")
                }
                Spacer(minLength: 0)
                Text("NO. \(serial) · GENERAL ADMISSION")
                    .font(.system(size: 15, weight: .bold, design: .monospaced))
                    .foregroundStyle(Color.black.opacity(0.45))
            }
            .padding(44)
            .frame(width: 720, height: 420, alignment: .topLeading)

            // Perforation
            VStack(spacing: 10) {
                ForEach(0..<16, id: \.self) { _ in
                    Circle().fill(Color.black.opacity(0.28)).frame(width: 8, height: 8)
                }
            }
            .frame(width: 20)

            // Stub
            VStack(spacing: 14) {
                Text("SEAT")
                    .font(.system(size: 15, weight: .black, design: .monospaced))
                    .foregroundStyle(Color.black.opacity(0.5))
                Text("A1")
                    .font(theme.display(54))
                    .foregroundStyle(Color.black.opacity(0.85))
                Text("ROW · FRONT")
                    .font(.system(size: 13, weight: .bold, design: .monospaced))
                    .foregroundStyle(Color.black.opacity(0.45))
            }
            .frame(width: 180, height: 420)
        }
        .background {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(LinearGradient(colors: [TVPalette.neon, TVPalette.neonAlt],
                                     startPoint: .topLeading, endPoint: .bottomTrailing))
        }
        .shadow(color: TVPalette.violet.opacity(0.5), radius: 50, y: 24)
    }

    /// The ticket's serial: stable for this Apple TV (from its name), not random.
    private var serial: String {
        let n = UIDevice.current.name.unicodeScalars.reduce(UInt32(7)) { ($0 &* 31) &+ $1.value }
        return String(format: "%06d", n % 1_000_000)
    }

    private func feature(_ systemImage: String, _ text: String) -> some View {
        Label(text, systemImage: systemImage)
            .font(.system(size: 18, weight: .bold, design: TVTheme.current.design))
            .foregroundStyle(Color.black.opacity(0.72))
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .background(Color.black.opacity(0.08), in: Capsule())
    }

    private var formSide: some View {
        VStack(alignment: .leading, spacing: 26) {
            VStack(alignment: .leading, spacing: 8) {
                Text("BOX OFFICE")
                    .font(TVType.eyebrow)
                    .tracking(theme.tracking)
                    .foregroundStyle(TVPalette.neonAlt)
                Text("Collect your ticket")
                    .font(theme.display(46, .bold))
                Text("Use the same account as the Lumisound iPhone app.")
                    .font(TVType.rowDetail)
                    .foregroundStyle(TVPalette.textSecondary)
            }

            VStack(spacing: 18) {
                TextField("Username", text: $username)
                    .textContentType(.username)
                    .autocorrectionDisabled()
                SecureField("Password", text: $password)
                    .textContentType(.password)
            }

            if let err = account.errorText {
                Label(err, systemImage: "exclamationmark.triangle.fill")
                    .font(TVType.rowDetail)
                    .foregroundStyle(Color(red: 1.0, green: 0.45, blue: 0.5))
            }

            Button {
                Task { await account.login(username: username, password: password) }
            } label: {
                TVPillLabel(title: "Sign In", systemImage: "arrow.right",
                            width: 540, isLoading: account.isLoggingIn)
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()
            .disabled(account.isLoggingIn || username.isEmpty || password.isEmpty)
            .opacity(username.isEmpty || password.isEmpty ? 0.6 : 1)
            .padding(.top, 6)
        }
        .padding(56)
        .tvGlassPanel(cornerRadius: 40)
        .padding(.trailing, 100)
    }
}
