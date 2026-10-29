import IsctCore
import SwiftUI
import UIKit

/// Settings pages, pushed onto whichever page's stack opened them, as the
/// desktop's settings sections are pages of their own. A group with more than
/// two controls gets a page; smaller ones stay on the root list.
enum SettingsRoute: Hashable {
    case account(SchoolAuthProfile)
    case notifications
    case assignmentLeads
    case studyProfile
    case dataUpdates
    case backup
    case reset(ResetKind)
}

struct SettingsDestination: View {
    let route: SettingsRoute

    var body: some View {
        switch route {
        case let .account(profile):
            AccountScreen(profile: profile, name: profile == .science ? "Science Tokyo" : t("settings.main.tokyoTechFormer"))
        case .notifications:
            NotificationsScreen()
        case .assignmentLeads:
            AssignmentLeadsScreen()
        case .studyProfile:
            StudyProfileScreen()
        case .dataUpdates:
            DataUpdatesScreen()
        case .backup:
            BackupScreen()
        case let .reset(kind):
            ResetScreen(kind: kind)
        }
    }
}

/// 設定: the desktop settings sections that apply on a phone — school
/// accounts, reminders, study profile, data updates, backup, language, the
/// ウェブ tab and reset.
/// Planning, calendars, folders and developer settings stay on desktop.
struct SettingsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @AppStorage(SchoolBrowserHost.textScaleKey) private var textScale = 115
    @State private var firstSite = WebRouter.firstSite
    @State private var logFile: ExportFile?
    @State private var notice: String?

    var body: some View {
        List {
            Section(t("settings.main.schoolAccounts")) {
                NavigationLink(value: SettingsRoute.account(.science)) {
                    accountRow("Science Tokyo", model.credentials?.science, model.sessions?.science, model.syncSource("lms")?.lastUpdated)
                }
                NavigationLink(value: SettingsRoute.account(.tokyoTech)) {
                    accountRow(t("settings.main.tokyoTechFormer"), model.credentials?.tokyoTech, model.sessions?.tokyoTech, model.syncSource("kyomu")?.lastUpdated)
                }
            }
            Section {
                NavigationLink(value: SettingsRoute.notifications) {
                    row(t("common.notifications"), notificationSummary, icon: "bell.fill", tint: Theme.danger)
                }
                NavigationLink(value: SettingsRoute.studyProfile) {
                    row(t("settings.main.studyProfile"), nil, icon: "graduationcap.fill", tint: Theme.accent)
                }
                NavigationLink(value: SettingsRoute.dataUpdates) {
                    row(t("settings.main.dataUpdates"), dataSummary, icon: "arrow.triangle.2.circlepath", tint: Theme.ok)
                }
                NavigationLink(value: SettingsRoute.backup) {
                    row(t("settings.main.backup"), backupSummary, icon: "icloud.fill", tint: Theme.accent)
                }
            }
            ThemeSection()
            LanguageSection()
            Section(t("common.web")) {
                Picker(selection: $textScale) {
                    ForEach(SchoolBrowserHost.textScales, id: \.self) { Text("\($0)%").tag($0) }
                } label: {
                    Label { Text(t("common.textSize")) } icon: { SettingsIcon(symbol: "textformat.size", tint: Theme.muted) }
                }
                Picker(selection: $firstSite) {
                    ForEach(WebRouter.tabSites, id: \.self) { Text(WebRouter.name($0)).tag($0) }
                } label: {
                    Label { Text(t("settings.main.firstSite")) } icon: { SettingsIcon(symbol: "globe", tint: Theme.accent) }
                }
            }
            Section {
                LabeledContent {
                    Text(Self.version)
                } label: {
                    Label { Text("IsctCompanion") } icon: { SettingsIcon(symbol: "info", tint: Theme.muted) }
                }
                Button {
                    Guide.showAgain()
                } label: {
                    Label { Text(t("settings.main.showWelcomeAndTipsAgain")).foregroundStyle(Theme.ink) } icon: {
                        SettingsIcon(symbol: "lightbulb", tint: Theme.muted)
                    }
                }
                if let policy = URL(string: "https://github.com/isct-companion/isct-companion/blob/main/docs/legal/PRIVACY.md") {
                    Link(destination: policy) {
                        Label { Text(t("settings.main.privacyPolicy")) } icon: {
                            SettingsIcon(symbol: "hand.raised", tint: Theme.muted)
                        }
                    }
                }
                // List gives a Menu no row treatment: its hit area and menu
                // preview are the label's own frame. So the label fills the
                // row, padded into the row insets (taken back outside, keeping
                // the row's height) so the preview holds the whole icon.
                Menu {
                    Button(t("settings.main.saveToFile"), systemImage: "square.and.arrow.down") { saveLogs() }
                    Button(t("settings.schoolAccounts.copy"), systemImage: "doc.on.doc") { copyLogs() }
                } label: {
                    Label { Text(t("settings.main.applicationLogs")).foregroundStyle(Theme.ink) } icon: {
                        SettingsIcon(symbol: "doc.text", tint: Theme.muted)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 6)
                    .contentShape(.rect)
                    .contentShape(.contextMenuPreview, .rect(cornerRadius: 10))
                }
                .padding(.vertical, -6)
            } header: {
                Text(t("settings.main.about"))
            } footer: {
                Text(t("settings.main.calendarEditingCourseFoldersAnd"))
            }
            // Destructive rows last, as in the system Settings.
            Section(t("settings.reset.group")) {
                NavigationLink(value: SettingsRoute.reset(.schoolCaches)) {
                    row(t("settings.reset.caches.title"), t("settings.reset.caches.hint"), icon: "arrow.counterclockwise", tint: Theme.warn)
                }
                NavigationLink(value: SettingsRoute.reset(.everything)) {
                    row(t("settings.reset.full.title"), t("settings.reset.full.hint"), icon: "exclamationmark.arrow.triangle.2.circlepath", tint: Theme.danger)
                        .foregroundStyle(Theme.danger)
                }
            }
        }
        .navigationTitle(t("common.settings"))
        .onChange(of: textScale) { _, percent in web.browser.setTextScale(percent) }
        .onChange(of: firstSite) { _, site in WebRouter.firstSite = site }
        .fileExporter(
            isPresented: Binding(get: { logFile != nil }, set: { if !$0 { logFile = nil } }),
            document: logFile, contentType: .plainText,
            defaultFilename: "IsctCompanion-logs-\(Self.today).txt"
        ) { _ in logFile = nil }
        .alert(notice ?? "", isPresented: Binding(get: { notice != nil }, set: { if !$0 { notice = nil } })) {
            Button("OK", role: .cancel) {}
        }
    }

    private static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    private static var today: String {
        Date().formatted(.iso8601.year().month().day())
    }

    /// Core formats and redacts the log; the shell only names its build.
    /// `systemName` tells iOS from iPadOS, which share this binary.
    private static func logText() throws -> String {
        let device = UIDevice.current
        return try exportApplicationLogs(about: "IsctCompanion \(version) · \(device.systemName) \(device.systemVersion) · \(device.model)")
    }

    private func saveLogs() {
        do {
            logFile = ExportFile(data: Data(try Self.logText().utf8))
        } catch {
            notice = t("settings.main.exportFailedReason", ["reason": AppModel.describe(error)])
        }
    }

    private func copyLogs() {
        do {
            UIPasteboard.general.string = try Self.logText()
            notice = t("settings.credentialImport.copied")
        } catch {
            notice = t("settings.otp.copyFailed")
        }
    }

    private func row(_ title: String, _ detail: String?, icon: String, tint: Color) -> some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let detail { Text(detail).font(.subheadline).foregroundStyle(Theme.muted) }
            }
        } icon: {
            SettingsIcon(symbol: icon, tint: tint)
        }
    }

    private var notificationSummary: String {
        let settings = model.records.settings["notifications"] as? [String: Any] ?? [:]
        var parts: [String] = []
        if settings["classes"] as? Bool == true {
            parts.append(t("settings.main.classesLead", ["lead": ReminderLeads.label(ReminderLeads.classLead(settings))]))
        }
        if ReminderLeads.exams(settings) {
            parts.append(t("settings.main.examsLead", ["lead": ReminderLeads.label(ReminderLeads.examLead(settings))]))
        }
        if settings["assignments"] as? Bool == true {
            let leads = ReminderLeads.assignmentLeads(settings)
            parts.append(t("settings.main.assignmentsCount", ["count": leads.count]))
        }
        return parts.isEmpty ? t("settings.main.off") : parts.joined(separator: " · ")
    }

    private var backupSummary: String {
        let icloud = model.cloud?.providers.first { $0.provider == .icloud }
        guard icloud?.enabled == true else { return t("settings.main.off") }
        return icloud?.lastSyncedAt.map { t("settings.backup.syncedDate", ["date": Format.due($0)]) } ?? "iCloud"
    }

    private var dataSummary: String {
        let refresh = model.records.settings["subscriptionRefresh"] as? [String: Any] ?? [:]
        return refresh["allowCellular"] as? Bool == true ? t("settings.main.updateOnCellularData") : t("settings.main.updatesAutomaticallyOnWiFi")
    }

    private func accountRow(
        _ name: String, _ credential: CredentialSummary?, _ session: SchoolProfileSession?, _ lastRun: String?
    ) -> some View {
        let connected = credential?.availability == .available || credential?.availability == .savedLocked
        let state: (String, Color) = {
            guard connected else { return (t("common.notConnected"), Theme.muted) }
            if session?.state == .failed, let problem = session?.problem {
                return (t("settings.main.signInFailedReason", ["reason": AppModel.reason(problem).components(separatedBy: "\n")[0]]), Theme.danger)
            }
            switch session?.state {
            case .active?:
                if let lastRun { return (t("settings.main.connectedUpdatedTime", ["time": Format.due(lastRun)]), Theme.ok) }
                return (t("settings.main.connected"), Theme.ok)
            case .failed?: return (t("common.signInFailed"), Theme.danger)
            case .maintenance?: return (t("common.underMaintenance"), Theme.warn)
            default: return (t("settings.main.checking"), Theme.muted)
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

/// The system Settings' row icon: a white symbol on a rounded square in a
/// token role, never a system rainbow colour: accent for preferences, ok for
/// data, warn and danger for caution, muted for utilities.
private struct SettingsIcon: View {
    let symbol: String
    let tint: Color

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(Theme.onSolid)
            .frame(width: 29, height: 29)
            .background(tint.gradient, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
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

    /// Why the last sign-in failed, as Core recorded it, while it still applies.
    private var problem: String? {
        let session = profile == .science ? model.sessions?.science : model.sessions?.tokyoTech
        guard let session, session.state != .active, let problem = session.problem else { return nil }
        var text = AppModel.reason(problem)
        if session.auto == .paused {
            text += "\n" + t("settings.main.automaticSignInWaitsUntil")
        } else if session.auto == .retryLater {
            text += "\n" + t("settings.main.automaticSignInTriesAgain")
        }
        return text
    }

    var body: some View {
        Form {
            Section {
                TextField(t("settings.main.studentID"), text: $username)
                    .textContentType(.username)
                    .keyboardType(.asciiCapable)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField(summary?.saved == true ? t("settings.main.passwordEnterOnlyToChange") : t("settings.main.password"), text: $password)
                    .textContentType(.password)
            } footer: {
                Text(t("settings.main.thePasswordIsSavedIn"))
            }
            // Pasted keys, not passwords: a second secure field makes AutoFill
            // read the form as a sign-up and offer new credentials instead of
            // the saved login. They stay out of screenshots and the app switcher.
            if profile == .science {
                Section(t("settings.main.oneTimePassword")) {
                    OtpImportField(saved: summary?.hasOtp == true, setup: $otpSecret)
                }
            } else {
                MatrixImportSection(saved: summary?.hasMatrix == true, matrix: $matrix)
            }
            if let message = error ?? (saving ? nil : problem) {
                Section { Text(message).foregroundStyle(Theme.danger) }
            }
            Section {
                Button {
                    Task { await save() }
                } label: {
                    if saving { ProgressView() } else { Text(t("settings.main.saveAndConnect")) }
                }
                .disabled(saving || username.isEmpty || (password.isEmpty && summary?.saved != true))
            }
            if summary?.saved == true {
                Section {
                    Button(t("settings.main.removeSignInDetailsFrom"), role: .destructive) {
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
            let result = try await model.connect(
                profile,
                SchoolCredentials(username: username, password: password, otpSecret: otpSecret, matrix: matrix))
            // Stay on the page with the reason, so the person can correct it.
            if let result, result.state == .failed || result.state == .maintenance {
                error = result.problem.map(AppModel.reason) ?? t(result.state == .maintenance ? "common.underMaintenance" : "common.signInFailed")
                return
            }
            dismiss()
        } catch {
            self.error = AppModel.describe(error)
        }
    }
}

/// The desktop's language rows: the UI language and the language of school
/// text. Both are workspace settings, so they follow the person to desktop.
/// Settings → the background theme: five swatches, applied at once and kept
/// in the synced settings (slotledger `approach/neutral-themes`).
private struct ThemeSection: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var scheme

    private static let names = ["warm": "settings.main.warm", "cool": "settings.main.cool", "paper": "settings.main.paper", "mist": "settings.main.mist", "moss": "settings.main.moss"]

    var body: some View {
        let chosen = model.records.settings.text("theme") ?? "warm"
        Section {
            HStack(spacing: 0) {
                Label { Text(t("settings.main.theme")) } icon: { SettingsIcon(symbol: "paintpalette", tint: Theme.accent) }
                Spacer(minLength: 12)
                HStack(spacing: 10) {
                    ForEach(Theme.themes, id: \.self) { id in swatch(id, selected: id == chosen) }
                }
            }
            .sensoryFeedback(.selection, trigger: chosen)
        }
    }

    private func swatch(_ id: String, selected: Bool) -> some View {
        let set = IsctThemes.all[id].map { scheme == .dark ? $0.dark : $0.light }
        return Button {
            Task { await model.updateSettings { $0["theme"] = id } }
        } label: {
            Circle()
                .fill(set?.bg ?? Theme.bg)
                .overlay { Circle().fill(set?.surface ?? Theme.surface).padding(8) }
                .overlay { Circle().strokeBorder(selected ? Theme.accent : Theme.line, lineWidth: selected ? 2 : 1) }
                .frame(width: 28, height: 28)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(t(Self.names[id] ?? id))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

private struct LanguageSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Section {
            Picker(selection: binding("locale", default: "en")) {
                Text(verbatim: "English").tag("en")
                Text(verbatim: "日本語").tag("ja")
                Text(verbatim: "中文").tag("zh")
            } label: {
                Label { Text(t("common.language")) } icon: { SettingsIcon(symbol: "character.bubble", tint: Theme.accent) }
            }
            Picker(selection: binding("contentLanguage", default: "auto")) {
                Text(t("common.automatic")).tag("auto")
                Text(verbatim: "English").tag("en")
                Text(verbatim: "日本語").tag("ja")
            } label: {
                Label { Text(t("settings.main.schoolContentLanguage")) } icon: { SettingsIcon(symbol: "text.book.closed", tint: Theme.accent) }
            }
        } footer: {
            Text(t("settings.main.courseNamesRoomsAndSyllabus"))
        }
    }

    private func binding(_ key: String, default fallback: String) -> Binding<String> {
        Binding(
            get: { model.records.settings.text(key) ?? fallback },
            set: { value in Task { await model.updateSettings { $0[key] = value } } }
        )
    }
}
