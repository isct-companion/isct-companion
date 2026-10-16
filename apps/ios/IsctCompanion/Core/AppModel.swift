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
    private(set) var pulls: [PullSnapshot] = []
    private(set) var lastPull: Date?
    /// Undo for the last hide, built from the intent Core committed.
    private(set) var undo: UndoAction?
    /// One "needs you" banner (sync feedback level 3).
    private(set) var problem: String?
    /// Bumped whenever Core reports new content, so open screens re-query.
    private(set) var contentRevision = ""

    let browser = SchoolBrowserHost()
    let web: WebRouter
    private var session: Session?
    private var revision = ""
    private var boundaryTask: Task<Void, Never>?
    private var refreshTask: Task<Void, Never>?
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

    var isPulling: Bool {
        pulls.contains { [.running, .applying, .yielding, .checking].contains($0.state) }
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
                notifications: nil, downloads: nil)
            self.session = session
            web.session = session
            let stream = try session.subscribe()
            Task { await self.listen(stream) }
            await refresh()
            await refreshAccounts()
            status = .ready
            watchNetwork()
            report(foreground: true)
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
            Task { await refresh() }
        case .background:
            foreground = false
            report(foreground: false)
            _ = try? session.suspend()
        default:
            break
        }
    }

    private func report(foreground: Bool) {
        guard let session else { return }
        _ = try? session.lifecycle(
            foreground: foreground, network: networkState, schoolVisible: web.isVisible)
        _ = try? session.schoolLifecycle(
            event: SchoolSessionLifecycle(kind: foreground ? .focus : .launch, visible: foreground))
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
                    case "pull-state-changed", "school-pull-progress":
                        await refreshPulls()
                    case "school-session", "school-login-status":
                        await refreshAccounts()
                    case "school-web-open":
                        web.handleOpenEvent(event.payload)
                    case "school-web-url", "school-download", "ocr-progress":
                        break
                    default:
                        scheduleRefresh()
                    }
                case .snapshotRequired:
                    scheduleRefresh()
                    await refreshAccounts()
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

    private func refreshPulls() async {
        guard let session else { return }
        if case let .pulls(snapshots)? = try? await session.pullState().value() {
            pulls = snapshots
            if snapshots.contains(where: { $0.state == .succeeded }) { lastPull = Date() }
            if let failed = snapshots.first(where: { [.failed, .blocked].contains($0.state) }) {
                problem = failed.lastError
            }
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
        await mark(item.assignmentId, hidden: hidden, pinned: nil)
        undo = hidden ? UndoAction(title: item.title, assignmentId: item.assignmentId) : nil
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

    /// Manual pull: LMS assignments and materials, then 教務Web.
    func pull() async {
        guard let session else { return }
        problem = nil
        for site in [SchoolSite.scienceLms, .kyomu] {
            do {
                if case let .schoolPull(outcomes) = try await session.pullSchool(
                    site: site, courseId: nil, loginFirst: true
                ).value(), let failed = outcomes.first(where: { $0.state == .reconnectRequired || $0.state == .error }) {
                    problem = failed.state == .reconnectRequired
                        ? Self.describe(SdkError.Authentication(detail: ""))
                        : failed.message
                    break
                }
            } catch {
                problem = Self.describe(error)
                break
            }
        }
        lastPull = Date()
        await refresh()
    }

    func saveCredentials(_ profile: SchoolAuthProfile, _ credentials: SchoolCredentials) async throws {
        guard let session else { return }
        _ = try await session.saveCredentials(profile: profile, credentials: credentials).value()
        await refreshAccounts()
        _ = try? session.ensureSchoolSession(profile: profile)
    }

    /// Edits `settings.notifications` and replaces the settings object.
    func setNotifications(_ edit: (inout [String: Any]) -> Void) async {
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
            _ = try await session.mutate(mutation: mutation).value()
        } catch {
            problem = Self.describe(error)
        }
        await refresh()
        await NotificationScheduler.sync(session)
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

    nonisolated static func date(_ raw: String) -> Date? {
        ISO8601DateFormatter().date(from: raw)
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
