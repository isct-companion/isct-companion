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
    private(set) var periods: [TeachingPeriod] = []
    /// Course id → Core's display colour, identical on every surface.
    private(set) var colors: [String: String] = [:]
    private(set) var records = WorkspaceRecords() {
        didSet { Localizer.shared.use(records.settings.text("locale")) }
    }
    private(set) var credentials: SchoolCredentialStatus?
    /// Core's school sign-in read model: each account's state, why it is
    /// stuck, when Core tries again, maintenance. The higher `revision` wins.
    private(set) var schoolSessions: SchoolSessionSnapshot?
    /// Core's sync planner: what runs, waits or failed, and when each source
    /// was last read. Rendered as is; Core decides what is due and when.
    private(set) var syncSnapshot: SyncSnapshot?
    /// Undo for the last hide, built from the intent Core committed.
    private(set) var undo: UndoAction?
    /// One "needs you" banner (sync feedback level 3).
    private(set) var problem: String?
    /// Cells read so far by a running Matrix scan, from Core's `ocr-progress`.
    private(set) var ocrProgress = 0
    private var ocrOperation: UInt64?
    /// The commit sequence of the latest answers; open screens re-query when
    /// it moves.
    private(set) var contentRevision = ""
    @ObservationIgnored private(set) lazy var backup = CloudBackup(
        session: { [weak self] in self?.session },
        refreshWorkspace: { [weak self] in
            await self?.refresh()
        },
        reportFailure: { [weak self] error in self?.problem = AppModel.describe(error) }
    )
    /// A newer release on the App Store, shown in Settings.
    private(set) var availableUpdate: AppRelease?
    /// A release a check of its own found, offered once over the page.
    var updateOffer: AppRelease?
    /// The ScienceTokyoExam account and its upload: an `Account` answer, then
    /// Core's `exam-account` events.
    private(set) var examAccount: ExamAccount?
    /// A check the person started found nothing newer.
    private(set) var upToDate = false
    private(set) var checkingUpdate = false

    let browser = SchoolBrowserHost()
    let web: WebRouter
    private(set) var session: Session?
    private var revision = ""
    private let dashboardReads = ScheduleReads()
    let timetableWeeks = TimetableWeeks()
    @ObservationIgnored private var pendingSessionMark: Task<Void, Never>?
    private(set) var upcomingExams: [ExamView] = []
    private var boundaryTask: Task<Void, Never>?
    private var settingsWrite: Task<Void, Never>?
    /// The one timer for Core's next deadline (`WakeDirective`).
    private var deadlineTask: Task<Void, Never>?
    private var directive: WakeDirective?
    private var opening: Task<Void, Never>?
    private let network = NWPathMonitor()
    private var networkState: SyncNetwork = .unknown
    private var powerObserver: NSObjectProtocol?

    init() {
        web = WebRouter(browser: browser)
        Self.shared = self
    }

    /// The model the processing task the app registers at launch reaches.
    nonisolated(unsafe) static weak var shared: AppModel?

    /// First launch: no school account configured yet.
    var needsAccount: Bool {
        guard let credentials else { return false }
        return credentials.science.availability == .notConfigured
            && credentials.tokyoTech.availability == .notConfigured
    }

    /// One school account's session, as Core last reported it.
    func profileSession(_ profile: SchoolAuthProfile) -> SchoolProfileSession? {
        profile == .science ? schoolSessions?.profiles.science : schoolSessions?.profiles.tokyoTech
    }

    /// One of Core's sync sources (`lms`, `kyomu`, `references`, `catalog`).
    func syncSource(_ source: String) -> SourceView? {
        syncSnapshot?.sources.first { $0.source == source }
    }

    /// Tells Core's planner a fact or an intent; it never waits for a read.
    func retrySync(_ plan: RetryPlan) {
        for scope in plan.scopes { syncHint(.refresh(scope: scope)) }
    }

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
            // Google Drive signs in through `BrowserAuthentication`, and Core reads its
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
            let inFront = UIApplication.shared.applicationState != .background
            let config = SessionConfig(
                dataDirectory: data.path, filesDirectory: files.path,
                appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "",
                initialNetwork: .unknown, startSuspended: !inFront,
                cloudProviders: icloud + [.googleDrive], authorizedCloudProviders: icloud)
            let session = try await Session.open(
                config: config, vault: KeychainVault(), browser: browser, cloud: cloud,
                webCookies: WebCookieSeed(),
                webAuth: BrowserAuthentication(),
                iosGoogleClientId: Bundle.main.object(forInfoDictionaryKey: "IsctGoogleClientID") as? String,
                browserOpener: nil, driveTokens: nil)
            if let widgets = WidgetStorage() {
                try session.attachWidgets(port: widgets)
            }
            try session.attachReminderPort(port: NotificationScheduler(session: session))
            self.session = session
            let stream = try session.subscribe()
            Task { await self.listen(stream) }
            // The web screen reads page snapshots only once events stream.
            web.session = session
            await refresh()
            await refreshAccounts()
            await refreshSyncSnapshot()
            await backup.refresh()
            status = .ready
            watchNetwork()
            watchPowerSaver()
            // A background refresh can launch the app without a scene in front.
            let foreground = UIApplication.shared.applicationState != .background
            syncHint(foreground ? .foreground : .background)
            if let directive = try? session.wakeDirective() { apply(directive) }
            if foreground && !inFront { _ = try? session.resume() }
            if !foreground && inFront { _ = try? session.suspend() }
            // The launch's first `.active` came before the session opened,
            // so its update check never ran.
            if foreground { Task { await checkForUpdate(launch: true) } }
        } catch {
            status = .failed(Self.describe(error))
        }
    }

    func refreshWidgets() {
        try? session?.refreshWidgets()
    }

    func refreshReminders() {
        try? session?.refreshReminderPort()
    }

    func widgetRoute(_ url: URL) async throws -> WidgetRoute? {
        guard let session,
              case .widgetRoute(let route) = try await session.widgetRoute(url: url.absoluteString).value()
        else { return nil }
        return route
    }

    func scenePhase(_ phase: ScenePhase) {
        guard let session else { return }
        switch phase {
        case .active:
            _ = try? session.resume()
            syncHint(.foreground)
            armDeadline()
            scheduleRefresh(force: true)
            Task { await checkForUpdate() }
        case .background:
            deadlineTask?.cancel()
            deadlineTask = nil
            syncHint(.background)
            _ = try? session.suspend()
            scheduleBackgroundWork()
        default:
            break
        }
    }

    /// Takes Core's directive, unless an older one: the in-front timer and
    /// the OS background requests follow it.
    private func apply(_ next: WakeDirective) {
        if let directive, directive.revision >= next.revision { return }
        directive = next
        if UIApplication.shared.applicationState != .background {
            armDeadline()
        } else {
            scheduleBackgroundWork()
        }
    }

    /// One task sleeps until the directive's deadline and tells Core; the
    /// next directive arms it again. Only while in front: out of front the
    /// OS gives Core its opportunities.
    private func armDeadline() {
        deadlineTask?.cancel()
        deadlineTask = nil
        guard let session, let directive, let raw = directive.at, let at = Self.date(raw) else { return }
        let revision = directive.revision
        deadlineTask = Task {
            let wait = at.timeIntervalSinceNow
            if wait > 0 { try? await Task.sleep(for: .seconds(wait)) }
            guard !Task.isCancelled else { return }
            _ = try? session.deadlineReached(revision: revision)
        }
    }

    /// The `BGAppRefreshTask` the scene runs (`IsctCompanionApp`): reads
    /// and cloud sync.
    static let backgroundRefresh = "app.isctcompanion.refresh"
    /// The `BGProcessingTask` registered at launch: Core's local work
    /// (reminder plan, the person's tasks, maintenance), no network needed.
    static let backgroundMaintenance = "app.isctcompanion.maintenance"

    /// Asks iOS for the background opportunities Core's directive names, at
    /// the earliest time each is due. iOS picks when they come.
    func scheduleBackgroundWork() {
        let requests = directive?.background ?? []
        // A kind Core no longer asks for is not left to wake the app.
        if !requests.contains(where: { $0.work == .network }) {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.backgroundRefresh)
        }
        if !requests.contains(where: { $0.work == .local }) {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.backgroundMaintenance)
        }
        for request in requests {
            let earliest = Self.date(request.earliest) ?? Date()
            switch request.work {
            case .network:
                let task = BGAppRefreshTaskRequest(identifier: Self.backgroundRefresh)
                task.earliestBeginDate = earliest
                try? BGTaskScheduler.shared.submit(task)
            case .local:
                let task = BGProcessingTaskRequest(identifier: Self.backgroundMaintenance)
                task.earliestBeginDate = earliest
                task.requiresNetworkConnectivity = false
                task.requiresExternalPower = false
                try? BGTaskScheduler.shared.submit(task)
            }
        }
    }

    /// One background refresh: Core reads what it may out of front, signing
    /// in with the saved details when a read needs it, and runs reminders and
    /// iCloud, within the budget iOS gives a refresh. The school web views
    /// stay closed. Cancelling the task ends it early.
    func backgroundRefresh() async {
        await opportunity(budget: 25, network: true)
    }

    /// One processing task: Core's local work only. iOS's expiration ends it.
    func backgroundMaintenance(_ task: BGProcessingTask) {
        Task {
            let id = UUID().uuidString
            task.expirationHandler = { [weak self] in
                Task { @MainActor in _ = try? self?.session?.endBackgroundOpportunity(id: id) }
            }
            await opportunity(budget: 60, network: false, id: id)
            task.setTaskCompleted(success: true)
        }
    }

    private func opportunity(budget: UInt32, network: Bool, id: String = UUID().uuidString) async {
        await start()
        guard let session,
            let operation = try? session.beginBackgroundOpportunity(id: id, budgetSeconds: budget, network: network)
        else { return }
        await withTaskCancellationHandler {
            _ = try? await operation.awaitResult()
        } onCancel: {
            _ = try? operation.cancel()
        }
        await refresh()
        if let directive = try? session.wakeDirective() { apply(directive) }
        scheduleBackgroundWork()
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
                    case "school-session":
                        if let snapshot = try? decodeSchoolSession(payload: event.payload) {
                            apply(snapshot)
                            await refreshCredentials()
                        } else {
                            await refreshAccounts()
                        }
                    case "ocr-progress":
                        if let row = try? JSONSerialization.jsonObject(with: Data(event.payload.utf8)) as? [String: Any],
                            let operation = (row["operationId"] as? NSNumber)?.uint64Value,
                            operation == ocrOperation,
                            let completed = (row["completed"] as? NSNumber)?.intValue
                        {
                            ocrProgress = completed
                        }
                    case "cloud-sync-state":
                        await backup.refresh()
                    case "wake-directive":
                        if let next = try? decodeWakeDirective(payload: event.payload) { apply(next) }
                    case "exam-account":
                        if let account = try? decodeExamAccount(payload: event.payload) { examAccount = account }
                    case "school-web-page", "school-web-notice":
                        web.receive(event.name, payload: event.payload)
                    // No screen here lists finder results; the catalog
                    // changes once per department while it downloads.
                    case "school-download", "catalog-changed":
                        break
                    case "workspace-changed":
                        scheduleRefresh()
                    default:
                        break
                    }
                case .snapshotRequired:
                    try? session?.refreshWidgets()
                    scheduleRefresh(force: true)
                    if let next = try? session?.wakeDirective() { apply(next) }
                    web.refreshPages()
                    await refreshAccounts()
                    await refreshSyncSnapshot()
                    // A missed `exam-account` event: read the account again
                    // once the past-exams page has shown it. Its own task, as
                    // an upload holds the account until it ends.
                    if examAccount != nil { Task { _ = try? await self.examShare(.account) } }
                }
            } catch {
                return
            }
        }
    }

    @discardableResult
    private func scheduleRefresh(force: Bool = false) -> Task<Void, Never>? {
        guard let session else { return nil }
        if force {
            dashboardReads.invalidate()
            timetableWeeks.invalidate()
        } else {
            timetableWeeks.invalidateIfChanged(session)
        }
        return dashboardReads.refresh(session, read: { generation in
            try await self.readDashboard(session, generation: generation)
        }, failed: { self.problem = Self.describe($0) })
    }

    func refresh() async { await scheduleRefresh(force: true)?.value }

    private func readDashboard(_ session: Session, generation: Int) async throws {
        let date = Date()
        let now = Self.timestamp(date)
        let from = Self.civil(date)
        let last = Self.civil(Calendar.tokyo.date(byAdding: .day, value: 14, to: date) ?? date)
        let previousRevision = revision
        guard case let .text(current) = try await session.workspaceRevision().value(),
              case var .schedule(result) = try await session.scheduleQuery(queries: [
                .records(fields: ["courses", "assignments"]), .today(now: now),
                .upcomingExams(from: from, to: last),
                .assignmentBoard(now: now, term: nil), .teachingPeriods,
              ]).value() else { throw SdkError.Unavailable(detail: "dashboard") }

        // A new install may have no term for an overview yet.
        var newColors: [String: String]?
        if case let .schedule(overview)? = try? await session.scheduleQuery(queries: [.workspaceOverview(term: "")]).value(),
           case let .workspaceOverview(view)? = overview.values.first {
            newColors = view.colors
            result.reads.append(contentsOf: overview.reads)
        }
        if let boundary = result.values.compactMap({ value -> Date? in
            guard case let .today(view) = value, let raw = view.nextBoundary else { return nil }
            return Self.date(raw)
        }).first, boundary <= Date() { return }
        guard try dashboardReads.accept(result, started: generation, session: session) else { return }
        if revision == previousRevision { revision = current }
        // No suspension between accepting the snapshot and publishing its views.
        for value in result.values {
            switch value {
            case let .records(json): records = WorkspaceRecords(json: json)
            case let .today(view):
                if let old = today?.term, old != view.term { timetableWeeks.invalidate() }
                today = view
            case let .upcomingExams(exams): upcomingExams = exams
            case let .assignmentBoard(value): board = value; writeShareChoices(value)
            case let .teachingPeriods(value): periods = value
            default: break
            }
        }
        if let newColors { colors = newColors }
        contentRevision = "\(generation)-\(result.seq)"
        scheduleBoundary()
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
        if case let .schoolSession(snapshot)? = try? await session.schoolSession().value() {
            apply(snapshot)
        }
        await refreshCredentials()
    }

    private func refreshCredentials() async {
        guard let session else { return }
        if case let .credentials(status)? = try? await session.credentialStatus().value() {
            credentials = status
        }
    }

    /// A read that started before a newer school session must not replace it.
    func apply(_ snapshot: SchoolSessionSnapshot) {
        if let current = schoolSessions, snapshot.revision <= current.revision { return }
        schoolSessions = snapshot
    }

    /// Reads the 70 Matrix cells from one photo, in row order (A1…J1, A2…).
    func scanMatrix(_ image: Data) async throws -> [MatrixCell] {
        guard let session else { return [] }
        ocrProgress = 0
        let operation = try session.scanMatrix(image: image)
        let id = operation.id()
        ocrOperation = id
        defer { if ocrOperation == id { ocrOperation = nil } }
        guard case let .recognitions(cells) = try await operation.value() else { return [] }
        return cells
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

    func week(starting monday: Date) async -> ScheduleView? {
        guard let session, let term = today?.term else { return nil }
        return await timetableWeeks.week(starting: monday, term: term, session: session)
    }

    /// A quarter as an iCalendar file, written by Core as the desktop's export.
    func quarterIcs(term: String, scope: CalendarExportScope) async throws -> String {
        guard let session else { throw SdkError.Unavailable(detail: "quarter calendar") }
        let query = WorkspaceQuery.quarterIcs(term: term, scope: scope, generatedAt: Self.timestamp(Date()))
        guard case let .schedule(result) = try await session.scheduleQuery(queries: [query]).value(),
            case let .quarterIcs(text)? = result.values.first
        else { throw SdkError.Unavailable(detail: "quarter calendar") }
        return text
    }

    /// The quarter's courses as Core lists them for the course list.
    func lectureList(_ term: String) async -> LectureList? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .lectureList(term: term, now: Self.timestamp(Date()))
            ]).value(),
            case let .lectureList(view)? = result.values.first
        else { return nil }
        return view
    }

    func examShare(_ action: ExamAction) async throws -> ExamAnswer {
        guard let session,
            case let .examShare(answer) = try await session.examShare(action: action).value()
        else { throw SdkError.Unavailable(detail: "unavailable") }
        if case let .account(account) = answer { examAccount = account }
        return answer
    }

    func examCourses() async -> ExamCourseList? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .examCourses
            ]).value(),
            case let .examCourses(list)? = result.values.first
        else { return nil }
        return list
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
        return axis
    }

    /// The last axis Core gave, so a timetable draws its rows at once.
    var lastTimetableAxis: TimetableAxis?

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

    func deadlineDisplay(at: String) async -> TaskDeadlineView? {
        guard let session,
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .deadlineDisplay(at: at, now: Self.timestamp(Date(), fractional: true))
            ]).value(),
            case let .deadlineDisplay(display)? = result.values.first
        else { return nil }
        return display
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

    /// What an item or link of the course `courseId` opens as; a file is
    /// downloaded (or reused) through the signed-in LMS session.
    func openMaterial(courseId: String, source: String) async -> IsctCore.Material? {
        guard let session,
            case let .material(material)? = try? await session.openMaterial(
                courseId: courseId, source: source, download: true, readText: false, limit: nil
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

    /// Route a committed intent through the same pending read as its event.
    private func commit(_ intent: WorkspaceIntent) async throws {
        guard let session else { throw SdkError.Closed }
        do {
            let mutation = WorkspaceMutation(expectedRevision: revision, intent: intent)
            guard case let .committed(receipt) = try await session.mutateCommit(mutation: mutation).value()
            else { throw SdkError.Unavailable(detail: "workspace commit") }
            revision = receipt.revision
            await scheduleRefresh()?.value
        } catch SdkError.Conflict(let detail) {
            await refresh()
            throw SdkError.Conflict(detail: detail)
        }
    }

    @discardableResult
    private func editWorkspace(_ intent: WorkspaceIntent) async -> Bool {
        do { try await commit(intent); return true }
        catch SdkError.Conflict { return false }
        catch { problem = Self.describe(error); return false }
    }

    private func mark(_ id: String, hidden: Bool?, pinned: Bool?) async {
        await editWorkspace(.setAssignmentMark(assignmentId: id, hidden: hidden, pinned: pinned, completedAt: nil))
    }

    /// Pull to refresh and "Refresh now": asks Core's planner for everything;
    /// it orders the sources and reports progress in the sync snapshot.
    func refreshFromSchool() {
        problem = nil
        syncHint(.refresh(scope: .everything))
    }

    /// Asks Core for a newer App Store release when a check is due (the
    /// device's `appUpdateChecks`), or now (`manual`). Core decides whether a
    /// check of its own offers the release over the page; either way it shows
    /// in Settings › About with the App Store beside it. When the last check
    /// ran is this device's wake bookkeeping, as on the desktop.
    /// A launch always looks: each foregrounding within 12 hours of the last
    /// check does not, but a cold start is rare enough on a phone that a
    /// release dismissed earlier is offered again.
    func checkForUpdate(manual: Bool = false, launch: Bool = false) async {
        guard let session, !checkingUpdate else { return }
        checkingUpdate = true
        defer { checkingUpdate = false }
        let defaults = UserDefaults.standard
        let running = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
        guard
            case let .appUpdate(checked, release, prompt) = try? await session.checkAppUpdate(
                running: running, lastChecked: launch ? nil : defaults.string(forKey: Self.lastUpdateCheck), manual: manual
            ).value(),
            checked
        else { return }
        defaults.set(Date().formatted(.iso8601), forKey: Self.lastUpdateCheck)
        availableUpdate = release
        upToDate = manual && release == nil
        if prompt { updateOffer = release }
    }

    /// This device stops offering `version`; Core skips it in later checks.
    func skipUpdate(_ version: String) async {
        updateOffer = nil
        await updateSettings { $0["skippedAppVersion"] = version }
    }

    private static let lastUpdateCheck = "lastAppUpdateCheck"

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
        var settings = records.settings
        edit(&settings)
        guard let data = try? JSONSerialization.data(withJSONObject: settings),
            let json = String(data: data, encoding: .utf8)
        else { return }
        await editWorkspace(.replaceSettings(settings: json))
    }

    /// Whether this build can read a Matrix card (arm64 devices; not the Intel simulator).
    var canScanMatrix: Bool {
        (try? session?.capabilities().ocr) ?? false
    }

    // MARK: Formatting helpers

    nonisolated static func describe(_ error: Error) -> String {
        // A failure Core raised carries its stable code as `detail`; its copy
        // is the shared catalog's `error.<code>`. Other details are never shown.
        if let code = coreCode(error) {
            if let copy = lookup("error.\(code)") { return copy }
        }
        switch error {
        case SdkError.Network: return t("failure.youreOfflineShowingSavedData")
        case SdkError.Authentication: return t("failure.signInToYourSchool")
        case SdkError.Maintenance: return t("failure.theSchoolSiteIsUnder")
        case SdkError.Timeout: return t("failure.theSchoolSiteIsNot")
        case SdkError.Busy: return t("failure.thePreviousTaskHasNot")
        case SdkError.Unavailable: return t("failure.thisIsNotAvailableRight")
        case is SdkError: return t("error.internal")
        default: return error.localizedDescription
        }
    }

    nonisolated private static func coreCode(_ error: Error) -> String? {
        switch error {
        case let SdkError.InvalidInput(detail), let SdkError.Conflict(detail),
            let SdkError.Storage(detail), let SdkError.Compatibility(detail),
            let SdkError.Authentication(detail), let SdkError.Maintenance(detail),
            let SdkError.Network(detail), let SdkError.Timeout(detail),
            let SdkError.Unavailable(detail), let SdkError.Busy(detail),
            let SdkError.Callback(detail), let SdkError.Internal(detail):
            return detail
        default: return nil
        }
    }

    nonisolated static func timestamp(_ date: Date, fractional: Bool = false) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.timeZone = .tokyo
        formatter.formatOptions = fractional ? [.withInternetDateTime, .withFractionalSeconds] : [.withInternetDateTime]
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
        do {
            defer { timetableWeeks.invalidate(discard: true) }
            _ = try await session.reset(request: request)
        }
        problem = nil
        await refreshAccounts()
        await refreshSyncSnapshot()
        await backup.refresh()
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
        try? await commit(intent)
    }

    /// Marks one session (`Occurrence.key`) as `exam`, `quiz`, `cancelled` or
    /// `class` (a plain class; Core clears the mark when that is the default).
    /// The commit names `calendar`, which re-queries the week.
    func markSession(_ key: String, _ mark: String) async {
        // Each mark uses the workspace revision returned by the preceding commit.
        let previous = pendingSessionMark
        let markTask = Task {
            await previous?.value
            await editWorkspace(.markSession(key: key, mark: mark))
        }
        pendingSessionMark = markTask
        await markTask.value
    }
}

// MARK: Course page: lesson marks

extension AppModel {
    /// Records `sectionId` as the lesson of the class at `occurrence` (Core's
    /// `CourseContent.classAt`), as a double click on the desktop rail does.
    /// `false` when Core did not take it.
    func recordLesson(courseId: String, occurrence: String, sectionId: UInt64) async -> Bool {
        await editWorkspace(.recordLesson(courseId: courseId, occurrence: occurrence, sectionId: sectionId))
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
            case let .schedule(result)? = try? await session.scheduleQuery(queries: [
                .records(fields: ["academicProfile"])
            ]).value(),
            case let .records(json)? = result.values.first,
            let root = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any]
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
        try await commit(.setStudyProfile(admissionYear: profile.admissionYear, studyEnrollments: profile.enrollments))
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
