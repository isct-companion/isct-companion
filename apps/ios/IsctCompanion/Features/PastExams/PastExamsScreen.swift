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

    static func == (a: Self, b: Self) -> Bool { a.request.id == b.request.id }
    func hash(into hasher: inout Hasher) { hasher.combine(request.id) }
}

/// The one Core call path each past-exams page owns: whether a call is
/// running, its failure, the website's
/// pages and the forms' choices.
@MainActor
@Observable
final class ExamPage {
    private var running = 0
    /// Signing in runs; only then does the sign-in gate show it.
    private(set) var connecting = false
    var problem: String?
    private(set) var site: ExamSitePages?
    private(set) var choices: ExamChoices?

    var busy: Bool { running > 0 }

    /// Runs one action; a failure shows its message. Calls may overlap (a
    /// search while a list loads), so `busy` counts them.
    func call(_ model: AppModel, _ action: ExamAction) async -> ExamAnswer? {
        let connect = if case .connect = action { true } else { false }
        running += 1
        if connect { connecting = true }
        problem = nil
        defer {
            running -= 1
            if connect { connecting = false }
        }
        do { return try await model.examShare(action) }
        catch SdkError.Cancelled { return nil }
        catch is CancellationError { return nil }
        catch {
            problem = AppModel.describe(error)
            return nil
        }
    }

    /// The website's pages, answered without the network or an account.
    func loadSite(_ model: AppModel) async {
        guard site == nil, case let .site(value)? = try? await model.examShare(.site) else { return }
        site = value
    }

    /// The forms' choices, read as a form opens (Core caches them for a few
    /// minutes); false when they did not arrive, the failure shown at the
    /// form's entry, and the next opening asks again.
    func loadChoices(_ model: AppModel) async -> Bool {
        guard case let .choices(value)? = await call(model, .choices) else { return false }
        choices = value
        return true
    }

    func confirm(_ key: String, notices: AppNotices, work: @escaping @MainActor () async -> Void) {
        notices.confirm(title: t(key), action: t("common.ok"), destructive: true) { Task { await work() } }
    }
}

extension View {
    /// A failure stays with the operation's page until another attempt or dismissal.
    func examPageFeedback(_ page: ExamPage) -> some View {
        safeAreaInset(edge: .top, spacing: 0) {
            if let problem = page.problem {
                ProblemBanner(text: problem, dismiss: { page.problem = nil })
                    .padding(.horizontal, Theme.gutter)
            }
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

    /// The site's page for the tab on screen; before connecting the tabs
    /// are hidden, so the search.
    private var webUrl: URL? {
        guard let site = page.site else { return nil }
        guard account?.status == .connected else { return URL(string: site.search) }
        return switch tab {
        case .browse: URL(string: site.search)
        case .requests: URL(string: site.requests)
        case .library: URL(string: site.library)
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
                ExamAccountGate(page: page, status: account?.status ?? .disconnected, site: page.site) { action in
                    await accountAction(action)
                }
            }
        }
        .frame(maxHeight: .infinity)
        .background(Theme.bg)
        .modifier(PastExamsTitle(root: courseId == nil, web: webUrl))
        .task(id: CoursesRead(revision: model.contentRevision, retry: coursesRetry)) {
            guard courseId == nil else { return }
            coursesFailed = false
            let answer = await model.examCourses()
            guard !Task.isCancelled else { return }
            courses = answer
            coursesFailed = answer == nil
        }
        .task { await page.loadSite(model) }
        .task(id: scenePhase) {
            guard scenePhase == .active else { return }
            await refresh()
        }
        // Back from a pushed page that may have changed the board or the
        // library: read the one on screen again.
        .onAppear { Task { await loadVisible(reload: true, onlyLoaded: true) } }
        .onChange(of: tab) {
            page.problem = nil
            Task { await loadVisible(reload: false) }
        }
        .onChange(of: account?.status) { _, status in
            guard status != .connected else { return }
            // Nothing past the account shows without a connection.
            hits = nil
            requestPages.removeAll()
            library = nil
            uploadEntry = nil
        }
        .sheet(item: $uploadEntry) { entry in
            ExamUploadSheet(entry: entry, choices: page.choices) { outcome in
                requestPages.removeAll()
                library = nil
                await loadVisible(reload: true)
                model.notices.show(outcome.notice)
                if let id = outcome.documentId { push(ExamDocumentRoute(id: id)) }
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
            .padding(.vertical, Theme.space4)
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
                HStack(spacing: Theme.space4) {
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
                        NavigationLink(value: ExamRequestRoute(request: item)) {
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
                    Button(t("examShare.upload.recover"), systemImage: "arrow.clockwise") {
                        // A recovery shows no form, so it needs no choices.
                        uploadEntry = .recovery
                    }
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
                    page.confirm("examShare.disconnectHint", notices: model.notices) { await accountAction(.disconnect) }
                }
                Button(t("examShare.deleteAccount"), role: .destructive) {
                    model.notices.confirm(title: t("examShare.deleteAccountTitle"),
                                          message: t("examShare.deleteAccountBody"),
                                          action: t("examShare.deleteAccount"), destructive: true) {
                        Task {
                            guard await page.call(model, .deleteAccount) != nil else { return }
                            model.notices.show(t("examShare.accountDeleted"))
                        }
                    }
                }
                    .disabled(uploadRunning)
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

    /// An upload Core is sending; the account stays until it ends.
    private var uploadRunning: Bool {
        switch account?.upload.phase {
        case .preparing?, .transferring?, .registering?, .cleaning?: true
        default: false
        }
    }

    @ViewBuilder
    private func documents(_ title: String, _ items: [ExamDocument], removal: ExamRemoval? = nil) -> some View {
        if !items.isEmpty {
            Section(t(title)) {
                ForEach(items, id: \.id) { item in
                    NavigationLink(value: ExamDocumentRoute(id: item.id, removal: removal)) {
                        ExamListRow(title: item.subjects.map(\.name).joined(separator: " / "), meta: [
                            examSitting(year: item.academicYear, occasion: item.occasion, occasionText: item.occasionText,
                                        number: item.occasionNumber, variant: item.variant),
                            ExamLabels.role(item.role, text: item.roleText),
                            ExamLabels.teachers(item.teachers, all: item.allTeachers),
                        ])
                    }
                    .swipeActions {
                        if let removal {
                            Button(t(removal.key), role: .destructive) {
                                page.confirm(removal.key, notices: model.notices) {
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
            requestPages[mine] = ExamRequestPage(items: previous + value.items, cursor: value.cursor)
        }
    }
}

/// Before a usable session: sign in, the terms, or why not.
private struct ExamAccountGate: View {
    @Environment(AppModel.self) private var model
    let page: ExamPage
    let status: ExamAccountStatus
    let site: ExamSitePages?
    let act: (ExamAction) async -> Void
    /// The consent button running; it alone spins, and neither takes a tap.
    @State private var consenting: Consent?

    private enum Consent { case accept, decline }

    var body: some View {
        ContentUnavailableView {
            Label(t("examShare.title"), systemImage: "doc.on.doc")
        } description: {
            switch status {
            case .needsConsent: Text(t("error.exam-share-consent"))
            case .unsupported: Text(t("error.exam-share-unsupported"))
            case .disconnected, .connected:
                Text(page.connecting ? t("common.signingIn") : t("examShare.connectHint"))
            }
        } actions: {
            switch status {
            case .needsConsent:
                consentButton(.accept, t("examShare.accept"), .acceptTerms)
                    .buttonStyle(.borderedProminent)
                consentButton(.decline, t("examShare.decline"), .decline)
                if let site {
                    HStack(spacing: Theme.space8) {
                        if let terms = URL(string: site.terms) { Link(t("examShare.terms"), destination: terms) }
                        if let privacy = URL(string: site.privacy) { Link(t("examShare.privacy"), destination: privacy) }
                    }
                    .font(.subheadline)
                }
            case .unsupported:
                EmptyView()
            case .disconnected, .connected:
                if page.connecting {
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

    private func consentButton(_ choice: Consent, _ title: String, _ action: ExamAction) -> some View {
        Button {
            consenting = choice
            Task {
                await act(action)
                consenting = nil
            }
        } label: {
            if consenting == choice { ProgressView() } else { Text(title) }
        }
        .disabled(consenting != nil || page.busy)
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
            HStack(spacing: Theme.space3) {
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
            .padding(.horizontal, Theme.space3)
            .padding(.vertical, Theme.space1)
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

/// How a document, a sitting or a request names its occasion, role and
/// teachers, on every past-exams page.
enum ExamLabels {
    /// An `other` occasion's own name, else the label for the API's code (a
    /// code this build does not know reads as other); none without one.
    static func occasion(_ code: String?, text: String) -> String? {
        if !text.isEmpty { return text }
        return code.map { (occasions[$0] ?? .other).label }
    }

    /// An `other` role's own name, else the label for the API's code (a code
    /// this build does not know reads as other); none without one.
    static func role(_ code: String?, text: String) -> String? {
        if !text.isEmpty { return text }
        return code.map { (roles[$0] ?? .other).label }
    }

    /// "Common to all teachers", the names, or "Teacher unknown".
    static func teachers(_ teachers: [ExamTeacher], all: Bool) -> String {
        if all { return t("examShare.allTeachers") }
        return teachers.isEmpty ? t("examShare.unknownTeacher") : teachers.map(\.name).joined(separator: " / ")
    }

    // The API's codes, as Core passes a document's or a sitting's through.
    private static let occasions: [String: ExamOccasion] = [
        "final": .final, "midterm": .midterm, "quiz": .quiz, "report": .report, "other": .other, "none": .notApplicable,
    ]
    private static let roles: [String: ExamRole] = [
        "question": .question, "answer": .answer, "both": .both, "notes": .notes, "other": .other,
    ]
}

/// "2024 · Final · Answers", a request's facts in one line.
func examRequestFacts(_ item: ExamRequest) -> String {
    [item.academicYear.map(String.init), item.occasion?.label, item.wantedRole?.label]
        .compactMap { $0 }.joined(separator: " · ")
}

/// "2024 Final 2 B", a sitting's label.
func examSitting(year: UInt32?, occasion: String?, occasionText: String, number: UInt32, variant: String) -> String {
    let occasion = ExamLabels.occasion(occasion, text: occasionText).map { label in
        number > 0 ? "\(label) \(number)" : label
    }
    return [year.map(String.init), occasion, variant]
        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
}

/// "Subject · 2024 Final · Questions", a document's title.
func examName(_ exam: ExamDocument) -> String {
    let fields = [
        exam.subjects.map(\.name).joined(separator: " / "),
        examSitting(year: exam.academicYear, occasion: exam.occasion, occasionText: exam.occasionText,
                    number: exam.occasionNumber, variant: exam.variant),
        ExamLabels.role(exam.role, text: exam.roleText),
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
                .toolbar {
                    if let web {
                        ToolbarItem(placement: .topBarTrailing) {
                            ExamLinkButton(url: web) { Image(systemName: "safari") }
                                .accessibilityLabel(t("examShare.openOnWeb"))
                        }
                    }
                }
        }
    }
}
