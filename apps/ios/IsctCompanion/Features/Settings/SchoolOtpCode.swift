import IsctCore
import SwiftUI
import UniformTypeIdentifiers

struct SchoolOtpCodeButton: View {
    @State private var opened = false

    var body: some View {
        Button(t("settings.otp.showCode")) { opened = true }
            .sheet(isPresented: $opened) {
                SchoolOtpCodeSheet()
                    .presentationDetents([.medium, .large])
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
            VStack(spacing: 12) {
                if scenePhase == .active, let current, seconds > 0 {
                    Text(current.otp)
                        .font(.largeTitle.weight(.semibold).monospacedDigit())
                        .tracking(4)
                        .privacySensitive()
                    Text(t("settings.otp.expiresIn", ["seconds": seconds]))
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(.secondary)
                    Button(t(copied ? "settings.credentialImport.copied" : "settings.otp.copyCode"), systemImage: "doc.on.doc") {
                        copy(current)
                    }
                    .buttonStyle(.borderedProminent)
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

func copySensitiveOtp(_ text: String, expiresAt: Date) {
    UIPasteboard.general.setItems(
        [[UTType.utf8PlainText.identifier: text]],
        options: [.localOnly: true, .expirationDate: expiresAt])
}
