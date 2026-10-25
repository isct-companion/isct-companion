import BackgroundTasks
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
    /// Files waiting in each assignment's `submit/`, by assignment id, for
    /// 今日's rows (Core's `waitingHandIns`).
    private(set) var waiting: [String: Int] = [:]
    private(set) var periods: [TeachingPeriod] = []
    /// Course id → Core's display colour, identical on every surface.
    private(set) var colors: [String: String] = [:]
    private(set) var records = WorkspaceRecords() {
        didSet { Localizer.shared.use(records.settings.text("locale")) }
    }
    private(set) var credentials: SchoolCredentialStatus?
    private(set) var sessions: SchoolSessionStates?
    /// Core's sync planner: what runs, waits or failed, and when each source
    /// was last read. Rendered as is; Core decides what is due and when.
    private(set) var syncSnapshot: SyncSnapshot?
    /// Undo for the last hide, built from the intent Core committed.
    private(set) var undo: UndoAction?
    /// One "needs you" banner (sync feedback level 3).
    private(set) var problem: String?
    /// Cells read so far by a running Matrix scan, from Core's `ocr-progress`.
    private(set) var ocrProgress = 0
    /// Bumped whenever Core reports new content, so open screens re-query.
    private(set) var contentRevision = ""
    /// Core's cloud journal: providers, unfinished tasks, versions, recovery files.
    private(set) var cloud: SyncState?

    let browser = SchoolBrowserHost()
    let web: WebRouter
    private var session: Session?
    private var revision = ""
    private var boundaryTask: Task<Void, Never>?
    private var refreshTask: Task<Void, Never>?
    private var settingsWrite: Task<Void, Never>?
    private var wakeTask: Task<Void, Never>?
    private var opening: Task<Void, Never>?
    private let network = NWPathMonitor()
    private var networkState: SyncNetwork = .unknown
    private var foreground = true
    private var powerObserver: NSObjectProtocol?

    init() {
        web = WebRouter(browser: browser)
    }

    /// First launch: no school account configured yet.
    var needsAccount: Bool {
        guard let credentials else { return false }
        return credentials.science.availability == .notConfigured
            && credentials.tokyoTech.availability == .notConfigured
    }

    /// One of Core's sync sources (`lms`, `kyomu`, `references`, `catalog`).
    func syncSource(_ source: String) -> SourceView? {
        syncSnapshot?.sources.first { $0.source == source }
    }

    /// Tells Core's planner a fact or an intent; it never waits for a read.
    func syncHint(_ hint: Hint) {
        _ = try? session?.syncHint(hint: hint)
    }

    // MARK: Lifecycle

    /// Opens the session once; the scene and a background refresh both call
    /// this, and the later caller waits for the first.
    func start() async {
        if opening == nil { opening = Task { await open() } }
        await opening?.value
    }

    private func open() async {
        do {
            let support = try FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            let data = support.appendingPathComponent("core", isDirectory: true)
            let files = support.appendingPathComponent("files", isDirectory: true)
            for directory in [data, files] {
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            }
            // iCloud uses the device's account, so it needs no separate sign-in;
            // Google Drive signs in through `GoogleSignIn`, and Core reads its
            // saved authorization itself.
            #if ISCT_NO_ICLOUD
            // Personal-team builds carry no iCloud entitlement, and CKContainer
            // traps without it.
            let cloud: NativeCloud? = nil
            let icloud: [CloudProvider] = []
            #else
            // A sideloaded build may lack the container (CloudKitStore.available).
            let cloud: NativeCloud? = CloudKitStore.available ? CloudKitStore() : nil
            let icloud: [CloudProvider] = cloud == nil ? [] : [.icloud]
            #endif
            let config = SessionConfig(
                dataDirectory: data.path, filesDirectory: files.path, initialNetwork: .unknown,
                cloudProviders: icloud + [.googleDrive], authorizedCloudProviders: icloud)
            let session = try await Session.open(
                config: config, vault: KeychainVault(), browser: browser, cloud: cloud,
                notifications: nil, downloads: WebCookieSession(),
                webAuthentication: GoogleSignIn(), driveTokens: nil)
            self.session = session
            web.session = session
            let stream = try session.subscribe()
            Task { await self.listen(stream) }
            await refresh()
            await refreshAccounts()
            await refreshSyncSnapshot()
            await refreshCloud()
            status = .ready
            watchNetwork()
            watchPowerSaver()
            // A background refresh can launch the app without a scene in front.
            foreground = UIApplication.shared.applicationState != .background
            syncHint(foreground ? .foreground : .background)
            if foreground {
                // The launch check signs in through the school web views,
                // which only run in front; out of front the first `.focus`
                // does it instead.
                reportSchool(.launch)
                startWaking()
            } else {
                _ = try? session.suspend()
            }
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
            syncHint(.foreground)
            reportSchool(.focus)
            startWaking()
            Task { await refresh() }
        case .background:
            foreground = false
            wakeTask?.cancel()
            wakeTask = nil
            syncHint(.background)
            reportSchool(.focus)
            _ = try? session.suspend()
            Self.scheduleBackgroundRefresh()
        default:
            break
        }
    }

    /// The periodic wakeup desktop gives Core every second (`core_ports.rs`):
    /// Core's planner decides which school reads are due, then reminders,
    /// cloud sync and scheduled tasks run. Only while in front.
    private func startWaking() {
        guard wakeTask == nil else { return }
        wakeTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, let session = self.session else { return }
                _ = try? await session.wake().value()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    /// The `BGAppRefreshTask` the scene runs (`IsctCompanionApp`).
    static let backgroundRefresh = "app.isctcompanion.refresh"

    /// Asks iOS for the next background refresh. iOS picks the time; Core
    /// decides what is due when it comes.
    static func scheduleBackgroundRefresh() {
        let request = BGAppRefreshTaskRequest(identifier: backgroundRefresh)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 30 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    /// One background refresh: Core reads what it may out of front (LMS
    /// deadlines and assignment states over the token, reference files) and
    /// runs reminders and iCloud, within the budget iOS gives a refresh. The
    /// school web views stay closed. Cancelling the task ends it early.
    func backgroundRefresh() async {
        Self.scheduleBackgroundRefresh()
        await start()
        guard let session, let operation = try? session.backgroundWake(budgetSeconds: 25) else { return }
        await withTaskCancellationHandler {
            _ = try? await operation.awaitResult()
        } onCancel: {
            _ = try? operation.cancel()
        }
        await refresh()
    }

    /// School sign-in hears what desktop sends (`school-session.ts`): `launch`
    /// once when the session opens, then `focus` as the app enters or leaves the
    /// front. Core only acts on a visible focus.
    private func reportSchool(_ kind: SchoolSessionLifecycleKind) {
        _ = try? session?.schoolLifecycle(event: SchoolSessionLifecycle(kind: kind, visible: foreground))
    }

    private func watchNetwork() {
        network.pathUpdateHandler = { [weak self] path in
            let state: SyncNetwork =
                path.status != .satisfied ? .offline
                : path.usesInterfaceType(.wifi) ? .wifi
                : path.usesInterfaceType(.cellular) ? .cellular : .unknown
            Task { @MainActor in
                guard let self, self.networkState != state else { return }
                self.networkState = state
                self.syncHint(.network(network: state))
            }
        }
        network.start(queue: .global(qos: .utility))
    }

    /// Low Power Mode, which Core's planner reads as a reason to hold
    /// background reads.
    private func watchPowerSaver() {
        syncHint(.powerSaver(on: ProcessInfo.processInfo.isLowPowerModeEnabled))
        powerObserver = NotificationCenter.default.addObserver(
            forName: .NSProcessInfoPowerStateDidChange, object: nil, queue: .main
        ) { [weak self] _ in
            let on = ProcessInfo.processInfo.isLowPowerModeEnabled
            Task { @MainActor in self?.syncHint(.powerSaver(on: on)) }
        }
    }

    private func listen(_ stream: Subscription) async {
        while true {
            do {
                switch try await stream.next() {
                case let .event(event):
                    switch event.name {
                    case "sync-snapshot":
                        if let snapshot = try? decodeSyncSnapshot(payload: event.payload) {
                            apply(snapshot)
                        }
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
                    case "cloud-sync-state":
                        await refreshCloud()
                    // No screen here lists finder results; the catalog
                    // changes once per department while it downloads.
                    case "school-web-url", "school-download", "catalog-changed":
                        break
                    case "workspace-changed":
                        // A key re-read with nothing new renews only its
                        // freshness, which no screen here shows.
                        if !Self.onlyFreshness(event.payload) {
                            scheduleRefresh()
                        }
                    default:
                        scheduleRefresh()
                    }
                case .snapshotRequired:
                    scheduleRefresh()
                    await refreshAccounts()
                    await refreshSyncSnapshot()
                }
            } catch {
                return
            }
        }
    }

    /// Whether a `workspace-changed` names sync freshness and nothing else.
    /// An unreadable payload counts as a real change.
    private static func onlyFreshness(_ payload: String) -> Bool {
        guard let row = try? JSONSerialization.jsonObject(with: Data(payload.utf8)) as? [String: Any],
            let changes = row["changes"] as? [String], !changes.isEmpty
        else { return false }
        return changes.allSatisfy { $0 == "freshness" }
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
                .today(now: now), .assignmentBoard(now: now, term: nil), .teachingPeriods,
            ]).value() else { return }
            for value in result.values {
                switch value {
                case let .today(view): today = view
                case let .assignmentBoard(value):
                    board = value
                    writeShareChoices(value)
                    await refreshWaiting()
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
            if changed || weeks.isEmpty {
                weeks = [:]
                Task { await prefetchTimetable() }
            }
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

    private func refreshSyncSnapshot() async {
        guard let session else { return }
        if case let .sync(snapshot)? = try? await session.syncSnapshot().value() {
            apply(snapshot)
        }
    }

    /// A read that started before a newer event must not replace it.
    private func apply(_ snapshot: SyncSnapshot) {
        guard snapshot.generation >= (syncSnapshot?.generation ?? 0) else { return }
        syncSnapshot = snapshot
    }

    /// The week from `monday`, read with `weekReach` weeks on either side in
    /// one query, so stepping or swiping through nearby weeks draws at once.
    func week(starting monday: Date) async -> ScheduleView? {
        let key = Self.civil(monday)
        if let cached = weeks[key] { return cached }
        guard let session, let term = today?.term else { return nil }
        let calendar = Calendar.tokyo
        let first = calendar.date(byAdding: .day, value: -7 * Self.weekReach, to: monday) ?? monday
        let last = calendar.date(byAdding: .day, value: 7 * Self.weekReach + 4, to: monday) ?? monday
        let query = WorkspaceQuery.scheduleView(
            term: term, from: Self.civil(first), to: Self.civil(last), includePersonal: false)
        guard case let .schedule(result)? = try? await session.scheduleQuery(queries: [query]).value(),
            case let .scheduleView(view)? = result.values.first
        else { return nil }
        for offset in -Self.weekReach...Self.weekReach {
            guard let start = calendar.date(byAdding: .day, value: 7 * offset, to: monday) else { continue }
            let from = Self.civil(start)
            let to = Self.civil(calendar.date(byAdding: .day, value: 4, to: start) ?? start)
            var slice = view
            // Civil dates compare as text.
            slice.agendas = view.agendas.filter { $0.date >= from && $0.date <= to }
            weeks[from] = slice
        }
        return weeks[key]
    }

    /// Weeks read on either side of the one asked for.
    private static let weekReach = 4

    /// Weeks already read this revision, by Monday, so the timetable draws
    /// them at once; a new revision re-reads them.
    private(set) var weeks: [String: ScheduleView] = [:]

    /// A quarter as an iCalendar file, written by Core as the desktop's export.
    func quarterIcs(term: String, scope: CalendarExportScope) async throws -> String {
        guard let session else { throw SdkError.Unavailable(detail: "quarter calendar") }
        let query = WorkspaceQuery.quarterIcs(term: term, scope: scope, generatedAt: Self.timestamp(Date()))
        guard case let .schedule(result) = try await session.scheduleQuery(queries: [query]).value(),
            case let .quarterIcs(text)? = result.values.first
        else { throw SdkError.Unavailable(detail: "quarter calendar") }
        return text
    }

    /// Reads this week and its axis ahead of the timetable being opened.
    private func prefetchTimetable() async {
        guard let view = await week(starting: TimetableScreen.monday(of: Date())) else { return }
        let blocks = view.agendas.flatMap(\.occurrences).filter { [.course, .exam].contains($0.kind) }
        let earliest = blocks.map(\.startTime).min { TimetableAxis.minutes($0) < TimetableAxis.minutes($1) }
        let latest = blocks.map(\.endTime).max { TimetableAxis.minutes($0) < TimetableAxis.minutes($1) }
        _ = await timetableAxis(earliest: earliest, latest: latest)
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

    /// One quarter's assignments as Core groups them, for 授業's 課題.
    func board(term: String) async -> AssignmentBoard? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .assignmentBoard(now: Self.timestamp(Date()), term: term)
            ]).value(),
            case let .assignmentBoard(board)? = result.values.first
        else { return nil }
        return board
    }

    /// Core's equal-row timetable axis, widened to the week's earliest start
    /// and latest end when those leave the teaching day.
    func timetableAxis(earliest: String?, latest: String?) async -> TimetableAxis? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .timetableAxis(earliest: earliest, latest: latest)
            ]).value(),
            case let .timetableAxis(axis)? = result.values.first
        else { return nil }
        lastTimetableAxis = axis
        return axis
    }

    /// The last axis Core gave, so a timetable draws its rows at once.
    private(set) var lastTimetableAxis: TimetableAxis?

    /// A course's LMS content lesson by lesson.
    func courseContent(_ id: String) async -> CourseContent? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .courseContent(courseId: id, now: Self.timestamp(Date()))
            ]).value(),
            case let .courseContent(content)? = result.values.first
        else { return nil }
        return content
    }

    /// One assignment with its LMS text and files, for the in-app page.
    func assignmentDetail(_ id: String) async -> AssignmentDetail? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .assignmentDetail(assignmentId: id)
            ]).value(),
            case let .assignmentDetail(detail)? = result.values.first
        else { return nil }
        return detail
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

    /// Pull to refresh and "Refresh now": asks Core's planner for everything;
    /// it orders the sources and reports progress in the sync snapshot.
    func refreshFromSchool() {
        problem = nil
        syncHint(.refresh(scope: .everything))
    }

    func saveCredentials(_ profile: SchoolAuthProfile, _ credentials: SchoolCredentials) async throws {
        guard let session else { return }
        _ = try await session.saveCredentials(profile: profile, credentials: credentials).value()
        await refreshAccounts()
        _ = try? session.ensureSchoolSession(profile: profile)
        syncHint(.signedIn(profile: profile))
    }

    /// The saved school OTP setup as an `otpauth://` link, read only when the
    /// person asks to add it to an authenticator or copy it.
    func otpSetupLink() async throws -> String {
        guard let session, case let .text(value) = try await session.schoolOtpLink(draft: nil).value() else {
            throw CancellationError()
        }
        return value
    }

    /// Edits `settings.notifications` and replaces the settings object.
    func setNotifications(_ edit: @escaping @MainActor (inout [String: Any]) -> Void) async {
        await updateSettings { settings in
            var notifications = settings["notifications"] as? [String: Any] ?? [:]
            edit(&notifications)
            settings["notifications"] = notifications
        }
    }

    /// Edits the settings object and replaces it, as the desktop store's
    /// `updateSettings` does: edits run one at a time, each on the settings
    /// the previous one committed, so two quick toggles cannot both copy the
    /// same old object.
    func updateSettings(_ edit: @escaping @MainActor (inout [String: Any]) -> Void) async {
        let previous = settingsWrite
        let write = Task {
            await previous?.value
            await replaceSettings(edit)
        }
        settingsWrite = write
        await write.value
    }

    private func replaceSettings(_ edit: (inout [String: Any]) -> Void) async {
        guard let session else { return }
        var settings = records.settings
        edit(&settings)
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

    /// Starts a live Matrix scan; its ten-second budget starts now.
    func beginMatrixCamera() throws -> MatrixCameraSession {
        guard let session else { throw SdkError.Closed }
        return try session.beginMatrixCamera()
    }

    func deleteCredentials(_ profile: SchoolAuthProfile) async {
        guard let session else { return }
        _ = try? await session.deleteCredentials(profile: profile).value()
        await refreshAccounts()
    }

    // MARK: Formatting helpers

    nonisolated static func describe(_ error: Error) -> String {
        switch error {
        case SdkError.Network: return t("You're offline. Showing saved data.")
        case SdkError.Authentication: return t("Sign in to your school account again.")
        case SdkError.Maintenance: return t("The school site is under maintenance.")
        case SdkError.Timeout: return t("The school site is not responding.")
        case SdkError.Busy: return t("The previous task has not finished yet.")
        case let SdkError.Unavailable(detail): return detail.isEmpty ? t("This is not available right now.") : detail
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

// MARK: - Backup

/// The desktop's Backup settings over the same Core journal: iCloud and Drive on or off,
/// automatic or by-hand sync, earlier versions, local recovery files and a
/// backup file. Core decides what uploads, merges and restores.
extension AppModel {
    func refreshCloud() async {
        guard let session else { return }
        if case let .cloud(state)? = try? await session.cloudState().value() {
            cloud = state
        }
    }

    /// Turning Drive on first shows Google's sign-in; Core keeps it off with
    /// the reason if the sign-in fails.
    func setCloudEnabled(_ provider: CloudProvider, _ enabled: Bool) async {
        await cloudOperation { try $0.setCloudEnabled(provider: provider, enabled: enabled) }
    }

    func setCloudAutomatic(_ automatic: Bool) async {
        await cloudOperation { try $0.setCloudAutomatic(enabled: automatic) }
    }

    func syncCloud(_ direction: Direction) async {
        await cloudOperation { try $0.syncCloud(direction: direction) }
        await refresh()
    }

    /// Restores a cloud version or a local recovery file, on every connected device.
    func restoreCloud(_ id: String) async {
        await cloudOperation { try $0.restoreCloud(id: id) }
        await refresh()
    }

    /// The current workspace as a backup file (the desktop's Export…).
    func exportBackup() async -> String? {
        guard let session else { return nil }
        do {
            guard case let .text(value) = try await session.exportBackup().value() else { return nil }
            return value
        } catch {
            problem = Self.describe(error)
            return nil
        }
    }

    /// Imports a backup file from this or another device; Core saves a recovery
    /// file first and publishes the import to iCloud like any restore.
    func importBackup(_ data: Data) async throws {
        guard let session else { return }
        // Core validates the document and rejects anything that is not a backup.
        _ = try await session.applyBackup(document: String(decoding: data, as: UTF8.self)).value()
        await refreshCloud()
        await refresh()
    }

    private func cloudOperation(_ start: (Session) throws -> IsctCore.Operation) async {
        guard let session else { return }
        do {
            if case let .cloud(state) = try await start(session).value() {
                cloud = state
            }
        } catch {
            problem = Self.describe(error)
            await refreshCloud()
        }
    }
}

// MARK: - Reset

/// Settings → Reset over Core's `reset`: Core decides what each scope
/// removes, stops iCloud sync before a full reset and announces the change,
/// which re-reads every screen.
extension AppModel {
    func resetPreview(_ request: ResetRequest) async -> ResetSummary? {
        guard let session else { return nil }
        return try? await session.resetPreview(request: request)
    }

    func reset(_ request: ResetRequest) async throws {
        guard let session else { return }
        _ = try await session.reset(request: request, web: SchoolWebSignOut())
        problem = nil
        await refreshAccounts()
        await refreshSyncSnapshot()
        await refreshCloud()
    }

    /// After a full reset, as the desktop's: iCloud on again, then its copy pulled.
    func restoreFromCloud() async throws {
        guard let session else { return }
        _ = try await session.setCloudEnabled(provider: .icloud, enabled: true).value()
        _ = try await session.syncCloud(direction: .pull).value()
        await refreshCloud()
        await refresh()
    }
}

// MARK: - Timetable session marks and upcoming exams

extension AppModel {
    /// Opening an item of 今日's 新着: Core stops listing it until it changes.
    func markRecentSeen(_ list: RecentKind, _ id: String) async {
        await markSeen(.markRecentSeen(list: list, id: id))
    }

    /// 新着's read all: Core stops listing everything new now, past the few it
    /// shows too.
    func markAllRecentSeen() async {
        await markSeen(.markAllRecentSeen)
    }

    private func markSeen(_ intent: WorkspaceIntent) async {
        guard let session else { return }
        let mutation = WorkspaceMutation(expectedRevision: revision, intent: intent)
        do {
            if case let .workspace(snapshot) = try await session.mutate(mutation: mutation).value() {
                revision = snapshot.revision
            }
        } catch {
            // The item only stays new a little longer; nothing to tell.
        }
        await refresh()
    }

    /// Marks one session (`Occurrence.key`) as `exam`, `quiz`, `cancelled` or
    /// `class` (a plain class; Core clears the mark when that is the default).
    /// Core republishes the schedule; the new content revision re-reads the week.
    func markSession(_ key: String, _ mark: String) async {
        guard let session else { return }
        let mutation = WorkspaceMutation(expectedRevision: revision, intent: .markSession(key: key, mark: mark))
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

    /// Exams Core lists as upcoming (linked and scheduled) from today
    /// through `days` days ahead, for 今日.
    func upcomingExams(days: Int = 14) async -> [ExamView] {
        guard let session, let term = today?.term else { return [] }
        let now = Date()
        let from = Self.civil(now)
        let last = Self.civil(Calendar.tokyo.date(byAdding: .day, value: days, to: now) ?? now)
        let query = WorkspaceQuery.scheduleView(term: term, from: from, to: from, includePersonal: false)
        guard case let .schedule(result)? = try? await session.scheduleQuery(queries: [query]).value(),
            case let .scheduleView(view)? = result.values.first
        else { return [] }
        return view.upcomingExams.filter { $0.date <= last }
    }
}

// MARK: Course page: lesson marks

extension AppModel {
    /// Records `sectionId` as the lesson of the class at `occurrence` (Core's
    /// `CourseContent.classAt`), as a double click on the desktop rail does.
    /// `false` when Core did not take it.
    func recordLesson(courseId: String, occurrence: String, sectionId: UInt64) async -> Bool {
        guard let session else { return false }
        let mutation = WorkspaceMutation(
            expectedRevision: revision,
            intent: .recordLesson(courseId: courseId, occurrence: occurrence, sectionId: sectionId))
        var recorded = false
        do {
            if case let .workspace(snapshot) = try await session.mutate(mutation: mutation).value() {
                revision = snapshot.revision
            }
            recorded = true
        } catch SdkError.Conflict {
            // Another device changed the workspace: the re-read lets the person retry.
        } catch {
            problem = Self.describe(error)
        }
        await refresh()
        return recorded
    }
}

// MARK: Settings pages (学習プロフィール, データ更新)

extension AppModel {
    /// The study profile as the workspace stores it (`academicProfile`).
    struct StudyProfile: Equatable {
        var admissionYear: Int32?
        var enrollments: [StudyEnrollment]
    }

    func studyProfile() async -> StudyProfile? {
        guard let session,
            case let .workspace(snapshot)? = try? await session.snapshot().value(),
            let root = (try? JSONSerialization.jsonObject(with: Data(snapshot.workspace.utf8))) as? [String: Any]
        else { return nil }
        let profile = root["academicProfile"] as? [String: Any] ?? [:]
        let rows = profile["studyEnrollments"] as? [[String: Any]] ?? []
        return StudyProfile(
            admissionYear: (profile["admissionYear"] as? NSNumber)?.int32Value,
            enrollments: rows.map { row in
                StudyEnrollment(
                    stage: Self.stage(row["stage"] as? String),
                    entryKind: Self.entryKind(row["entryKind"] as? String),
                    entryGrade: (row["entryGrade"] as? NSNumber)?.uint8Value,
                    admittedTerm: row["admittedTerm"] as? String ?? "")
            })
    }

    /// Saves the cohort and degree history through the desktop's intent.
    func setStudyProfile(_ profile: StudyProfile) async throws {
        guard let session else { return }
        let mutation = WorkspaceMutation(
            expectedRevision: revision,
            intent: .setStudyProfile(admissionYear: profile.admissionYear, studyEnrollments: profile.enrollments))
        _ = try await session.mutate(mutation: mutation).value()
        await refresh()
    }

    /// The syllabus departments, as the desktop's course finder lists them.
    func departments() async -> [FinderDepartment] {
        guard let session, let term = today?.term, let year = UInt16(term.prefix(4)),
            case let .finder(page)? = try? await session.finderQuery(request: FinderRequest(
                year: year, term: term, includeIneligible: false, keys: [], input: "", action: nil,
                offset: 0, limit: 0
            )).value()
        else { return [] }
        return page.context.departments
    }

    private static func stage(_ raw: String?) -> StudyStage {
        switch raw {
        case "undergraduate": .undergraduate
        case "master": .master
        case "professional": .professional
        case "doctoral": .doctoral
        default: .unknown
        }
    }

    private static func entryKind(_ raw: String?) -> StudyEntryKind {
        switch raw {
        case "regular": .regular
        case "transfer": .transfer
        default: .unknown
        }
    }
}

// MARK: - Hand-in (see HandIn.swift)

extension AppModel {
    /// The files waiting in the assignment's `submit/`.
    func submitFiles(_ assignment: String) async -> [LocalFile] {
        guard let session,
            case let .files(files)? = try? await session.assignmentSubmitFiles(assignmentId: assignment).value()
        else { return [] }
        return files
    }

    /// Copies the files into the assignment's `submit/`; the sources stay.
    func addSubmitFiles(_ assignment: String, _ files: [SharedFile]) async throws -> [LocalFile] {
        guard let session else { throw SdkError.Closed }
        guard case let .files(listed) = try await session.addAssignmentFiles(assignmentId: assignment, files: files).value()
        else { return [] }
        await refreshWaiting()
        return listed
    }

    func removeSubmitFile(_ assignment: String, name: String) async throws -> [LocalFile] {
        guard let session else { throw SdkError.Closed }
        guard case let .files(listed) = try await session.removeAssignmentFile(assignmentId: assignment, name: name).value()
        else { return [] }
        await refreshWaiting()
        return listed
    }

    /// Re-reads which assignments have files waiting to be handed in.
    func refreshWaiting() async {
        guard let session,
            case let .waiting(assignments)? = try? await session.waitingHandIns().value()
        else { return }
        waiting = Dictionary(assignments.map { ($0.assignmentId, Int($0.files)) }, uniquingKeysWith: +)
    }

    /// Queues `submit/` for the LMS draft area and returns the page that
    /// receives it, for the in-app web view.
    func handIn(_ assignment: String) async throws -> String {
        guard let session else { throw SdkError.Closed }
        let messages = UploadMessages(
            waiting: t("Preparing your LMS upload…"),
            queued: t("Files sent to the upload area. Wait for LMS to finish, review the files, then submit yourself."),
            failed: t("The upload area is not available. Choose the files in LMS."))
        guard case let .text(url) = try await session.handInAssignment(assignmentId: assignment, messages: messages).value()
        else { throw SdkError.Unavailable(detail: "") }
        return url
    }

    /// The quarter's open assignments, for the share sheet's list.
    func writeShareChoices(_ board: AssignmentBoard) {
        let groups = board.today
        let open = groups.pinned + groups.dueThisWeek + groups.later + groups.undated + groups.pastRecent
        var seen = Set<String>()
        let choices = open.filter { seen.insert($0.assignmentId).inserted }.map {
            SharedInbox.Choice(id: $0.assignmentId, title: $0.title, course: $0.courseTitle, due: Format.due($0.dueAt))
        }
        SharedInbox.writeChoices(SharedInbox.Choices(language: Localizer.shared.language, assignments: choices))
    }
}
