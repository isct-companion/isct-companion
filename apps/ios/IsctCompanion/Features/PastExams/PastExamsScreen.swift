import IsctCore
import SwiftUI
import QuickLook

struct PastExamsRoute: Hashable {}

struct PastExamsScreen: View {
    @Environment(AppModel.self) private var model
    @State private var tab: Tab = .browse
    @State private var account: ExamAccount?
    @State private var query = ""
    @State private var subjects: [ExamSubject]?
    @State private var subject: ExamSubjectPage?
    @State private var file: ExamLocalDocument?
    @State private var quickLook: URL?
    @State private var library: ExamLibrary?
    @State private var requestPages: [Bool: ExamRequestPage] = [:]
    @State private var mine = false
    @State private var busy = false
    @State private var problem: String?
    @State private var notice: String?
    @State private var showsTimetable = false
    @State private var showsRequest = false
    @State private var showsReport = false
    @State private var uploadEntry: ExamUploadEntry?
    @State private var pendingUpload = false
    @State private var confirmation: Confirmation?
    @State private var year: UInt32?
    @State private var teacher = ""
    @State private var occasion = ""

    private func subjectName(_ subject: ExamSubject) -> String {
        subject.name
    }

    private enum Tab: String, CaseIterable { case browse, requests, library }
    private struct Confirmation {
        let key: String
        let work: () async -> Void
    }

    var body: some View {
        List {
            Section {
                if let email = account?.email { Text(email).foregroundStyle(Theme.muted) }
                if let problem { Text(problem).foregroundStyle(.red) }
                if let notice { Text(notice).foregroundStyle(Theme.muted) }
                switch account?.status {
                case .connected:
                    if pendingUpload {
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
                case .unavailable:
                    Text(t("error.exam-share-unsupported"))
                case .disconnected, .none:
                    Button(t("examShare.connect")) { Task { await accountAction(.connect) } }
                }
            }
            Section {
                Picker(t("examShare.title"), selection: $tab) {
                    ForEach(Tab.allCases, id: \.self) { Text(t("examShare.\($0.rawValue)")).tag($0) }
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
        }
        .themedRows()
        .themedList()
        .navigationTitle(t("examShare.title"))
        .toolbar {
            if busy {
                ToolbarItem(placement: .topBarTrailing) { ProgressView().controlSize(.small) }
            }
        }
        .disabled(busy)
        .task { await accountAction(.account) }
        .refreshable {
            guard file == nil else { return }
            await loadTab(refresh: true)
            await refreshUploadStatus()
        }
        .onChange(of: tab) {
            file = nil
            problem = nil
            notice = nil
            Task { await loadTab() }
        }
        .quickLookPreview($quickLook)
        .sheet(isPresented: $showsTimetable) {
            NavigationStack {
                TimetableImportPreview()
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button(t("common.close")) { showsTimetable = false }
                        }
                    }
            }
        }
        .sheet(isPresented: $showsRequest) {
            if let subject {
                ExamRequestForm(subject: subject, busy: busy, problem: problem) { draft in
                    if await call(.createRequest(draft: draft)) != nil {
                        requestPages.removeAll()
                        showsRequest = false
                        notice = t("examShare.requestCreated")
                    }
                }
            }
        }
        .sheet(isPresented: $showsReport) {
            if let file {
                ExamReportForm(busy: busy, problem: problem) { reason, detail in
                    if await call(.report(id: file.document.id, reason: reason, detail: detail)) != nil {
                        showsReport = false
                        notice = t("examShare.reported")
                    }
                }
            }
        }
        .sheet(item: $uploadEntry, onDismiss: {
            Task { await refreshUploadStatus() }
        }) { entry in
            ExamUploadSheet(entry: entry, subjectName: subjectName) {
                uploadEntry = nil
                pendingUpload = false
                requestPages.removeAll()
                library = nil
                await loadTab(refresh: true)
                notice = t("examShare.upload.uploaded")
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
        Section {
            Button(t("examShare.importPreview"), systemImage: "calendar") { showsTimetable = true }
        }
        if account?.status == .connected {
            Section {
                TextField(t("examShare.search"), text: $query)
                    .submitLabel(.search)
                    .onSubmit { Task { await search() } }
                Button(t("examShare.search")) { Task { await search() } }
                    .disabled(query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            if let subject {
                Section(subjectName(subject.subject)) {
                    Button(t("common.back")) { self.subject = nil }
                    Button(t("examShare.newRequest")) { showsRequest = true }
                    Button(t("examShare.upload.title"), systemImage: "arrow.up.doc") {
                        uploadEntry = .subject(subject.subject)
                    }
                    Picker(t("examShare.year"), selection: $year) {
                        Text(t("examShare.allYears")).tag(nil as UInt32?)
                        ForEach(Set(subject.exams.compactMap(\.academicYear)).sorted(by: >), id: \.self) { Text(String($0)).tag(Optional($0)) }
                    }
                    Picker(t("examShare.teacher"), selection: $teacher) {
                        Text(t("examShare.allTeachers")).tag("")
                        ForEach(subjectTeachers(subject.exams), id: \.id) { Text($0.name).tag($0.id) }
                    }
                    Picker(t("examShare.occasion"), selection: $occasion) {
                        Text(t("examShare.allOccasions")).tag("")
                        ForEach(examOccasions, id: \.self) { Text(t("examShare.occasion.\($0)")).tag($0) }
                    }
                    documents(subject.exams.filter {
                        (year == nil || $0.academicYear == year) &&
                        (teacher.isEmpty || $0.teachers.contains { $0.id == teacher }) &&
                        (occasion.isEmpty || $0.occasion == occasion)
                    })
                }
            } else if let subjects {
                Section {
                    if subjects.isEmpty { Text(t("examShare.noItems")) }
                    ForEach(subjects, id: \.code) { result in
                        Button {
                            Task {
                                guard case let .subject(page)? = await call(.subject(code: result.code)) else { return }
                                subject = page
                                year = nil; teacher = ""; occasion = ""
                            }
                        } label: {
                            VStack(alignment: .leading) {
                                Text(subjectName(result))
                                Text(result.code).font(.footnote.monospaced()).foregroundStyle(Theme.muted)
                            }
                        }
                    }
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
            .onChange(of: mine) { Task { await loadTab() } }
            Button(t("examShare.newRequest")) { tab = .browse }
            if requests?.items.isEmpty == true { Text(t("examShare.noItems")) }
            ForEach(requests?.items ?? [], id: \.id) { item in
                VStack(alignment: .leading, spacing: 6) {
                    Text(subjectName(item.subject)).font(.headline)
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
                                    await changeRequest(.setRequestOpen(id: item.id, open: item.status != "open"))
                                }
                            }
                            Button(t("common.delete"), role: .destructive) {
                                confirmation = Confirmation(key: "examShare.deleteRequest") {
                                    await changeRequest(.deleteRequest(id: item.id))
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
                                if await call(action) != nil { await loadTab(refresh: true) }
                            }
                        }
                    }
                }
        }
    }

    private func document(_ local: ExamLocalDocument) -> some View {
        Section(examName(local.document)) {
            let id = local.document.id
            Button(t("common.back")) { file = nil; Task { await loadTab() } }
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
            if case let .account(snapshot)? = try? await model.examShare(.accountSnapshot) {
                displayAccount(snapshot)
            }
            return nil
        }
    }

    private func accountAction(_ action: ExamAction) async {
        guard case let .account(value)? = await call(action) else { return }
        displayAccount(value)
        if value.status == .connected {
            await refreshUploadStatus()
            await loadTab()
        }
    }

    private func displayAccount(_ value: ExamAccount) {
        account = value
        if value.status != .connected {
            subjects = nil
            subject = nil
            file = nil
            quickLook = nil
            requestPages.removeAll()
            library = nil
            showsRequest = false
            showsReport = false
            uploadEntry = nil
            pendingUpload = false
        }
    }

    private func refreshUploadStatus() async {
        if case let .uploadStatus(status)? = try? await model.examShare(.uploadStatus) {
            pendingUpload = status.phase == .pending
        }
    }

    private func search() async {
        guard case let .subjects(result)? = await call(.search(query: query)) else { return }
        subjects = result
        subject = nil
    }

    private func loadTab(refresh: Bool = false) async {
        guard account?.status == .connected else { return }
        switch tab {
        case .browse:
            guard refresh else { return }
            if let code = subject?.subject.code {
                if case let .subject(page)? = await call(.subject(code: code)) { subject = page }
            } else if subjects != nil {
                await search()
            }
        case .requests:
            if refresh || requestPages[mine] == nil { await loadRequests() }
        case .library:
            if (refresh || library == nil), case let .library(value)? = await call(.library) {
                library = value
            }
        }
    }

    private func open(_ id: String, download: Bool = false, present: Bool = true) async {
        guard case let .document(value)? = await call(.open(id: id, download: download)) else { return }
        file = value
        if download { library = nil }
        if present { quickLook = URL(fileURLWithPath: value.path) }
    }

    private func changeRequest(_ action: ExamAction) async {
        guard await call(action) != nil else { return }
        requestPages[!mine] = nil
        await loadRequests()
    }

    private func loadRequests(_ cursor: ExamRequestCursor? = nil) async {
        let mine = mine
        if case let .requests(value)? = await call(.requests(mine: mine, cursor: cursor)) {
            let previous = cursor == nil ? [] : requestPages[mine]?.items ?? []
            requestPages[mine] = ExamRequestPage(items: previous + value.items, cursor: value.cursor)
        }
    }
}

private let examOccasions = ["final", "midterm", "makeup", "quiz", "check", "report", "exercise", "textbook", "mock"]

private func subjectTeachers(_ exams: [ExamDocument]) -> [ExamTeacher] {
    var seen = Set<String>()
    return exams.flatMap(\.teachers).filter { seen.insert($0.id).inserted }
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
    let busy: Bool
    let problem: String?
    let submit: (ExamRequestDraft) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var classId: UInt32?
    @State private var year = ""
    @State private var occasion = ""
    @State private var role = ""
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
                        Text(t("examShare.allOccasions")).tag("")
                        ForEach(examOccasions, id: \.self) { Text(t("examShare.occasion.\($0)")).tag($0) }
                    }
                    Picker(t("examShare.wantedRole"), selection: $role) {
                        Text(t("examShare.wantedRole")).tag("")
                        Text(t("examShare.role.question")).tag("question")
                        Text(t("examShare.role.answer")).tag("answer")
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
                            academicYear: UInt32(year), occasion: occasion.isEmpty ? nil : occasion,
                            wantedRole: role.isEmpty ? nil : role, description: description)
                        Task { await submit(draft) }
                    }.disabled(busy || description.utf16.count > 1000 || (!year.isEmpty && UInt32(year) == nil))
                }
            }
        }
    }
}

private struct ExamReportForm: View {
    let busy: Bool
    let problem: String?
    let submit: (ExamReportReason, String) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var reason: ExamReportReason = .copyright
    @State private var detail = ""
    var body: some View {
        NavigationStack {
            Form {
                Picker(t("examShare.report"), selection: $reason) {
                    Text(t("examShare.reason.copyright")).tag(ExamReportReason.copyright)
                    Text(t("examShare.reason.inappropriate")).tag(ExamReportReason.inappropriate)
                    Text(t("examShare.reason.wrongAnswer")).tag(ExamReportReason.wrongAnswer)
                    Text(t("examShare.reason.brokenFile")).tag(ExamReportReason.brokenFile)
                    Text(t("examShare.reason.other")).tag(ExamReportReason.other)
                }
                TextField(t("examShare.description"), text: $detail, axis: .vertical)
                if let problem { Text(problem).foregroundStyle(.red) }
            }
            .navigationTitle(t("examShare.report"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("examShare.report")) { Task { await submit(reason, detail) } }
                        .disabled(busy || detail.utf16.count > 900)
                }
            }
        }
    }
}
