import IsctCore
import SwiftUI
import QuickLook

struct PastExamsRoute: Hashable {
    let courseId: String?

    init(courseId: String? = nil) { self.courseId = courseId }
}

struct PastExamsScreen: View {
    let courseId: String?
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @State private var tab: Tab = .browse
    @State private var choices: ExamChoices?
    @State private var courseLookupPending = true
    @State private var query = ""
    @State private var subjects: [ExamSubject]?
    @State private var subject: ExamSubjectPage?
    /// An upload finished on another tab; the subject reloads when shown.
    @State private var subjectStale = false
    @State private var file: ExamLocalDocument?
    @State private var quickLook: URL?
    @State private var library: ExamLibrary?
    @State private var requestPages: [Bool: ExamRequestPage] = [:]
    @State private var mine = false
    @State private var busy = false
    @State private var problem: String?
    @State private var notice: String?
    @State private var courses: ExamCourseList?
    @State private var coursesFailed = false
    @State private var coursesRetry = 0
    @State private var showsRequest = false
    @State private var showsReport = false
    @State private var uploadEntry: ExamUploadEntry?
    @State private var confirmation: Confirmation?
    @State private var year: UInt32?
    @State private var teacher = ""
    @State private var occasion = ""

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
    private struct Confirmation {
        let key: String
        let work: () async -> Void
    }

    var body: some View {
        List {
            Section {
                if let email = account?.email { Text(email).foregroundStyle(Theme.muted) }
                if let problem, account?.status != .unsupported { Text(problem).foregroundStyle(.red) }
                if let notice { Text(notice).foregroundStyle(Theme.muted) }
                switch account?.status {
                case .connected:
                    if account?.upload.phase == .pending {
                        Button(t("examShare.upload.recover"), systemImage: "arrow.clockwise") {
                            uploadEntry = .recovery
                        }
                    }
                    Button(t("examShare.disconnect"), role: .destructive) {
                        confirmation = Confirmation(key: "examShare.disconnectHint") { await accountAction(.disconnect) }
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
            if account?.status != .unsupported {
                Section {
                    Picker(t("examShare.title"), selection: $tab) {
                        ForEach(Tab.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }
                if let file {
                    document(file)
                } else {
                    switch tab {
                    case .browse: browse
                    case .requests:
                        if account?.status == .connected { requestBoard }
                    case .library:
                        if account?.status == .connected { librarySections }
                    }
                }
                Section {} footer: { credit }
            }
        }
        .themedRows()
        .themedList()
        .modifier(PastExamsTitle(root: courseId == nil, busy: busy))
        .disabled(busy)
        .task(id: CoursesRead(revision: model.contentRevision, retry: coursesRetry)) {
            coursesFailed = false
            let answer = await model.examCourses()
            guard !Task.isCancelled else { return }
            courses = answer
            coursesFailed = answer == nil
        }
        .task {
            if case let .choices(value)? = try? await model.examShare(.choices) { choices = value }
        }
        .task(id: scenePhase) {
            guard scenePhase == .active else { return }
            await refresh()
        }
        .refreshable { await refresh() }
        .onChange(of: tab) {
            file = nil
            problem = nil
            notice = nil
            Task { await loadVisible(reload: false) }
        }
        .onChange(of: account?.status) { _, status in
            guard status != .connected else { return }
            // Nothing below the account section shows without a connection.
            courseLookupPending = true
            subjects = nil
            subject = nil
            file = nil
            quickLook = nil
            requestPages.removeAll()
            library = nil
            showsRequest = false
            showsReport = false
            uploadEntry = nil
            if status == .unsupported { confirmation = nil }
        }
        .quickLookPreview($quickLook)
        .sheet(isPresented: $showsRequest) {
            if let subject, let choices {
                ExamRequestForm(subject: subject, choices: choices, busy: busy, problem: problem) { draft in
                    if await call(.createRequest(draft: draft)) != nil {
                        requestPages.removeAll()
                        showsRequest = false
                        notice = t("examShare.requestCreated")
                    }
                }
            }
        }
        .sheet(isPresented: $showsReport) {
            if let file, let choices {
                ExamReportForm(reasons: choices.reportReasons, maxDetail: choices.limits.reportDetail,
                               busy: busy, problem: problem) { reason, detail in
                    if await call(.report(id: file.document.id, reason: reason, detail: detail)) != nil {
                        showsReport = false
                        notice = t("examShare.reported")
                    }
                }
            }
        }
        .sheet(item: $uploadEntry) { entry in
            if let choices {
                ExamUploadSheet(entry: entry, choices: choices) { published in
                    await uploadFinished()
                    notice = published ? t("examShare.upload.uploaded") : t("examShare.upload.cleaned")
                }
            }
        }
        .confirmationDialog(
            confirmation.map { t($0.key) } ?? "",
            isPresented: Binding(get: { confirmation != nil }, set: { if !$0 { confirmation = nil } }),
            titleVisibility: .visible,
            presenting: confirmation
        ) { confirmed in
            Button(t("common.ok"), role: .destructive) { Task { await confirmed.work() } }
        }
    }

    @ViewBuilder private var browse: some View {
        if account?.status == .connected {
            Section {
                TextField(t("examShare.search"), text: $query)
                    .submitLabel(.search)
                    .onSubmit { Task { await search() } }
                Button(t("examShare.search")) { Task { await search() } }
                    .disabled(query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            if let subject {
                Section(subject.subject.name) {
                    Button(t("common.back")) { self.subject = nil }
                    if let web = URL(string: subject.webUrl) {
                        Link(t("examShare.openOnWeb"), destination: web)
                    }
                    Button(t("examShare.newRequest")) { showsRequest = true }
                    Button(t("examShare.upload.title"), systemImage: "arrow.up.doc") {
                        uploadEntry = .subject(subject.subject)
                    }
                    Picker(t("examShare.year"), selection: $year) {
                        Text(t("examShare.allYears")).tag(nil as UInt32?)
                        ForEach(subject.years, id: \.self) { Text(String($0)).tag(Optional($0)) }
                    }
                    Picker(t("examShare.teacher"), selection: $teacher) {
                        Text(t("examShare.allTeachers")).tag("")
                        ForEach(subject.teachers, id: \.id) { Text($0.name).tag($0.id) }
                    }
                    Picker(t("examShare.occasion"), selection: $occasion) {
                        Text(t("examShare.allOccasions")).tag("")
                        ForEach(subject.occasions, id: \.self) { Text(t("examShare.occasion.\($0)")).tag($0) }
                    }
                    documents(subject.exams.filter {
                        (year == nil || $0.academicYear == year) &&
                        (teacher.isEmpty || $0.teachers.contains { $0.id == teacher }) &&
                        (occasion.isEmpty || $0.occasion == occasion)
                    })
                }
            } else {
                if let subjects {
                    Section {
                        if subjects.isEmpty { Text(t("examShare.noItems")) }
                        ForEach(subjects, id: \.code) { result in
                            Button {
                                Task { await loadSubject(.subject(code: result.code, classId: nil)) }
                            } label: {
                                VStack(alignment: .leading) {
                                    Text(result.name)
                                    Text(result.code).font(.footnote.monospaced()).foregroundStyle(Theme.muted)
                                }
                            }
                        }
                    }
                }
                CurrentCoursesSection(list: courses, failed: coursesFailed, retry: { coursesRetry += 1 }) { id in
                    Task { await loadSubject(.courseSubject(courseId: id)) }
                }
            }
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
            if requests?.items.isEmpty == true { Text(t("examShare.noItems")) }
            ForEach(requests?.items ?? [], id: \.id) { item in
                VStack(alignment: .leading, spacing: 6) {
                    Text(item.subject.name).font(.headline)
                    Text([item.academicYear.map(String.init), t("examShare.requestStatus.\(item.status)"), item.description]
                        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")).foregroundStyle(Theme.muted)
                    if let answer = item.answerExamId {
                        Button(t("examShare.viewAnswer")) { Task { await open(answer) } }.buttonStyle(.borderless)
                    }
                    if item.status == "open" {
                        Button(t("examShare.upload.answerRequest")) { uploadEntry = .request(item) }
                            .buttonStyle(.borderless)
                    }
                    if item.mine {
                        HStack {
                            Button(t(item.status == "open" ? "examShare.closeRequest" : "examShare.reopenRequest")) {
                                Task {
                                    await changeRequest(.setRequestOpen(id: item.id, open: item.status != "open", mine: mine))
                                }
                            }
                            Button(t("common.delete"), role: .destructive) {
                                confirmation = Confirmation(key: "examShare.deleteRequest") {
                                    await changeRequest(.deleteRequest(id: item.id, mine: mine))
                                }
                            }
                        }.buttonStyle(.borderless)
                    }
                }
            }
            if let cursor = requests?.cursor {
                Button(t("examShare.more")) { Task { await loadRequests(cursor) } }
            }
        }
    }

    /// The ScienceTokyoExam credit, English only by design: it names the
    /// service and its operator.
    private var credit: some View {
        Image("PastExams/credit")
            .resizable()
            .renderingMode(.template)
            .scaledToFit()
            .frame(height: 11)
            .foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .accessibilityLabel(Text(verbatim: "Supported by ScienceTokyoExam · teduryu"))
    }

    @ViewBuilder private var librarySections: some View {
        if let library {
            if library.offline {
                Section { Text(t("examShare.libraryUnavailable")).foregroundStyle(Theme.muted) }
            } else {
                Section(t("examShare.saved")) { documents(library.saved) }
            }
            Section(t("examShare.downloaded")) { documents(library.downloaded, removal: "examShare.removeDownload") }
            if !library.offline {
                Section(t("examShare.submitted")) { documents(library.submitted, removal: "examShare.deleteSubmission") }
            }
        }
    }

    @ViewBuilder private func documents(_ items: [ExamDocument], removal: String? = nil) -> some View {
        if items.isEmpty { Text(t("examShare.noItems")) }
        ForEach(items, id: \.id) { item in
            Button(examName(item)) { Task { await open(item.id) } }
                .swipeActions {
                    if let removal {
                        Button(t(removal), role: .destructive) {
                            confirmation = Confirmation(key: removal) {
                                let action: ExamAction = removal == "examShare.removeDownload"
                                    ? .removeDownload(id: item.id) : .deleteSubmission(id: item.id)
                                if case let .library(value)? = await call(action) { library = value }
                            }
                        }
                    }
                }
        }
    }

    private func document(_ local: ExamLocalDocument) -> some View {
        Section(examName(local.document)) {
            let id = local.document.id
            Button(t("common.back")) { file = nil; Task { await loadVisible(reload: false) } }
            Button(t("examShare.preview")) { quickLook = URL(fileURLWithPath: local.path) }
            Button(t(local.downloaded ? "examShare.downloaded" : "examShare.download")) {
                Task { await open(id, download: true, present: false) }
            }.disabled(local.downloaded || local.offline)
            if let saved = local.saved {
                Button(t(saved ? "examShare.unsave" : "examShare.save")) {
                    Task {
                        if case let .saved(updated)? = await call(.save(id: id, saved: !saved)) {
                            var refreshed = local
                            refreshed.saved = updated
                            file = refreshed
                            library = nil
                        }
                    }
                }.disabled(local.offline)
            }
            Button(t("examShare.report")) { showsReport = true }.disabled(local.offline)
            if local.offline { Text(t("examShare.offline")) }
        }
    }

    private func call(_ action: ExamAction) async -> ExamAnswer? {
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

    private func accountAction(_ action: ExamAction) async {
        guard await call(action) != nil else { return }
        await loadVisible(reload: false)
    }

    /// Scene activation and pull-to-refresh: the account, then the view on
    /// screen. Search results and this quarter's courses are not read again.
    private func refresh() async {
        guard await call(.account) != nil else { return }
        await loadVisible(reload: true)
    }

    /// The tab's view, read the first time it shows and kept across tab
    /// switches; `reload` reads it again.
    private func loadVisible(reload: Bool) async {
        guard account?.status == .connected else { return }
        switch tab {
        case .browse:
            if let subject, reload || subjectStale {
                await loadSubject(.subject(code: subject.subject.code, classId: subject.classId), resetFilters: false)
            } else if let courseId, courseLookupPending {
                await loadSubject(.courseSubject(courseId: courseId))
            }
        case .requests:
            if reload || requestPages[mine] == nil { await loadRequests() }
        case .library:
            if reload || library == nil, case let .library(value)? = await call(.library) {
                library = value
            }
        }
    }

    /// A finished upload changes every view: the visible one reloads, the
    /// others load fresh when shown.
    private func uploadFinished() async {
        subjectStale = tab != .browse
        if tab != .library { library = nil }
        requestPages = tab == .requests ? requestPages.filter { $0.key == mine } : [:]
        await loadVisible(reload: true)
    }

    private func search() async {
        await loadSubject(.search(query: query))
    }

    private func loadSubject(_ action: ExamAction, resetFilters: Bool = true) async {
        switch await call(action) {
        case let .subject(page)?:
            subject = page
            subjectStale = false
            if resetFilters {
                year = nil
                teacher = ""
                occasion = ""
            }
        case let .subjects(results)?:
            subjects = results
            subject = nil
        default: return
        }
        courseLookupPending = false
    }

    private func open(_ id: String, download: Bool = false, present: Bool = true) async {
        guard case let .document(value)? = await call(.open(id: id, download: download)) else { return }
        file = value
        if download { library = nil }
        if present { quickLook = URL(fileURLWithPath: value.path) }
    }

    /// A request write answers the first page of the board it was made on;
    /// the other board loads fresh when shown.
    private func changeRequest(_ action: ExamAction) async {
        let mine = mine
        guard case let .requests(page)? = await call(action) else { return }
        requestPages[mine] = page
        requestPages[!mine] = nil
    }

    private func loadRequests(_ cursor: ExamRequestCursor? = nil) async {
        let mine = mine
        if case let .requests(value)? = await call(.requests(mine: mine, cursor: cursor)) {
            let previous = cursor == nil ? [] : requestPages[mine]?.items ?? []
            requestPages[mine] = ExamRequestPage(items: previous + value.items, cursor: value.cursor)
        }
    }
}

private func examName(_ exam: ExamDocument) -> String {
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

private struct ExamRequestForm: View {
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

private struct ExamReportForm: View {
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
