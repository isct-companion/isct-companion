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
    @Environment(\.openURL) private var openURL
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @AppStorage(SchoolBrowserHost.textScaleKey) private var textScale = 115
    @State private var firstSite = WebRouter.firstSite
    @State private var logFile: ExportFile?
    @State private var notice: String?

    var body: some View {
        List {
            Group {
                Section(t("settings.main.schoolAccounts")) {
                    NavigationLink(value: SettingsRoute.account(.science)) {
                        accountRow(.science, "Science Tokyo", model.syncSource("lms")?.lastUpdated)
                    }
                    NavigationLink(value: SettingsRoute.account(.tokyoTech)) {
                        accountRow(.tokyoTech, t("settings.main.tokyoTechFormer"), model.syncSource("kyomu")?.lastUpdated)
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
                MapSection()
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
                    updateRows
                    Button {
                        Guide.showAgain()
                    } label: {
                        Label { Text(t("settings.main.showWelcomeAndTipsAgain")).foregroundStyle(Theme.ink) } icon: {
                            SettingsIcon(symbol: "lightbulb", tint: Theme.muted)
                        }
                    }
                    if let feedback = URL(string: feedbackFormUrl(version: Self.version, device: Self.device)) {
                        Link(destination: feedback) {
                            Label { Text(t("about.feedback")) } icon: {
                                SettingsIcon(symbol: "bubble.left.and.text.bubble.right", tint: Theme.muted)
                            }
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
                        .padding(.vertical, Theme.space3)
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
            .themedRows()
        }
        .themedList()
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

    /// The update check: what it found (opening the App Store), or a check
    /// now, and whether this device checks on its own (Core reads
    /// `appUpdateChecks`; absent is on).
    @ViewBuilder private var updateRows: some View {
        if let release = model.availableUpdate {
            HStack {
                Label {
                    Text(t("appUpdate.prompt.available", ["version": release.version])).foregroundStyle(Theme.ink)
                } icon: {
                    SettingsIcon(symbol: "arrow.down.app", tint: Theme.accent)
                }
                Spacer()
                if let button = OpenReleaseButton(release: release, openURL: openURL) {
                    button
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                }
            }
        } else {
            Button {
                Task { await model.checkForUpdate(manual: true) }
            } label: {
                Label {
                    Text(model.upToDate ? t("appUpdate.upToDate") : t("settings.about.checkForUpdates"))
                        .foregroundStyle(Theme.ink)
                } icon: {
                    SettingsIcon(symbol: "arrow.down.app", tint: Theme.muted)
                }
            }
            .disabled(model.checkingUpdate)
        }
        Toggle(isOn: Binding(
            get: { model.records.settings["appUpdateChecks"] as? Bool ?? true },
            set: { on in Task { await model.updateSettings { $0["appUpdateChecks"] = on } } }
        )) {
            Label { Text(t("appUpdate.checkAutomatically")) } icon: {
                SettingsIcon(symbol: "arrow.triangle.2.circlepath", tint: Theme.muted)
            }
        }
    }

    private static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    private static var today: String {
        Date().formatted(.iso8601.year().month().day())
    }

    /// The system this build runs on, as the log and the feedback form name
    /// it. `systemName` tells iOS from iPadOS, which share this binary.
    private static var device: String {
        let device = UIDevice.current
        return "\(device.systemName) \(device.systemVersion) · \(device.model)"
    }

    /// Core formats and redacts the log; the shell only names its build.
    private static func logText() throws -> String {
        try exportApplicationLogs(about: "IsctCompanion \(version) · \(device)")
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
            VStack(alignment: .leading, spacing: Theme.space1) {
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

    /// An account with its session as Core reports it, and a paused site.
    private func accountRow(_ profile: SchoolAuthProfile, _ name: String, _ lastRun: String?) -> some View {
        let session = model.profileSession(profile)
        let state = session?.state ?? .notConfigured
        let text: String =
            if session?.signOutPending == true {
                t("error.sign-out-incomplete")
            } else if state == .normal, let lastRun {
                t("settings.main.connectedUpdatedTime", ["time": Format.due(lastRun)])
            } else if let session {
                SchoolSessionText.line(session)
            } else {
                SchoolSessionText.state(state)
            }
        return HStack(spacing: Theme.rowGap) {
            Circle().fill(Self.tint(state)).frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: Theme.space1) {
                Text(name)
                Text(text).font(.subheadline).foregroundStyle(Theme.muted)
                if let maintenance = session?.maintenance {
                    Text(SchoolSessionText.maintenance(maintenance, profile: profile))
                        .font(.footnote)
                        .foregroundStyle(Theme.warn)
                }
            }
        }
    }

    private static func tint(_ state: SchoolSessionState) -> Color {
        switch state {
        case .notConfigured: Theme.line
        case .signingIn: Theme.muted
        case .normal: Theme.ok
        case .needsWeb, .unavailable: Theme.warn
        case .refused: Theme.danger
        }
    }
}

/// The row icon: the symbol in a token role on a soft square of the same
/// role, never a system rainbow colour: accent for preferences, ok for data,
/// warn and danger for caution, muted for utilities. Solid squares in these
/// deep roles, half of them grey, made the list look heavy.
private struct SettingsIcon: View {
    let symbol: String
    let tint: Color

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 15, weight: .medium))
            .symbolVariant(.fill)
            .foregroundStyle(tint)
            .frame(width: 29, height: 29)
            .background(tint.opacity(0.14), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
    }
}

/// Sign in, change or forget a school account. The password goes straight to
/// Core, which signs in with it and stores it in the Keychain only once the
/// school accepted it. While the account is stuck, the page says why and
/// offers Core's Retry.
private struct AccountScreen: View {
    let profile: SchoolAuthProfile
    let name: String
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss
    @State private var username = ""
    @State private var password = ""
    @State private var otpSecret = ""
    @State private var matrix = ""
    @State private var saving = false
    @State private var retrying = false
    @State private var removing = false
    @State private var error: String?
    /// What the last Connect did, shown until the next one.
    @State private var outcome: SchoolConnectOutcome?

    private var summary: CredentialSummary? {
        profile == .science ? model.credentials?.science : model.credentials?.tokyoTech
    }

    private var session: SchoolProfileSession? { model.profileSession(profile) }

    /// The states that wait for the person or for Core's next try, where
    /// Retry makes one attempt now with the synced credentials.
    private static let stuck: [SchoolSessionState] = [.refused, .needsWeb, .unavailable]

    var body: some View {
        Form {
            Group {
                Section {
                    TextField(t("settings.schoolAccounts.id"), text: $username)
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
                    // Its own sections: the OTP state, and the import paths
                    // while no setup is saved or read.
                    OtpImportField(
                        saved: summary?.hasOtp == true,
                        verifiedAccount: summary?.saved == true,
                        canShowCode: session?.signOutPending != true,
                        setup: $otpSecret,
                        openSetup: {
                            web.showOtpSetup(username: username)
                            navigator.tab = .web
                        }
                    )
                } else {
                    MatrixImportSection(saved: summary?.hasMatrix == true, matrix: $matrix)
                }
                if let error {
                    Section { Text(error).foregroundStyle(Theme.danger) }
                } else if let outcome {
                    connectOutcome(outcome)
                } else if session?.signOutPending == true {
                    Text(t("error.sign-out-incomplete"))
                } else if !saving, let session, Self.stuck.contains(session.state) {
                    status(session)
                }
                if let maintenance = session?.maintenance {
                    Section { Text(SchoolSessionText.maintenance(maintenance, profile: profile)).foregroundStyle(Theme.warn) }
                }
                Section {
                    Button {
                        Task { await save() }
                    } label: {
                        if saving { ProgressView() } else { Text(t("settings.main.saveAndConnect")) }
                    }
                    .disabled(saving || session?.signOutPending == true || username.isEmpty || (password.isEmpty && summary?.saved != true))
                }
                if summary?.saved == true || session?.signOutPending == true {
                    Section {
                        // Core first stops the account's reads, so this can take a moment.
                        Button(role: .destructive) {
                            removing = true
                            error = nil
                            Task {
                                do {
                                    try await model.deleteCredentials(profile)
                                    dismiss()
                                } catch {
                                    // Signing out did not finish: the account
                                    // stays saved, and this button with it.
                                    self.error = AppModel.describe(error)
                                }
                                removing = false
                            }
                        } label: {
                            if removing { ProgressView() } else { Text(t("settings.main.removeSignInDetailsFrom")) }
                        }
                        .disabled(removing)
                    }
                }
            }
            .themedRows()
        }
        .themedList()
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { if username.isEmpty { username = summary?.username ?? "" } }
        .onChange(of: summary?.username) { _, owner in
            if username.isEmpty { username = owner ?? "" }
        }
    }

    /// What Connect did. Only `.connected` saved the entries; after anything
    /// else they stay in the form to be fixed and sent again.
    private func connectOutcome(_ outcome: SchoolConnectOutcome) -> some View {
        Section {
            switch outcome {
            case .connected:
                Text(t("schoolSession.connect.connected")).foregroundStyle(Theme.ok)
            case let .refused(reason, _):
                Text(SchoolSessionText.reason(reason) ?? SchoolSessionText.state(.refused)).foregroundStyle(Theme.danger)
                Text(t("schoolSession.connect.notSaved")).font(.footnote).foregroundStyle(Theme.muted)
            case let .needsWeb(reason, _):
                Text(SchoolSessionText.reason(reason) ?? SchoolSessionText.state(.needsWeb)).foregroundStyle(Theme.warn)
                Text(t("schoolSession.connect.notSaved")).font(.footnote).foregroundStyle(Theme.muted)
            case .unreachable:
                Text(t("schoolSession.connect.unreachable")).foregroundStyle(Theme.warn)
            case .notSaved:
                Text(t("schoolSession.connect.saveFailed")).foregroundStyle(Theme.danger)
            case .stopped:
                Text(t("schoolSession.connect.stopped")).foregroundStyle(Theme.warn)
            }
        }
    }

    /// Why the account is stuck, then what to do. Without school OTP that is
    /// setting it up on the portal (Retry would only get the emailed code
    /// again); other needs-web says the person signs in on the school page;
    /// refused says when Retry helps.
    private func status(_ session: SchoolProfileSession) -> some View {
        Section {
            Text(SchoolSessionText.line(session))
                .foregroundStyle(session.state == .refused ? Theme.danger : Theme.warn)
            if SchoolSessionText.otpNotSetUp(session.state, problem: session.problem) {
                Text(t("schoolSession.problem.emailOtp")).font(.footnote).foregroundStyle(Theme.muted)
                Button(t("schoolSession.openPortal"), action: openPortal)
            } else {
                if session.state == .needsWeb {
                    Text(t("schoolSession.needsWeb.signInYourself")).font(.footnote).foregroundStyle(Theme.muted)
                }
                Button {
                    Task { await retry() }
                } label: {
                    if retrying { ProgressView() } else { Text(t("common.retry")) }
                }
                .disabled(retrying)
            }
        } footer: {
            if session.state == .refused { Text(t("schoolSession.retry.otherDevice")) }
        }
    }

    /// The portal's start page as it is, where the person signs in and sets
    /// up school OTP by hand.
    private func openPortal() {
        web.showUnsigned(.sciencePortal)
        navigator.tab = .web
    }

    private func retry() async {
        retrying = true
        defer { retrying = false }
        do {
            try await model.retrySchool(profile)
        } catch {
            self.error = AppModel.describe(error)
        }
    }

    private func save() async {
        saving = true
        defer { saving = false }
        error = nil
        outcome = nil
        do {
            outcome = try await model.connect(
                profile,
                SchoolCredentials(username: username, password: password, otpSecret: otpSecret, matrix: matrix))
            // The secrets are saved now; the fields go back to "enter only to change".
            if case .connected = outcome {
                password = ""
                otpSecret = ""
                matrix = ""
            }
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
                Spacer(minLength: Theme.space6)
                HStack(spacing: Theme.space5) {
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
                .overlay { Circle().fill(set?.surface ?? Theme.surface).padding(Theme.space4) }
                .overlay { Circle().strokeBorder(selected ? Theme.accent : Theme.line, lineWidth: selected ? 2 : 1) }
                .frame(width: 28, height: 28)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(t(Self.names[id] ?? id))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Settings → Map: which maps app the campus map's "Open in Maps" links
/// open in. Core builds the link from this device setting.
private struct MapSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Section(t("settings.main.map")) {
            Picker(selection: Binding(
                get: { model.records.settings.text("mapsApp") ?? "apple" },
                set: { value in Task { await model.updateSettings { $0["mapsApp"] = value } } }
            )) {
                Text(t("settings.main.appleMaps")).tag("apple")
                Text(t("settings.main.googleMaps")).tag("google")
            } label: {
                Label { Text(t("settings.main.openPlacesIn")) } icon: { SettingsIcon(symbol: "map", tint: Theme.ok) }
            }
        }
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
