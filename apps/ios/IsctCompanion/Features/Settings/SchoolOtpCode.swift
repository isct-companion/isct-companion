import IsctCore
import SwiftUI
import UniformTypeIdentifiers

struct SchoolOtpCodeButton: View {
    @State private var opened = false

    var body: some View {
        Button(t("settings.otp.showCode"), systemImage: "123.rectangle") { opened = true }
            .sheet(isPresented: $opened) {
                SchoolOtpCodeSheet()
                    .presentationDetents([.medium])
                    .presentationBackground(Theme.bg)
            }
    }
}

private struct SchoolOtpCodeSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @State private var current: SchoolOtpCode?
    @State private var seconds = 0
    @State private var copied = false
    @State private var unavailable = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                if scenePhase == .active, let current, seconds > 0 {
                    HStack(spacing: 16) {
                        // The code is the copy target too, as in an authenticator.
                        Button { copy(current) } label: { OtpDigits(otp: current.otp) }
                            .buttonStyle(.plain)
                            .accessibilityLabel("\(current.otp), \(t("settings.otp.copyCode"))")
                        Spacer(minLength: 0)
                        OtpCountdown(
                            expiresAt: Date(timeIntervalSince1970: TimeInterval(current.expiresAt)),
                            period: TimeInterval(current.period),
                            seconds: seconds)
                    }
                    .padding(.horizontal, 20)
                    .padding(.vertical, 18)
                    .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusColumn, style: .continuous))
                    Button {
                        copy(current)
                    } label: {
                        Label(t(copied ? "settings.credentialImport.copied" : "settings.otp.copyCode"),
                              systemImage: copied ? "checkmark" : "doc.on.doc")
                            .font(.body.weight(.semibold))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                    }
                    .buttonStyle(CapsuleButtonStyle())
                } else if unavailable {
                    Text(t("settings.otp.codeUnavailable"))
                        .foregroundStyle(.secondary)
                } else {
                    ProgressView()
                }
            }
            .padding()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .navigationTitle(t("settings.otp.currentCode"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("common.done")) { dismiss() }
                }
            }
        }
        .task(id: scenePhase) { await update() }
    }

    private func update() async {
        current = nil
        unavailable = false
        copied = false
        guard scenePhase == .active else { return }
        do {
            while !Task.isCancelled {
                if current.map({ remaining($0) <= 0 }) ?? true {
                    current = nil
                    copied = false
                    let next = try await model.schoolOtpCode()
                    try Task.checkCancellation()
                    guard let next else {
                        unavailable = true
                        return
                    }
                    current = next
                }
                seconds = current.map { Int(ceil(remaining($0))) } ?? 0
                let fraction = Date().timeIntervalSince1970.truncatingRemainder(dividingBy: 1)
                try await Task.sleep(for: .seconds(1 - fraction))
            }
        } catch {
            if !Task.isCancelled { unavailable = true }
        }
    }

    private func remaining(_ code: SchoolOtpCode) -> TimeInterval {
        TimeInterval(code.expiresAt) - Date().timeIntervalSince1970
    }

    private func copy(_ code: SchoolOtpCode) {
        let expiry = Date(timeIntervalSince1970: TimeInterval(code.expiresAt))
        guard expiry > .now else { return }
        copySensitiveOtp(code.otp, expiresAt: expiry)
        copied = true
    }
}

/// Grouped as authenticators show it: 123 456.
private struct OtpDigits: View {
    let otp: String

    var body: some View {
        let half = otp.index(otp.startIndex, offsetBy: otp.count / 2)
        HStack(spacing: 10) {
            Text(otp[..<half])
            Text(otp[half...])
        }
        .font(.system(size: 42, weight: .semibold, design: .rounded).monospacedDigit())
        .foregroundStyle(Theme.ink)
        .lineLimit(1)
        .minimumScaleFactor(0.6)
        .privacySensitive()
    }
}

/// What is left of the time step: a ring that empties smoothly, the seconds
/// inside it, both in the danger colour for the last five seconds.
private struct OtpCountdown: View {
    let expiresAt: Date
    let period: TimeInterval
    let seconds: Int

    var body: some View {
        let tint = seconds <= 5 ? Theme.danger : Theme.accent
        TimelineView(.animation(minimumInterval: 1.0 / 30)) { context in
            let left = max(0, min(1, expiresAt.timeIntervalSince(context.date) / period))
            ZStack {
                Circle().stroke(Theme.line, lineWidth: 3)
                Circle()
                    .trim(from: 0, to: left)
                    .stroke(tint, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                Text("\(seconds)")
                    .font(.footnote.weight(.semibold).monospacedDigit())
                    .foregroundStyle(seconds <= 5 ? Theme.danger : Theme.muted)
            }
        }
        .frame(width: 40, height: 40)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(t("settings.otp.expiresIn", ["seconds": seconds]))
    }
}

func copySensitiveOtp(_ text: String, expiresAt: Date) {
    UIPasteboard.general.setItems(
        [[UTType.utf8PlainText.identifier: text]],
        options: [.localOnly: true, .expirationDate: expiresAt])
}
