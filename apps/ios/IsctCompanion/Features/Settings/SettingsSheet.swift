import IsctCore
import SwiftUI

/// 設定 (sheet from Today's gear): school accounts and reminders. Planning,
/// calendars, folders and developer settings stay on desktop.
struct SettingsSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("学校アカウント") {
                    NavigationLink {
                        AccountScreen(profile: .science, name: "Science Tokyo")
                    } label: {
                        accountRow("Science Tokyo", model.credentials?.science, model.sessions?.science)
                    }
                    NavigationLink {
                        AccountScreen(profile: .tokyoTech, name: "Tokyo Tech（旧）")
                    } label: {
                        accountRow("Tokyo Tech（旧）", model.credentials?.tokyoTech, model.sessions?.tokyoTech)
                    }
                }
                Section {
                    NavigationLink {
                        NotificationsScreen()
                    } label: {
                        Label {
                            VStack(alignment: .leading, spacing: 2) {
                                Text("通知")
                                Text(notificationSummary).font(.subheadline).foregroundStyle(Theme.muted)
                            }
                        } icon: {
                            Image(systemName: "bell")
                        }
                    }
                } footer: {
                    Text("学年暦の編集、コースフォルダ、データ更新、開発者向け設定はデスクトップ版にあります。")
                }
                Section {
                    LabeledContent("IsctCompanion", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")
                }
            }
            .navigationTitle("設定")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button { dismiss() } label: { Image(systemName: "xmark") }
                        .accessibilityLabel("閉じる")
                }
            }
        }
    }

    private var notificationSummary: String {
        let settings = model.records.settings["notifications"] as? [String: Any] ?? [:]
        var parts: [String] = []
        if settings["classes"] as? Bool == true {
            parts.append("授業 \((settings["classLeadMinutes"] as? NSNumber)?.intValue ?? 10) 分前")
        }
        if settings["assignments"] as? Bool == true { parts.append("課題 当日") }
        return parts.isEmpty ? "オフ" : parts.joined(separator: " · ")
    }

    private func accountRow(_ name: String, _ credential: CredentialSummary?, _ session: SchoolSessionState?) -> some View {
        let connected = credential?.availability == .available || credential?.availability == .savedLocked
        let state: (String, Color) = {
            guard connected else { return ("未接続", Theme.muted) }
            switch session {
            case .active?:
                if let last = model.lastPull { return ("接続済 · \(Format.time(last)) 更新", Theme.ok) }
                return ("接続済", Theme.ok)
            case .failed?: return ("ログインに失敗しました", Theme.danger)
            case .maintenance?: return ("メンテナンス中", Theme.warn)
            default: return ("確認中", Theme.muted)
            }
        }()
        return HStack(spacing: 12) {
            Circle().fill(connected ? state.1 : Theme.line).frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 2) {
                Text(name)
                Text(state.0).font(.subheadline).foregroundStyle(Theme.muted)
            }
        }
    }
}

/// Sign in, change or forget a school account. The password goes straight to
/// Core, which stores it in the Keychain and starts the session check.
private struct AccountScreen: View {
    let profile: SchoolAuthProfile
    let name: String
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var username = ""
    @State private var password = ""
    @State private var otpSecret = ""
    @State private var matrix = ""
    @State private var saving = false
    @State private var error: String?

    private var summary: CredentialSummary? {
        profile == .science ? model.credentials?.science : model.credentials?.tokyoTech
    }

    var body: some View {
        Form {
            Section {
                TextField("学籍番号", text: $username)
                    .textContentType(.username)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField(summary?.saved == true ? "パスワード（変更する場合のみ）" : "パスワード", text: $password)
                    .textContentType(.password)
            } footer: {
                Text("パスワードはこの端末のキーチェーンに保存され、iCloud キーチェーンでデスクトップ版と共有されます。")
            }
            if profile == .science {
                Section("ワンタイムパスワード") {
                    SecureField(summary?.hasOtp == true ? "設定済み（変更する場合のみ）" : "TOTP シークレット", text: $otpSecret)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
            } else {
                Section("マトリクス表") {
                    SecureField(summary?.hasMatrix == true ? "設定済み（変更する場合のみ）" : "70 文字", text: $matrix)
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                }
            }
            if let error {
                Section { Text(error).foregroundStyle(Theme.danger) }
            }
            Section {
                Button {
                    Task { await save() }
                } label: {
                    if saving { ProgressView() } else { Text("保存して接続") }
                }
                .disabled(saving || username.isEmpty || (password.isEmpty && summary?.saved != true))
            }
            if summary?.saved == true {
                Section {
                    Button("この端末からログイン情報を削除", role: .destructive) {
                        Task {
                            await model.deleteCredentials(profile)
                            dismiss()
                        }
                    }
                }
            }
        }
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { username = summary?.username ?? "" }
    }

    private func save() async {
        saving = true
        defer { saving = false }
        error = nil
        do {
            try await model.saveCredentials(
                profile,
                SchoolCredentials(username: username, password: password, otpSecret: otpSecret, matrix: matrix))
            dismiss()
        } catch {
            self.error = AppModel.describe(error)
        }
    }
}

/// Reminder switches; the system permission is asked on the first switch-on.
private struct NotificationsScreen: View {
    @Environment(AppModel.self) private var model
    @State private var denied = false

    private var settings: [String: Any] {
        model.records.settings["notifications"] as? [String: Any] ?? [:]
    }

    var body: some View {
        Form {
            Section {
                Toggle("授業の前に通知", isOn: binding("classes"))
                if settings["classes"] as? Bool == true {
                    Picker("通知のタイミング", selection: leadBinding) {
                        ForEach([5, 10, 15, 30], id: \.self) { Text("\($0) 分前").tag($0) }
                    }
                }
                Toggle("課題の締切当日に通知", isOn: binding("assignments"))
            } footer: {
                if denied {
                    Text("通知が許可されていません。設定アプリの「通知」から許可してください。")
                        .foregroundStyle(Theme.danger)
                }
            }
        }
        .navigationTitle("通知")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func binding(_ key: String) -> Binding<Bool> {
        Binding(
            get: { settings[key] as? Bool ?? false },
            set: { on in
                Task {
                    var permission = "default"
                    if on {
                        let granted = await NotificationScheduler.requestPermission()
                        denied = !granted
                        permission = granted ? "granted" : "denied"
                        guard granted else { return }
                    }
                    await model.setNotifications {
                        $0[key] = on
                        if on { $0["localPermission"] = permission }
                    }
                }
            })
    }

    private var leadBinding: Binding<Int> {
        Binding(
            get: { (settings["classLeadMinutes"] as? NSNumber)?.intValue ?? 10 },
            set: { minutes in Task { await model.setNotifications { $0["classLeadMinutes"] = minutes } } })
    }
}
