import IsctCore
import SwiftUI

/// One subject's materials by sitting, as Core groups them: a row per
/// sitting, a pill per document (questions, answers) that opens it. The
/// filters narrow the sittings; the toolbar uploads, requests or opens the
/// site.
struct ExamSubjectScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push
    let route: ExamSubjectRoute
    @State private var page = ExamPage()
    @State private var subject: ExamSubjectPage?
    /// The route named several subjects (a course listing more than one).
    @State private var candidates: [ExamSubject]?
    @State private var failed = false
    @State private var opening: String?
    @State private var showsRequest = false
    @State private var uploadEntry: ExamUploadEntry?
    @State private var year: UInt32?
    @State private var teacher = ""
    @State private var occasion = ""

    var body: some View {
        List {
            if let subject {
                Section {
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
                }
                Section {
                    let sets = subject.sets.filter {
                        (year == nil || $0.academicYear == year) &&
                        (teacher.isEmpty || $0.teachers.contains { $0.id == teacher }) &&
                        (occasion.isEmpty || $0.occasion == occasion)
                    }
                    if sets.isEmpty { Text(t("examShare.noItems")).foregroundStyle(Theme.muted) }
                    ForEach(sets, id: \.self) { set in
                        ExamSetRow(set: set, opening: opening) { id in Task { await open(id) } }
                    }
                }
                Section {} footer: { ExamCredit() }
            } else if let candidates {
                ExamSubjectResults(subjects: candidates)
            } else if failed {
                Button(t("common.retry")) { Task { await load() } }
            }
        }
        .themedRows()
        .themedList()
        .overlay { if subject == nil && candidates == nil && !failed { ProgressView() } }
        .navigationTitle(subject?.subject.name ?? route.title ?? t("examShare.title"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let subject {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button(t("examShare.upload.title"), systemImage: "arrow.up.doc") {
                            uploadEntry = .subject(subject.subject)
                        }
                        Button(t("examShare.newRequest"), systemImage: "text.bubble") { showsRequest = true }
                        if let web = URL(string: subject.webUrl) {
                            Link(destination: web) { Label(t("examShare.openOnWeb"), systemImage: "safari") }
                        }
                    } label: {
                        Label(t("examShare.upload.title"), systemImage: "plus")
                    }
                    .disabled(page.busy)
                }
            }
        }
        .refreshable { await load(resetFilters: false) }
        .task {
            await page.loadChoices(model)
            if subject == nil && candidates == nil { await load() }
        }
        .sheet(isPresented: $showsRequest) {
            if let subject, let choices = page.choices {
                ExamRequestForm(subject: subject, choices: choices, busy: page.busy, problem: page.problem) { draft in
                    if await page.call(model, .createRequest(draft: draft)) != nil {
                        showsRequest = false
                        page.notice = t("examShare.requestCreated")
                    }
                }
            }
        }
        .sheet(item: $uploadEntry) { entry in
            if let choices = page.choices {
                ExamUploadSheet(entry: entry, choices: choices) { outcome in
                    await load(resetFilters: false)
                    page.notice = outcome.notice
                    if let id = outcome.documentId { push(ExamDocumentRoute(id: id)) }
                }
            }
        }
        .examPageFeedback(page)
    }

    private func load(resetFilters: Bool = true) async {
        failed = false
        // A refresh keeps the class the page was ordered by.
        let action: ExamAction = subject.map { .subject(code: $0.subject.code, classId: $0.classId) } ?? route.action
        switch await page.call(model, action) {
        case let .subject(value)?:
            subject = value
            candidates = nil
            if resetFilters {
                year = nil
                teacher = ""
                occasion = ""
            }
        case let .subjects(value)?:
            candidates = value
        default:
            failed = subject == nil && candidates == nil
        }
    }

    /// Reads the document here, the tapped pill spinning, then pushes it.
    private func open(_ id: String) async {
        guard opening == nil else { return }
        opening = id
        defer { opening = nil }
        if case let .document(file)? = await page.call(model, .open(id: id, download: false)) {
            push(ExamDocumentRoute(id: id, file: file))
        }
    }
}

/// One sitting: its label, its teachers when they are not the course's own,
/// and a pill per document. The row itself does not open; each pill does.
private struct ExamSetRow: View {
    let set: ExamSet
    let opening: String?
    let open: (String) -> Void

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                let label = examSitting(year: set.academicYear, occasion: set.occasion,
                                        number: set.occasionNumber, variant: set.variant)
                Text(label.isEmpty ? t("examShare.occasion.unknown") : label).foregroundStyle(Theme.ink)
                if !set.mine && !set.teachers.isEmpty {
                    Text(set.teachers.map(\.name).joined(separator: " / "))
                        .font(.footnote).foregroundStyle(Theme.muted).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            ForEach(set.documents, id: \.id) { document in
                pill(document)
            }
        }
        .padding(.vertical, 2)
    }

    private func pill(_ document: ExamDocument) -> some View {
        let answer = document.role == "answer"
        let tint = answer ? Theme.ok : Theme.accent
        // A borderless button answers only its own tap; a link would make
        // the whole row open and add a chevron.
        return Button { open(document.id) } label: {
            ZStack {
                Text(document.role.map { t("examShare.role.\($0)") } ?? document.fileName)
                    .opacity(opening == document.id ? 0 : 1)
                if opening == document.id { ProgressView().controlSize(.small) }
            }
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 12)
            .padding(.vertical, 5)
            .background(answer ? Theme.okSoft : Theme.accentSoft, in: Capsule())
            .overlay { Capsule().strokeBorder(tint.opacity(0.35)) }
        }
        .buttonStyle(.borderless)
        .disabled(opening != nil)
    }
}
