import IsctCore
import SwiftUI
import TipKit
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
                        Label { Text(t("web.pageZoom")) } icon: { SettingsIcon(symbol: "textformat.size", tint: Theme.muted) }
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
                        .popoverTip(FeedbackTip(), arrowEdge: .bottom)
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
        .appNoticeFlow(logFile != nil)
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
        AppClock.now.formatted(.iso8601.year().month().day())
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
            model.notices.show(t("settings.main.exportFailedReason", ["reason": AppModel.describe(error)]))
        }
    }

    private func copyLogs() {
        do {
            UIPasteboard.general.string = try Self.logText()
            model.notices.show(t("settings.credentialImport.copied"))
        } catch {
            model.notices.show(t("settings.otp.copyFailed"))
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
        let icloud = model.backup.state?.providers.first { $0.provider == .icloud }
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
            if state == .normal, session?.presentation.recovery.isEmpty == true, let lastRun {
                t("settings.main.connectedUpdatedTime", ["time": Format.due(lastRun)])
            } else if let session {
                SchoolSessionText.line(session)
            } else {
                t("common.notConnected")
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
struct SettingsIcon: View {
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
