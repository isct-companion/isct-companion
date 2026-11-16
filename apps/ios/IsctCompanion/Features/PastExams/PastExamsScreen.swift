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

    private(set) var busy = false
    var problem: String?
    var notice: String?
    var confirmation: Confirmation?
    private(set) var choices: ExamChoices?

    func call(_ model: AppModel, _ action: ExamAction) async -> ExamAnswer? {
        guard !busy else { return nil }
        busy = true
        problem = nil
        notice = nil
        defer { busy = false }
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
    /// A pushed past-exams page's failure or notice above the bottom edge,
    /// its confirmations and its running call in the bar.
    func examPageFeedback(_ page: ExamPage) -> some View {
        modifier(ExamPageFeedback(page: page))
    }

    func examConfirmations(_ page: ExamPage) -> some View {
        confirmationDialog(
            page.confirmation.map { t($0.key) } ?? "",
            isPresented: Binding(get: { page.confirmation != nil }, set: { if !$0 { page.confirmation = nil } }),
            titleVisibility: .visible,
            presenting: page.confirmation
        ) { confirmed in
            Button(t("common.ok"), role: .destructive) { Task { await confirmed.work() } }
        }
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
                        // A notice goes by itself; a failure stays until tapped.
                        .task(id: message) {
                            guard page.problem == nil else { return }
                            try? await Task.sleep(for: .seconds(4))
                            if page.notice == message { page.notice = nil }
                        }
                }
            }
            .toolbar {
                if page.busy {
                    ToolbarItem(placement: .topBarTrailing) { ProgressView().controlSize(.small) }
                }
            }
            .examConfirmations(page)
    }
}

/// ScienceTokyoExam: the account, then search and this quarter's courses,
/// requests and the person's library, each a list that pushes its subject,
/// request or document. Opened from a course, it is that course's subject.
struct PastExamsScreen: View {
    let courseId: String?
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.push) private var push
    @State private var page = ExamPage()
    @State private var tab: Tab = .browse
    @State private var query = ""
    @State private var subjects: [ExamSubject]?
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

    private var root: some View {
        List {
            Section {
                if let email = account?.email { Text(email).foregroundStyle(Theme.muted) }
                if let problem = page.problem, account?.status != .unsupported { Text(problem).foregroundStyle(.red) }
                if let notice = page.notice { Text(notice).foregroundStyle(Theme.muted) }
                switch account?.status {
                case .connected:
                    if account?.upload.phase == .pending {
                        Button(t("examShare.upload.recover"), systemImage: "arrow.clockwise") {
                            uploadEntry = .recovery
                        }
                    }
                    Button(t("examShare.disconnect"), role: .destructive) {
                        page.confirm("examShare.disconnectHint") { await accountAction(.disconnect) }
                    }
                case .needsConsent:
                    if let terms = URL(string: "https://sciencetokyoexam.com/terms") {
                        Link(t("examShare.terms"), destination: terms)
                    }
                    if let privacy = URL(string: "https://sciencetokyoexam.com/privacy") {
                        Link(t("examShare.privacy"), destination: privacy)
                    }
                    Button(t("examShare.accept")) { Task { await accountAction(.acceptTerms) } }
                case .unsupported:
                    Text(t("error.exam-share-unsupported"))
                case .disconnected, .none:
                    Button(t("examShare.connect")) { Task { await accountAction(.connect) } }
                }
            }
            // From a course, only the account shows until it connects.
            if account?.status != .unsupported && courseId == nil {
                Section {
                    Picker(t("examShare.title"), selection: $tab) {
                        ForEach(Tab.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }
                if account?.status == .connected {
                    switch tab {
                    case .browse: browse
                    case .requests: requestBoard
                    case .library: librarySections
                    }
                }
                Section {} footer: { ExamCredit() }
            }
        }
        .themedRows()
        .themedList()
        .modifier(PastExamsTitle(root: courseId == nil, busy: page.busy))
        .disabled(page.busy)
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
        .refreshable { await refresh() }
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
            // Nothing below the account section shows without a connection.
            subjects = nil
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
        .examConfirmations(page)
    }

    @ViewBuilder private var browse: some View {
        Section {
            TextField(t("examShare.search"), text: $query)
                .submitLabel(.search)
                .onSubmit { Task { await search() } }
            Button(t("examShare.search")) { Task { await search() } }
                .disabled(query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        if let subjects {
            ExamSubjectResults(subjects: subjects)
        }
        CurrentCoursesSection(list: courses, failed: coursesFailed, retry: { coursesRetry += 1 }) { course in
            push(ExamSubjectRoute(source: .course(id: course.courseId), title: course.title))
        }
    }

    @ViewBuilder private var requestBoard: some View {
        let requests = requestPages[mine]
        Section {
            Picker(t("examShare.requests"), selection: $mine) {
                Text(t("examShare.openRequests")).tag(false)
                Text(t("examShare.myRequests")).tag(true)
            }
            .pickerStyle(.segmented)
            .onChange(of: mine) { Task { await loadVisible(reload: false) } }
            Button(t("examShare.newRequest")) { tab = .browse }
            if let web = requests.flatMap({ URL(string: $0.webUrl) }) {
                Link(t("examShare.openOnWeb"), destination: web)
            }
            if requests?.items.isEmpty == true { Text(t("examShare.noItems")) }
            ForEach(requests?.items ?? [], id: \.id) { item in
                NavigationLink(value: ExamRequestRoute(request: item, webUrl: requests?.webUrl ?? "")) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(item.subject.name).foregroundStyle(Theme.ink)
                        Text(examRequestFacts(item)).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(2)
                    }
                }
            }
            if let cursor = requests?.cursor {
                Button(t("examShare.more")) { Task { await loadRequests(cursor) } }
            }
        }
    }

    @ViewBuilder private var librarySections: some View {
        if let library {
            if let web = URL(string: library.webUrl), !library.offline {
                Section { Link(t("examShare.openOnWeb"), destination: web) }
            }
            if library.offline {
                Section { Text(t("examShare.libraryUnavailable")).foregroundStyle(Theme.muted) }
            } else {
                Section(t("examShare.saved")) { documents(library.saved) }
            }
            Section(t("examShare.downloaded")) { documents(library.downloaded, removal: .download) }
            if !library.offline {
                Section(t("examShare.submitted")) { documents(library.submitted, removal: .submission) }
            }
        }
    }

    @ViewBuilder private func documents(_ items: [ExamDocument], removal: ExamRemoval? = nil) -> some View {
        if items.isEmpty { Text(t("examShare.noItems")) }
        ForEach(items, id: \.id) { item in
            NavigationLink(examName(item), value: ExamDocumentRoute(id: item.id, removal: removal))
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

    private func search() async {
        if case let .subjects(results)? = await page.call(model, .search(query: query)) { subjects = results }
    }

    private func loadRequests(_ cursor: ExamRequestCursor? = nil) async {
        let mine = mine
        if case let .requests(value)? = await page.call(model, .requests(mine: mine, cursor: cursor)) {
            let previous = cursor == nil ? [] : requestPages[mine]?.items ?? []
            requestPages[mine] = ExamRequestPage(items: previous + value.items, cursor: value.cursor, webUrl: value.webUrl)
        }
    }
}

/// Subjects a search or a course found, each pushing its page.
struct ExamSubjectResults: View {
    let subjects: [ExamSubject]

    var body: some View {
        Section {
            if subjects.isEmpty { Text(t("examShare.noItems")) }
            ForEach(subjects, id: \.code) { result in
                NavigationLink(value: ExamSubjectRoute(source: .subject(code: result.code, classId: nil), title: result.name)) {
                    VStack(alignment: .leading) {
                        Text(result.name)
                        Text(result.code).font(.footnote.monospaced()).foregroundStyle(Theme.muted)
                    }
                }
            }
        }
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

/// "2024 · Open · details", a request's facts in one line.
func examRequestFacts(_ item: ExamRequest) -> String {
    [item.academicYear.map(String.init), t("examShare.requestStatus.\(item.status)"), item.description]
        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
}

/// "2024 · Final 2 · B", a sitting's label.
func examSitting(year: UInt32?, occasion: String?, number: UInt32, variant: String) -> String {
    let occasion = occasion.map { value in
        let label = t("examShare.occasion.\(value)")
        return number > 1 ? "\(label) \(number)" : label
    }
    return [year.map(String.init), occasion, variant]
        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
}

func examName(_ exam: ExamDocument) -> String {
    let occasion = exam.occasion.map { value in
        let label = t("examShare.occasion.\(value)")
        return exam.occasionNumber > 1 ? "\(label) \(exam.occasionNumber)" : label
    }
    let fields = [
        exam.subjects.map(\.name).joined(separator: " / "),
        exam.academicYear.map(String.init),
        occasion,
        exam.variant,
        exam.role.map { t("examShare.role.\($0)") },
        exam.teachers.map(\.name).joined(separator: " / "),
        exam.note,
    ].compactMap { $0 }.filter { !$0.isEmpty }
    return fields.isEmpty ? exam.fileName : fields.joined(separator: " · ")
}

struct ExamRequestForm: View {
    @Environment(AppModel.self) private var model
    let subject: ExamSubjectPage
    let choices: ExamChoices
    let busy: Bool
    let problem: String?
    let submit: (ExamRequestDraft) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var classId: UInt32?
    @State private var year = ""
    @State private var occasion: ExamOccasion?
    @State private var role: ExamRole?
    @State private var description = ""

    var body: some View {
        NavigationStack {
            Form {
                Section(subject.subject.name) {
                    Picker(t("examShare.allClasses"), selection: $classId) {
                        Text(t("examShare.allClasses")).tag(nil as UInt32?)
                        ForEach(subject.rows, id: \.classIds) { row in
                            if let id = row.classIds.first {
                                Text((row.labels + row.teachers.map(\.name) + [row.taughtIn]).joined(separator: " · ")).tag(Optional(id))
                            }
                        }
                    }
                    TextField(t("examShare.year"), text: $year).keyboardType(.numberPad)
                    Picker(t("examShare.occasion"), selection: $occasion) {
                        Text(t("examShare.allOccasions")).tag(nil as ExamOccasion?)
                        ForEach(choices.requestOccasions, id: \.self) { Text($0.label).tag(Optional($0)) }
                    }
                    Picker(t("examShare.wantedRole"), selection: $role) {
                        Text(t("examShare.wantedRole")).tag(nil as ExamRole?)
                        ForEach(choices.requestRoles, id: \.self) { Text($0.label).tag(Optional($0)) }
                    }
                    TextField(t("examShare.description"), text: $description, axis: .vertical)
                    if let problem { Text(problem).foregroundStyle(.red) }
                }
            }
            .navigationTitle(t("examShare.newRequest"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("examShare.newRequest")) {
                        let draft = ExamRequestDraft(subjectCode: subject.subject.code, classId: classId,
                            academicYear: UInt32(year), occasion: occasion, wantedRole: role, description: description)
                        Task { await submit(draft) }
                    }.disabled(busy || description.utf16.count > choices.limits.requestDescription
                        || (!year.isEmpty && UInt32(year) == nil))
                }
            }
        }
    }
}

struct ExamReportForm: View {
    let reasons: [ExamReportReason]
    let maxDetail: UInt32
    let busy: Bool
    let problem: String?
    let submit: (ExamReportReason, String) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var reason: ExamReportReason
    @State private var detail = ""

    init(reasons: [ExamReportReason], maxDetail: UInt32, busy: Bool, problem: String?,
         submit: @escaping (ExamReportReason, String) async -> Void) {
        self.reasons = reasons
        self.maxDetail = maxDetail
        self.busy = busy
        self.problem = problem
        self.submit = submit
        _reason = State(initialValue: reasons[0])
    }

    var body: some View {
        NavigationStack {
            Form {
                Picker(t("examShare.report"), selection: $reason) {
                    ForEach(reasons, id: \.self) { Text($0.label).tag($0) }
                }
                TextField(t("examShare.description"), text: $detail, axis: .vertical)
                if let problem { Text(problem).foregroundStyle(.red) }
            }
            .navigationTitle(t("examShare.report"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("examShare.report")) { Task { await submit(reason, detail) } }
                        .disabled(busy || detail.utf16.count > maxDetail)
                }
            }
        }
    }
}

// Labels for the values Core's `ExamChoices` offers. Each case names its
// key, so the locale checker sees every one.

extension ExamRole {
    var label: String {
        switch self {
        case .question: t("examShare.role.question")
        case .answer: t("examShare.role.answer")
        case .both: t("examShare.role.both")
        case .notes: t("examShare.role.notes")
        case .other: t("examShare.role.other")
        }
    }
}

extension ExamOccasion {
    var label: String {
        switch self {
        case .unknown: t("examShare.occasion.unknown")
        case .final: t("examShare.occasion.final")
        case .midterm: t("examShare.occasion.midterm")
        case .makeup: t("examShare.occasion.makeup")
        case .quiz: t("examShare.occasion.quiz")
        case .check: t("examShare.occasion.check")
        case .report: t("examShare.occasion.report")
        case .exercise: t("examShare.occasion.exercise")
        case .textbook: t("examShare.occasion.textbook")
        case .mock: t("examShare.occasion.mock")
        case .notApplicable: t("examShare.occasion.none")
        }
    }
}

extension ExamProvenance {
    var label: String {
        switch self {
        case .original: t("examShare.upload.original")
        case .reconstructed: t("examShare.upload.reconstructed")
        }
    }
}

extension ExamAnswerSource {
    var label: String {
        switch self {
        case .unknown: t("examShare.upload.unknown")
        case .official: t("examShare.upload.official")
        case .student: t("examShare.upload.student")
        }
    }
}

extension ExamReportReason {
    var label: String {
        switch self {
        case .copyright: t("examShare.reason.copyright")
        case .inappropriate: t("examShare.reason.inappropriate")
        case .wrongAnswer: t("examShare.reason.wrongAnswer")
        case .brokenFile: t("examShare.reason.brokenFile")
        case .other: t("examShare.reason.other")
        }
    }
}

/// The 過去問 tab's title sits at the content's gutter, as the other tabs'
/// do; opened from a course it is a pushed page with the bar's title.
private struct PastExamsTitle: ViewModifier {
    let root: Bool
    let busy: Bool

    func body(content: Content) -> some View {
        if root {
            VStack(spacing: 0) {
                PageHeader(title: t("examShare.title")) {
                    if busy { ProgressView().controlSize(.small) }
                }
                content.pageListMargins()
            }
            .toolbar(.hidden, for: .navigationBar)
        } else {
            content
                .navigationTitle(t("examShare.title"))
                .toolbar {
                    if busy {
                        ToolbarItem(placement: .topBarTrailing) { ProgressView().controlSize(.small) }
                    }
                }
        }
    }
}
