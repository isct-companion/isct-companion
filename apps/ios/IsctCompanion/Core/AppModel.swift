import Foundation
import IsctCore
import Network
import Observation
import SwiftUI

/// The shell's one owner of the Core session. It renders Core read models,
/// forwards intents and reports lifecycle facts; every decision about what is
/// due, grouped or shown stays in Core.
@MainActor
@Observable
final class AppModel {
    enum Status: Equatable {
        case opening
        case ready
        case failed(String)
    }

    private(set) var status: Status = .opening
    private(set) var today: TodayView?
    private(set) var board: AssignmentBoard?
    private(set) var periods: [TeachingPeriod] = []
    /// Course id → Core's display colour, identical on every surface.
    private(set) var colors: [String: String] = [:]
    private(set) var records = WorkspaceRecords()
    private(set) var credentials: SchoolCredentialStatus?
    private(set) var sessions: SchoolSessionStates?
    /// What the school connection is doing, decided by Core (`school_status.rs`).
    private(set) var schoolStatus: SchoolStatus?
    /// Undo for the last hide, built from the intent Core committed.
    private(set) var undo: UndoAction?
    /// One "needs you" banner (sync feedback level 3).
    private(set) var problem: String?
    /// Cells read so far by a running Matrix scan, from Core's `ocr-progress`.
    private(set) var ocrProgress = 0
    /// Bumped whenever Core reports new content, so open screens re-query.
    private(set) var contentRevision = ""

    let browser = SchoolBrowserHost()
    let web: WebRouter
    private var session: Session?
    private var revision = ""
    private var boundaryTask: Task<Void, Never>?
    private var refreshTask: Task<Void, Never>?
    private var settingsWrite: Task<Void, Never>?
    private var wakeTask: Task<Void, Never>?
    private let network = NWPathMonitor()
    private var networkState: PullNetwork = .unknown
    private var foreground = true

    init() {
        web = WebRouter(browser: browser)
    }

    /// First launch: no school account configured yet.
    var needsAccount: Bool {
        guard let credentials else { return false }
        return credentials.science.availability == .notConfigured
            && credentials.tokyoTech.availability == .notConfigured
    }

    func source(_ site: SchoolSite) -> SchoolSourceStatus? {
        schoolStatus?.sources.first { $0.site == site }
    }

    // MARK: Lifecycle

    func start() async {
        guard session == nil else { return }
        do {
            let support = try FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            let data = support.appendingPathComponent("core", isDirectory: true)
            let files = support.appendingPathComponent("files", isDirectory: true)
            for directory in [data, files] {
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            }
            let config = SessionConfig(
                dataDirectory: data.path, filesDirectory: files.path, initialNetwork: .unknown,
                cloudProviders: [], authorizedCloudProviders: [])
            let session = try await Session.open(
                config: config, vault: KeychainVault(), browser: browser, cloud: nil,
                notifications: nil, downloads: WebCookieSession())
            self.session = session
            web.session = session
            let stream = try session.subscribe()
            Task { await self.listen(stream) }
            await refresh()
            await refreshAccounts()
            await refreshSchoolStatus()
            status = .ready
            watchNetwork()
            report(foreground: true)
            reportSchool(.launch)
            startWaking()
        } catch {
            status = .failed(Self.describe(error))
        }
    }

    func scenePhase(_ phase: ScenePhase) {
        guard let session else { return }
        switch phase {
        case .active:
            foreground = true
            _ = try? session.resume()
            report(foreground: true)
            reportSchool(.focus)
            startWaking()
            Task { await refresh() }
        case .background:
            foreground = false
            wakeTask?.cancel()
            wakeTask = nil
            report(foreground: false)
            reportSchool(.focus)
            _ = try? session.suspend()
        default:
            break
        }
    }

    private func report(foreground: Bool) {
        guard let session else { return }
        _ = try? session.lifecycle(
            foreground: foreground, network: networkState, schoolVisible: web.isVisible)
    }

    /// The periodic wakeup desktop gives Core every second (`core_ports.rs`):
    /// Core decides what is due — automatic school pulls, reference data such
    /// as the academic calendar, scheduled tasks. Only while in front.
    private func startWaking() {
        guard wakeTask == nil else { return }
        wakeTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, let session = self.session else { return }
                _ = try? await session.wake(schoolVisible: self.web.isVisible).value()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    /// School sign-in hears what desktop sends (`school-session.ts`): `launch`
    /// once when the session opens, then `focus` as the app enters or leaves the
    /// front. Core only acts on a visible focus.
    private func reportSchool(_ kind: SchoolSessionLifecycleKind) {
        _ = try? session?.schoolLifecycle(event: SchoolSessionLifecycle(kind: kind, visible: foreground))
    }

    private func watchNetwork() {
        network.pathUpdateHandler = { [weak self] path in
            let state: PullNetwork =
                path.status != .satisfied ? .offline
                : path.usesInterfaceType(.wifi) ? .wifi
                : path.usesInterfaceType(.cellular) ? .cellular : .unknown
            Task { @MainActor in
                guard let self, self.networkState != state else { return }
                self.networkState = state
                self.report(foreground: self.foreground)
            }
        }
        network.start(queue: .global(qos: .utility))
    }

    private func listen(_ stream: Subscription) async {
        while true {
            do {
                switch try await stream.next() {
                case let .event(event):
                    switch event.name {
                    case "school-status":
                        await refreshSchoolStatus()
                    case "pull-state-changed", "school-pull-progress":
                        // Folded into `school-status` by Core.
                        break
                    case "school-session", "school-login-status":
                        await refreshAccounts()
                    case "school-web-open":
                        web.handleOpenEvent(event.payload)
                    case "ocr-progress":
                        if let row = try? JSONSerialization.jsonObject(with: Data(event.payload.utf8)) as? [String: Any],
                            let completed = (row["completed"] as? NSNumber)?.intValue
                        {
                            ocrProgress = completed
                        }
                    case "school-web-url", "school-download":
                        break
                    default:
                        scheduleRefresh()
                    }
                case .snapshotRequired:
                    scheduleRefresh()
                    await refreshAccounts()
                    await refreshSchoolStatus()
                }
            } catch {
                return
            }
        }
    }

    /// Coalesces bursts of change hints into one read.
    private func scheduleRefresh() {
        refreshTask?.cancel()
        refreshTask = Task {
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }
            await refresh()
        }
    }

    // MARK: Read models

    func refresh() async {
        guard let session else { return }
        let now = Self.timestamp(Date())
        do {
            guard case let .workspace(snapshot) = try await session.snapshot().value() else { return }
            revision = snapshot.revision
            if snapshot.contentRevision != contentRevision {
                records = WorkspaceRecords(json: snapshot.workspace)
            }
            guard case let .schedule(result) = try await session.scheduleQuery(queries: [
                .today(now: now), .assignmentBoard(now: now), .teachingPeriods,
            ]).value() else { return }
            for value in result.values {
                switch value {
                case let .today(view): today = view
                case let .assignmentBoard(value): board = value
                case let .teachingPeriods(value): periods = value
                default: break
                }
            }
            // A fresh install has no current term yet, so the overview may fail alone.
            if case let .schedule(overview)? = try? await session.scheduleQuery(queries: [
                .workspaceOverview(term: "")
            ]).value(), case let .workspaceOverview(value)? = overview.values.first {
                colors = value.colors
            }
            let changed = contentRevision != snapshot.contentRevision
            contentRevision = snapshot.contentRevision
            scheduleBoundary()
            if changed { await NotificationScheduler.sync(session) }
        } catch is CancellationError {
            // A newer change hint replaced this read; it is not a failure.
        } catch SdkError.Cancelled {
        } catch {
            problem = Self.describe(error)
        }
    }

    /// Re-query when Core says the phase or hero changes next.
    private func scheduleBoundary() {
        boundaryTask?.cancel()
        guard let raw = today?.nextBoundary, let at = Self.date(raw) else { return }
        let delay = max(1, at.timeIntervalSinceNow + 1)
        boundaryTask = Task {
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled else { return }
            await refresh()
        }
    }

    func refreshAccounts() async {
        guard let session else { return }
        if case let .credentials(status)? = try? await session.credentialStatus().value() {
            credentials = status
        }
        if case let .schoolSession(snapshot)? = try? await session.schoolSession().value() {
            sessions = snapshot.states
        }
    }

    private func refreshSchoolStatus() async {
        guard let session else { return }
        if case let .schoolStatus(status)? = try? await session.schoolStatus().value() {
            schoolStatus = status
        }
    }

    func week(starting monday: Date) async -> ScheduleView? {
        guard let session, let term = today?.term else { return nil }
        let from = Self.civil(monday)
        let to = Self.civil(Calendar.tokyo.date(byAdding: .day, value: 4, to: monday) ?? monday)
        let query = WorkspaceQuery.scheduleView(term: term, from: from, to: to, includePersonal: false)
        guard case let .schedule(result)? = try? await session.scheduleQuery(queries: [query]).value(),
            case let .scheduleView(view)? = result.values.first
        else { return nil }
        return view
    }

    /// The quarter's courses as Core lists them for the course list.
    func overview(_ term: String) async -> WorkspaceOverview? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .workspaceOverview(term: term)
            ]).value(),
            case let .workspaceOverview(view)? = result.values.first
        else { return nil }
        return view
    }

    /// The academic year (April–March) a date belongs to.
    func academicYear(of date: Date) async -> Int32? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .academicYears(date: Self.civil(date), selectedYear: 0)
            ]).value(),
            case let .academicYears(index)? = result.values.first
        else { return nil }
        return index.currentYear
    }

    /// Every week of an academic year with its quarter, for the timetable picker.
    func timetableYear(_ year: Int32) async -> TimetableYear? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [.timetableYear(year: year)]).value(),
            case let .timetableYear(view)? = result.values.first
        else { return nil }
        return view
    }

    /// Downloads (or reuses) a course file through the signed-in LMS session.
    func openMaterial(_ files: CourseKey, source: String) async -> IsctCore.Material? {
        guard let session,
            case let .material(material)? = try? await session.openMaterial(
                context: files, source: source, download: true, readText: false, limit: nil
            ).value()
        else { return nil }
        return material
    }

    func courseDetail(_ id: String) async -> CourseDetail? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .courseDetail(courseId: id)
            ]).value(),
            case let .courseDetail(detail)? = result.values.first
        else { return nil }
        return detail
    }

    // MARK: Intents

    func setHidden(_ item: DeadlineItem, _ hidden: Bool) async {
        // Offered at once: the re-read that follows can take a moment.
        undo = hidden ? UndoAction(title: item.title, assignmentId: item.assignmentId) : nil
        await mark(item.assignmentId, hidden: hidden, pinned: nil)
    }

    func setPinned(_ item: DeadlineItem, _ pinned: Bool) async {
        await mark(item.assignmentId, hidden: nil, pinned: pinned)
    }

    func undoHide() async {
        guard let undo else { return }
        self.undo = nil
        await mark(undo.assignmentId, hidden: false, pinned: nil)
    }

    func dismissUndo() { undo = nil }
    func dismissProblem() { problem = nil }

    private func mark(_ id: String, hidden: Bool?, pinned: Bool?) async {
        guard let session else { return }
        let mutation = WorkspaceMutation(
            expectedRevision: revision,
            intent: .setAssignmentMark(assignmentId: id, hidden: hidden, pinned: pinned, completedAt: nil))
        do {
            if case let .workspace(snapshot) = try await session.mutate(mutation: mutation).value() {
                revision = snapshot.revision
            }
        } catch SdkError.Conflict {
            // Another device changed the workspace: re-read and let the person retry.
            await refresh()
        } catch {
            problem = Self.describe(error)
        }
        await refresh()
    }

    /// Pull to refresh, and the first read after an account is saved: 教務Web
    /// first (registrations are what puts courses on the timetable), then the
    /// current quarter's LMS courses, content and deadlines — the order of the
    /// desktop's "Refresh now". It runs on after the gesture ends; the header
    /// ring follows Core's school status.
    func refreshFromSchool() {
        var steps: [(Session) throws -> IsctCore.Operation] = []
        if [.available, .savedLocked].contains(credentials?.tokyoTech.availability) {
            steps.append { try $0.pullSchool(site: .kyomu, courseId: nil, loginFirst: false) }
        }
        steps.append { try $0.pullCurrentTerm() }
        Task { await pull(steps) }
    }

    /// Runs every pull in order. The accounts are separate, so one that needs
    /// the person does not hold back the other; the first problem is shown.
    private func pull(_ steps: [(Session) throws -> IsctCore.Operation]) async {
        guard let session else { return }
        problem = nil
        var first: String?
        for step in steps {
            do {
                if case let .schoolPull(outcomes) = try await step(session).value(),
                    let failed = outcomes.first(where: { $0.state == .reconnectRequired || $0.state == .error })
                {
                    first = first ?? (failed.state == .reconnectRequired
                        ? Self.describe(SdkError.Authentication(detail: ""))
                        : failed.message)
                }
            } catch {
                first = first ?? Self.describe(error)
            }
        }
        problem = first
        await refresh()
    }

    func saveCredentials(_ profile: SchoolAuthProfile, _ credentials: SchoolCredentials) async throws {
        guard let session else { return }
        _ = try await session.saveCredentials(profile: profile, credentials: credentials).value()
        await refreshAccounts()
        _ = try? session.ensureSchoolSession(profile: profile)
        if profile == .science { refreshFromSchool() }
    }

    /// Edits `settings.notifications` and replaces the settings object. Like the
    /// desktop store's `updateSettings`, edits run one at a time, each on the
    /// settings the previous one committed, so two quick toggles cannot both
    /// copy the same old object.
    func setNotifications(_ edit: @escaping @MainActor (inout [String: Any]) -> Void) async {
        let previous = settingsWrite
        let write = Task {
            await previous?.value
            await replaceNotifications(edit)
        }
        settingsWrite = write
        await write.value
    }

    private func replaceNotifications(_ edit: (inout [String: Any]) -> Void) async {
        guard let session else { return }
        var settings = records.settings
        var notifications = settings["notifications"] as? [String: Any] ?? [:]
        edit(&notifications)
        settings["notifications"] = notifications
        guard let data = try? JSONSerialization.data(withJSONObject: settings),
            let json = String(data: data, encoding: .utf8)
        else { return }
        let mutation = WorkspaceMutation(expectedRevision: revision, intent: .replaceSettings(settings: json))
        do {
            if case let .workspace(snapshot) = try await session.mutate(mutation: mutation).value() {
                revision = snapshot.revision
                records = WorkspaceRecords(json: snapshot.workspace)
            }
        } catch {
            problem = Self.describe(error)
        }
        await refresh()
        await NotificationScheduler.sync(session)
    }

    /// Whether this build can read a Matrix card (arm64 devices; not the Intel simulator).
    var canScanMatrix: Bool {
        (try? session?.capabilities().ocr) ?? false
    }

    /// Reads the 70 Matrix cells from one photo, in row order (A1…J1, A2…).
    func scanMatrix(_ image: Data) async throws -> [Recognition] {
        guard let session else { return [] }
        ocrProgress = 0
        guard case let .recognitions(cells) = try await session.scanMatrix(image: image).value() else { return [] }
        return cells
    }

    func deleteCredentials(_ profile: SchoolAuthProfile) async {
        guard let session else { return }
        _ = try? await session.deleteCredentials(profile: profile).value()
        await refreshAccounts()
    }

    // MARK: Formatting helpers

    nonisolated static func describe(_ error: Error) -> String {
        switch error {
        case SdkError.Network: return "オフラインです。保存済みのデータを表示しています。"
        case SdkError.Authentication: return "学校アカウントに再ログインが必要です。"
        case SdkError.Maintenance: return "学校のサイトはメンテナンス中です。"
        case SdkError.Timeout: return "学校サイトの応答がありません。"
        case SdkError.Busy: return "前の処理がまだ終わっていません。"
        case let SdkError.Unavailable(detail): return detail.isEmpty ? "この機能は今使えません。" : detail
        default: return error.localizedDescription
        }
    }

    nonisolated static func timestamp(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.timeZone = .tokyo
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }

    /// Core writes both `…T10:00:00+09:00` and, for stored runs, `…T01:00:00.123456789+00:00`;
    /// one formatter reads only one of the two.
    nonisolated static func date(_ raw: String) -> Date? {
        if let date = ISO8601DateFormatter().date(from: raw) { return date }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: raw)
    }

    nonisolated static func civil(_ date: Date) -> String {
        let parts = Calendar.tokyo.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    }
}

struct UndoAction: Equatable {
    let title: String
    let assignmentId: String
}

extension TimeZone {
    static let tokyo = TimeZone(identifier: "Asia/Tokyo") ?? .current
}

extension Calendar {
    /// School dates are Tokyo dates wherever the phone is.
    static let tokyo: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .tokyo
        calendar.locale = Locale(identifier: "ja_JP")
        calendar.firstWeekday = 2
        return calendar
    }()
}
