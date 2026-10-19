import IsctCore
import SwiftUI

/// Settings pages, pushed onto whichever page's stack opened them, as the
/// desktop's settings sections are pages of their own. A group with more than
/// two controls gets a page; smaller ones stay on the root list.
enum SettingsRoute: Hashable {
    case root
    case account(SchoolAuthProfile)
    case notifications
    case assignmentLeads
    case studyProfile
    case dataUpdates
}

struct SettingsDestination: View {
    let route: SettingsRoute

    var body: some View {
        switch route {
        case .root:
            SettingsScreen()
        case let .account(profile):
            AccountScreen(profile: profile, name: profile == .science ? "Science Tokyo" : "Tokyo Tech（旧）")
        case .notifications:
            NotificationsScreen()
        case .assignmentLeads:
            AssignmentLeadsScreen()
        case .studyProfile:
            StudyProfileScreen()
        case .dataUpdates:
            DataUpdatesScreen()
        }
    }
}

/// 設定: the desktop settings sections that apply on a phone — school
/// accounts, reminders, study profile, data updates and the ウェブ tab.
/// Planning, calendars, folders and developer settings stay on desktop.
struct SettingsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @AppStorage(SchoolBrowserHost.textScaleKey) private var textScale = 115
    @State private var firstSite = WebRouter.firstSite

    var body: some View {
        List {
            Section("学校アカウント") {
                NavigationLink(value: SettingsRoute.account(.science)) {
                    accountRow("Science Tokyo", model.credentials?.science, model.sessions?.science, model.source(.scienceLms)?.lastUpdated)
                }
                NavigationLink(value: SettingsRoute.account(.tokyoTech)) {
                    accountRow("Tokyo Tech（旧）", model.credentials?.tokyoTech, model.sessions?.tokyoTech, model.source(.kyomu)?.lastUpdated)
                }
            }
            Section {
                NavigationLink(value: SettingsRoute.notifications) {
                    row("通知", notificationSummary, icon: "bell")
                }
                NavigationLink(value: SettingsRoute.studyProfile) {
                    row("学習プロフィール", nil, icon: "graduationcap")
                }
                NavigationLink(value: SettingsRoute.dataUpdates) {
                    row("データ更新", dataSummary, icon: "arrow.triangle.2.circlepath")
                }
            }
            Section("ウェブ") {
                Picker("文字サイズ", selection: $textScale) {
                    ForEach(SchoolBrowserHost.textScales, id: \.self) { Text("\($0)%").tag($0) }
                }
                Picker("最初のサイト", selection: $firstSite) {
                    ForEach(WebRouter.tabSites, id: \.self) { Text(WebRouter.name($0)).tag($0) }
                }
            }
            Section {
                LabeledContent("IsctCompanion", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")
            } header: {
                Text("情報")
            } footer: {
                Text("学年暦の編集、コースフォルダ、開発者向け設定はデスクトップ版にあります。")
            }
        }
        .navigationTitle("設定")
        .onChange(of: textScale) { _, percent in web.browser.setTextScale(percent) }
        .onChange(of: firstSite) { _, site in WebRouter.firstSite = site }
    }

    private func row(_ title: String, _ detail: String?, icon: String) -> some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let detail { Text(detail).font(.subheadline).foregroundStyle(Theme.muted) }
            }
        } icon: {
            Image(systemName: icon)
        }
    }

    private var notificationSummary: String {
        let settings = model.records.settings["notifications"] as? [String: Any] ?? [:]
        var parts: [String] = []
        if settings["classes"] as? Bool == true {
            parts.append("授業 \(ReminderLeads.label(ReminderLeads.classLead(settings)))")
        }
        if ReminderLeads.exams(settings) {
            parts.append("試験 \(ReminderLeads.label(ReminderLeads.examLead(settings)))")
        }
        if settings["assignments"] as? Bool == true {
            let leads = ReminderLeads.assignmentLeads(settings)
            parts.append("課題 \(leads.count) 回")
        }
        return parts.isEmpty ? "オフ" : parts.joined(separator: " · ")
    }

    private var dataSummary: String {
        let refresh = model.records.settings["subscriptionRefresh"] as? [String: Any] ?? [:]
        return refresh["allowCellular"] as? Bool == true ? "モバイル通信でも更新" : "Wi-Fi のときだけ自動更新"
    }

    private func accountRow(
        _ name: String, _ credential: CredentialSummary?, _ session: SchoolSessionState?, _ lastRun: String?
    ) -> some View {
        let connected = credential?.availability == .available || credential?.availability == .savedLocked
        let state: (String, Color) = {
            guard connected else { return ("未接続", Theme.muted) }
            switch session {
            case .active?:
                if let lastRun { return ("接続済 · \(Format.due(lastRun)) 更新", Theme.ok) }
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
                    .keyboardType(.asciiCapable)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField(summary?.saved == true ? "パスワード（変更する場合のみ）" : "パスワード", text: $password)
                    .textContentType(.password)
            } footer: {
                Text("パスワードはこの端末のキーチェーンに保存され、iCloud キーチェーンでデスクトップ版と共有されます。")
            }
            // Pasted keys, not passwords: a second secure field makes AutoFill
            // read the form as a sign-up and offer new credentials instead of
            // the saved login. They stay out of screenshots and the app switcher.
            if profile == .science {
                Section("ワンタイムパスワード") {
                    OtpImportField(saved: summary?.hasOtp == true, setup: $otpSecret)
                }
            } else {
                MatrixImportSection(saved: summary?.hasMatrix == true, matrix: $matrix)
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
