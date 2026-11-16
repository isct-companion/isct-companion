import IsctCore
import SwiftUI

struct PastExamsRoute: Hashable {
    let courseId: String?

    init(courseId: String? = nil) { self.courseId = courseId }
}

/// A subject's page: one subject by code, or what a local course resolves to.
struct ExamSubjectRoute: Hashable {
    enum Source: Hashable {
        case course(id: String)
        case subject(code: String, classId: UInt32?)
    }

    let source: Source
    /// Shown in the bar until the page arrives.
    var title: String?

    var action: ExamAction {
        switch source {
        case let .course(id): .courseSubject(courseId: id)
        case let .subject(code, classId): .subject(code: code, classId: classId)
        }
    }
}

/// What a document opened from the library can also remove.
enum ExamRemoval: Hashable {
    case download, submission

    var key: String {
        switch self {
        case .download: "examShare.removeDownload"
        case .submission: "examShare.deleteSubmission"
        }
    }

    func action(_ id: String) -> ExamAction {
        switch self {
        case .download: .removeDownload(id: id)
        case .submission: .deleteSubmission(id: id)
        }
    }
}

/// One document. `file` is the copy the opening page already read, so the
/// pushed page shows it at once.
struct ExamDocumentRoute: Hashable {
    let id: String
    var file: ExamLocalDocument?
    var removal: ExamRemoval?

    static func == (a: Self, b: Self) -> Bool { a.id == b.id && a.removal == b.removal }
    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(removal)
    }
}

struct ExamRequestRoute: Hashable {
    let request: ExamRequest
    /// The board's page on the site.
    let webUrl: String

    static func == (a: Self, b: Self) -> Bool { a.request.id == b.request.id }
    func hash(into hasher: inout Hasher) { hasher.combine(request.id) }
}

/// The site's pages Core has no answer for: the search before anything is
/// read, and the terms shown before consent.
enum ExamSite {
    static let base = "https://sciencetokyoexam.com"
    static let search = URL(string: "\(base)/search")
    static let terms = URL(string: "\(base)/terms")
    static let privacy = URL(string: "\(base)/privacy")
}

/// The one Core call path each past-exams page owns: whether a call is
/// running, its failure or notice, a pending confirmation and the forms'
/// choices.
@MainActor
@Observable
final class ExamPage {
    struct Confirmation {
        let key: String
        let work: () async -> Void
    }

    private var running = 0
    var problem: String?
    var notice: String?
    var confirmation: Confirmation?
    private(set) var choices: ExamChoices?

    var busy: Bool { running > 0 }

    /// Runs one action; a failure shows its message. Calls may overlap (a
    /// search while a list loads), so `busy` counts them.
    func call(_ model: AppModel, _ action: ExamAction) async -> ExamAnswer? {
        running += 1
        problem = nil
        defer { running -= 1 }
        do { return try await model.examShare(action) }
        catch SdkError.Cancelled { return nil }
        catch is CancellationError { return nil }
        catch {
            problem = AppModel.describe(error)
            return nil
        }
    }

    /// The forms' choices, answered without the network.
    func loadChoices(_ model: AppModel) async {
        guard choices == nil, case let .choices(value)? = try? await model.examShare(.choices) else { return }
        choices = value
    }

    func confirm(_ key: String, work: @escaping () async -> Void) {
        confirmation = Confirmation(key: key, work: work)
    }
}

extension View {
    /// A past-exams page's failure or notice above the bottom edge, and its
    /// confirmations.
    func examPageFeedback(_ page: ExamPage) -> some View {
        modifier(ExamPageFeedback(page: page))
    }
}

private struct ExamPageFeedback: ViewModifier {
    let page: ExamPage

    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .bottom) {
                if let message = page.problem ?? page.notice {
                    Text(message)
                        .font(.subheadline)
                        .foregroundStyle(page.problem == nil ? Theme.ink : Theme.danger)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
                        .padding(.horizontal, Theme.gutter)
                        .padding(.bottom, 8)
                        .onTapGesture { page.problem = nil; page.notice = nil }
                        .task(id: message) {
                            try? await Task.sleep(for: .seconds(4))
                            guard !Task.isCancelled else { return }
                            if page.problem == message { page.problem = nil }
                            if page.notice == message { page.notice = nil }
                        }
                }
            }
            .confirmationDialog(
                page.confirmation.map { t($0.key) } ?? "",
                isPresented: Binding(get: { page.confirmation != nil }, set: { if !$0 { page.confirmation = nil } }),
                titleVisibility: .visible,
                presenting: page.confirmation
            ) { confirmed in
                Button(t("common.ok"), role: .destructive) { Task { await confirmed.work() } }
            }
    }
}

/// ScienceTokyoExam: this quarter's courses (or a search), requests and the
/// person's own documents, each a list that pushes its subject, request or
/// document. Opened from a course, it is that course's subject.
struct PastExamsScreen: View {
    let courseId: String?
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.push) private var push
    @State private var page = ExamPage()
    @State private var tab: Tab = .browse
    @State private var query = ""
    @State private var hits: (query: String, subjects: [ExamSubject])?
    @State private var library: ExamLibrary?
    @State private var requestPages: [Bool: ExamRequestPage] = [:]
    @State private var mine = false
    @State private var courses: ExamCourseList?
    @State private var coursesFailed = false
    @State private var coursesRetry = 0
    @State private var uploadEntry: ExamUploadEntry?

    init(courseId: String? = nil) { self.courseId = courseId }

    private var account: ExamAccount? { model.examAccount }

    private enum Tab: CaseIterable {
        case browse, requests, library

        var label: String {
            switch self {
            case .browse: t("examShare.browse")
            case .requests: t("examShare.requests")
            case .library: t("examShare.library")
            }
        }
    }
    private struct CoursesRead: Equatable {
        let revision: String
        let retry: Int
    }

    var body: some View {
        if let courseId, account?.status == .connected {
            ExamSubjectScreen(route: ExamSubjectRoute(source: .course(id: courseId)))
        } else {
            root
        }
    }

    /// The site's page for the tab on screen.
    private var webUrl: URL? {
        switch tab {
        case .browse: ExamSite.search
        case .requests: requestPages[mine].flatMap { URL(string: $0.webUrl) }
        case .library: library.flatMap { URL(string: $0.webUrl) }
        }
    }

    private var root: some View {
        Group {
            switch account?.status {
            case .connected where courseId == nil:
                connected
            case nil where page.problem == nil:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            default:
                ExamAccountGate(page: page, status: account?.status ?? .disconnected) { action in
                    await accountAction(action)
                }
            }
        }
        .frame(maxHeight: .infinity)
        .background(Theme.bg)
        .modifier(PastExamsTitle(root: courseId == nil, web: account?.status == .connected ? webUrl : nil))
        .examLinks()
        .task(id: CoursesRead(revision: model.contentRevision, retry: coursesRetry)) {
            guard courseId == nil else { return }
            coursesFailed = false
            let answer = await model.examCourses()
            guard !Task.isCancelled else { return }
            courses = answer
            coursesFailed = answer == nil
        }
        .task { await page.loadChoices(model) }
        .task(id: scenePhase) {
            guard scenePhase == .active else { return }
            await refresh()
        }
        // Back from a pushed page that may have changed the board or the
        // library: read the one on screen again.
        .onAppear { Task { await loadVisible(reload: true, onlyLoaded: true) } }
        .onChange(of: tab) {
            page.problem = nil
            page.notice = nil
            Task { await loadVisible(reload: false) }
        }
        .onChange(of: account?.status) { _, status in
            guard status != .connected else { return }
            // Nothing past the account shows without a connection.
            hits = nil
            requestPages.removeAll()
            library = nil
            uploadEntry = nil
            if status == .unsupported { page.confirmation = nil }
        }
        .sheet(item: $uploadEntry) { entry in
            if let choices = page.choices {
                ExamUploadSheet(entry: entry, choices: choices) { outcome in
                    requestPages.removeAll()
                    library = nil
                    await loadVisible(reload: true)
                    page.notice = outcome.notice
                    if let id = outcome.documentId { push(ExamDocumentRoute(id: id)) }
                }
            }
        }
        .examPageFeedback(page)
    }

    private var connected: some View {
        VStack(spacing: 0) {
            Picker(t("examShare.title"), selection: $tab) {
                ForEach(Tab.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, Theme.gutter)
            .padding(.vertical, 8)
            switch tab {
            case .browse: browse
            case .requests: requestBoard
            case .library: librarySections
            }
        }
    }

    // MARK: Browse

    private var browse: some View {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return List {
            Section {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass").foregroundStyle(Theme.muted)
                    TextField(t("examShare.search"), text: $query)
                        .submitLabel(.search)
                        .autocorrectionDisabled()
                    if !query.isEmpty {
                        Button { query = "" } label: {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.muted)
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(t("common.clear"))
                    }
                }
            }
            if !trimmed.isEmpty {
                if let hits, hits.query == trimmed {
                    ExamSubjectResults(subjects: hits.subjects)
                }
            } else {
                CurrentCoursesSection(list: courses, failed: coursesFailed) { coursesRetry += 1 }
            }
            Section {} footer: { ExamCredit() }
        }
        .themedRows()
        .themedList()
        .pageListMargins()
        .refreshable { await refresh() }
        // Results follow the typing, a moment after it stops.
        .task(id: trimmed) {
            guard !trimmed.isEmpty else { return }
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            if case let .subjects(found)? = await page.call(model, .search(query: trimmed)), !Task.isCancelled {
                hits = (trimmed, found)
            }
        }
    }

    // MARK: Requests

    private var requestBoard: some View {
        let requests = requestPages[mine]
        return List {
            Section {
                Picker(t("examShare.requests"), selection: $mine) {
                    Text(t("examShare.openRequests")).tag(false)
                    Text(t("examShare.myRequests")).tag(true)
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
                .onChange(of: mine) { Task { await loadVisible(reload: false) } }
            }
            if let requests {
                Section {
                    ForEach(requests.items, id: \.id) { item in
                        NavigationLink(value: ExamRequestRoute(request: item, webUrl: requests.webUrl)) {
                            ExamRequestRow(item: item)
                        }
                    }
                    if let cursor = requests.cursor {
                        Button(t("examShare.more")) { Task { await loadRequests(cursor) } }
                    }
                }
                if requests.items.isEmpty {
                    Text(t("examShare.noItems")).foregroundStyle(Theme.muted)
                }
            }
        }
        .themedRows()
        .themedList()
        .pageListMargins()
        .overlay { if requests == nil && page.busy { ProgressView() } }
        .refreshable { await refresh() }
    }

    // MARK: Library

    private var librarySections: some View {
        List {
            if account?.upload.phase == .pending {
                Section {
                    Button(t("examShare.upload.recover"), systemImage: "arrow.clockwise") { uploadEntry = .recovery }
                } footer: {
                    Text(t("examShare.upload.pending"))
                }
            }
            if let library {
                if library.offline {
                    Text(t("examShare.libraryUnavailable")).foregroundStyle(Theme.muted)
                } else {
                    documents("examShare.saved", library.saved)
                }
                documents("examShare.downloaded", library.downloaded, removal: .download)
                if !library.offline {
                    documents("examShare.submitted", library.submitted, removal: .submission)
                }
                if !library.offline && library.saved.isEmpty && library.downloaded.isEmpty && library.submitted.isEmpty {
                    Text(t("examShare.noItems")).foregroundStyle(Theme.muted)
                }
            }
            // The account sits with the person's own things, out of the way.
            Section {
                if let email = account?.email {
                    LabeledContent(t("examShare.account"), value: email)
                }
                Button(t("examShare.disconnect"), role: .destructive) {
                    page.confirm("examShare.disconnectHint") { await accountAction(.disconnect) }
                }
            } footer: {
                ExamCredit()
            }
        }
        .themedRows()
        .themedList()
        .pageListMargins()
        .overlay { if library == nil && page.busy { ProgressView() } }
        .refreshable { await refresh() }
    }

    @ViewBuilder
    private func documents(_ title: String, _ items: [ExamDocument], removal: ExamRemoval? = nil) -> some View {
        if !items.isEmpty {
            Section(t(title)) {
                ForEach(items, id: \.id) { item in
                    NavigationLink(value: ExamDocumentRoute(id: item.id, removal: removal)) {
                        ExamListRow(title: item.subjects.map(\.name).joined(separator: " / "), meta: [
                            examSitting(year: item.academicYear, occasion: item.occasion,
                                        number: item.occasionNumber, variant: item.variant),
                            item.role.map { t("examShare.role.\($0)") },
                        ])
                    }
                    .swipeActions {
                        if let removal {
                            Button(t(removal.key), role: .destructive) {
                                page.confirm(removal.key) {
                                    if case let .library(value)? = await page.call(model, removal.action(item.id)) {
                                        library = value
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // MARK: Loading

    private func accountAction(_ action: ExamAction) async {
        guard await page.call(model, action) != nil else { return }
        await loadVisible(reload: false)
    }

    /// Scene activation and pull-to-refresh: the account, then the view on
    /// screen. Search results and this quarter's courses are not read again.
    private func refresh() async {
        guard await page.call(model, .account) != nil else { return }
        await loadVisible(reload: true)
    }

    /// The tab's view, read the first time it shows and kept across tab
    /// switches; `reload` reads it again, `onlyLoaded` only one already read.
    private func loadVisible(reload: Bool, onlyLoaded: Bool = false) async {
        guard account?.status == .connected, courseId == nil else { return }
        switch tab {
        case .browse:
            return
        case .requests:
            let loaded = requestPages[mine] != nil
            if onlyLoaded && !loaded { return }
            if reload || !loaded { await loadRequests() }
        case .library:
            if onlyLoaded && library == nil { return }
            if reload || library == nil, case let .library(value)? = await page.call(model, .library) {
                library = value
            }
        }
    }

    private func loadRequests(_ cursor: ExamRequestCursor? = nil) async {
        let mine = mine
        if case let .requests(value)? = await page.call(model, .requests(mine: mine, cursor: cursor)) {
            let previous = cursor == nil ? [] : requestPages[mine]?.items ?? []
            requestPages[mine] = ExamRequestPage(items: previous + value.items, cursor: value.cursor, webUrl: value.webUrl)
        }
    }
}

/// Before a usable session: sign in, the terms, or why not.
private struct ExamAccountGate: View {
    @Environment(AppModel.self) private var model
    let page: ExamPage
    let status: ExamAccountStatus
    let act: (ExamAction) async -> Void

    var body: some View {
        ContentUnavailableView {
            Label(t("examShare.title"), systemImage: "doc.on.doc")
        } description: {
            switch status {
            case .needsConsent: Text(t("error.exam-share-consent"))
            case .unsupported: Text(t("error.exam-share-unsupported"))
            case .disconnected, .connected:
                Text(page.busy ? t("common.signingIn") : t("examShare.connectHint"))
            }
        } actions: {
            switch status {
            case .needsConsent:
                Button(t("examShare.accept")) { Task { await act(.acceptTerms) } }
                    .buttonStyle(.borderedProminent)
                    .disabled(page.busy)
                HStack(spacing: 16) {
                    if let terms = ExamSite.terms { Link(t("examShare.terms"), destination: terms) }
                    if let privacy = ExamSite.privacy { Link(t("examShare.privacy"), destination: privacy) }
                }
                .font(.subheadline)
            case .unsupported:
                EmptyView()
            case .disconnected, .connected:
                if page.busy {
                    // Sign-in waits on the person in the browser; they may leave it.
                    Button(t("common.cancel")) { Task { _ = try? await model.examShare(.cancelConnect) } }
                } else {
                    Button(t("examShare.connect")) { Task { await act(.connect) } }
                        .buttonStyle(.borderedProminent)
                }
            }
            ExamCredit().frame(width: 220)
        }
    }
}

/// A list row: the title, then its facts in one muted line.
struct ExamListRow: View {
    let title: String
    let meta: [String?]

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).foregroundStyle(Theme.ink)
            let detail = meta.compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
            if !detail.isEmpty { Text(detail).font(.footnote).foregroundStyle(Theme.muted) }
        }
    }
}

/// Subjects a search or a course found, each pushing its page.
struct ExamSubjectResults: View {
    let subjects: [ExamSubject]

    var body: some View {
        Section {
            ForEach(subjects, id: \.code) { result in
                NavigationLink(value: ExamSubjectRoute(source: .subject(code: result.code, classId: nil), title: result.name)) {
                    ExamListRow(title: result.name, meta: [result.code])
                }
            }
        }
        if subjects.isEmpty { Text(t("examShare.noItems")).foregroundStyle(Theme.muted) }
    }
}

struct ExamRequestRow: View {
    let item: ExamRequest

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(item.subject.name).foregroundStyle(Theme.ink)
                if item.status != "open" { ExamStatusTag(status: item.status) }
            }
            let detail = [examRequestFacts(item), item.description].filter { !$0.isEmpty }.joined(separator: " · ")
            if !detail.isEmpty {
                Text(detail).font(.footnote).foregroundStyle(Theme.muted).lineLimit(2)
            }
        }
    }
}

struct ExamStatusTag: View {
    let status: String

    var body: some View {
        let tint = status == "fulfilled" ? Theme.ok : status == "open" ? Theme.accent : Theme.muted
        Text(t("examShare.requestStatus.\(status)"))
            .font(.caption2.weight(.medium))
            .foregroundStyle(tint)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(tint.opacity(0.12), in: Capsule())
    }
}

/// The ScienceTokyoExam credit, English only by design: it names the
/// service and its operator.
struct ExamCredit: View {
    var body: some View {
        Image("PastExams/credit")
            .resizable()
            .renderingMode(.template)
            .scaledToFit()
            .frame(height: 11)
            .foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .accessibilityLabel(Text(verbatim: "Supported by ScienceTokyoExam · teduryu"))
    }
}

/// "2024 · Final · Answers", a request's facts in one line.
func examRequestFacts(_ item: ExamRequest) -> String {
    [
        item.academicYear.map(String.init),
        item.occasion.map { t("examShare.occasion.\($0)") },
        item.wantedRole.map { t("examShare.role.\($0)") },
    ]
    .compactMap { $0 }.joined(separator: " · ")
}

/// "2024 Final 2 B", a sitting's label.
func examSitting(year: UInt32?, occasion: String?, number: UInt32, variant: String) -> String {
    let occasion = occasion.map { value in
        let label = t("examShare.occasion.\(value)")
        return number > 1 ? "\(label) \(number)" : label
    }
    return [year.map(String.init), occasion, variant]
        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
}

/// "Subject · 2024 Final · Questions", a document's title.
func examName(_ exam: ExamDocument) -> String {
    let fields = [
        exam.subjects.map(\.name).joined(separator: " / "),
        examSitting(year: exam.academicYear, occasion: exam.occasion, number: exam.occasionNumber, variant: exam.variant),
        exam.role.map { t("examShare.role.\($0)") },
    ].compactMap { $0 }.filter { !$0.isEmpty }
    return fields.isEmpty ? exam.fileName : fields.joined(separator: " · ")
}

/// The 過去問 tab's title sits at the content's gutter, as the other tabs'
/// do, with the site's page beside it; opened from a course it is a pushed
/// page with the bar's title.
private struct PastExamsTitle: ViewModifier {
    let root: Bool
    let web: URL?

    func body(content: Content) -> some View {
        if root {
            VStack(spacing: 0) {
                PageHeader(title: t("examShare.title")) {
                    if let web {
                        ExamLinkButton(url: web) {
                            Image(systemName: "safari").font(.title3)
                        }
                        .accessibilityLabel(t("examShare.openOnWeb"))
                    }
                }
                content
            }
            .toolbar(.hidden, for: .navigationBar)
        } else {
            content
                .navigationTitle(t("examShare.title"))
                .navigationBarTitleDisplayMode(.inline)
        }
    }
}
