import IsctCore
import SwiftUI

/// One subject's sittings: the course's own teachers first, the others folded
/// away, a pill per document (questions, answers) that opens it. The toolbar
/// narrows the sittings, and uploads, requests or opens the site.
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
    @State private var othersOpen = false
    @State private var showsRequest = false
    @State private var uploadEntry: ExamUploadEntry?

    var body: some View {
        List {
            if let subject {
                let sets = subject.sets
                // Only a course with its own teachers among the sittings splits them.
                let grouped = subject.sets.contains(where: \.mine)
                let mine = grouped ? sets.filter(\.mine) : sets
                let others = grouped ? sets.filter { !$0.mine } : []
                if sets.isEmpty {
                    Text(t("examShare.noDocuments")).foregroundStyle(Theme.muted)
                } else {
                    if !mine.isEmpty {
                        Section {
                            ForEach(mine, id: \.self) { row($0, withTeachers: !$0.mine) }
                        }
                    }
                    if !others.isEmpty {
                        Section {
                            DisclosureGroup(t("examShare.otherTeachers", ["count": others.count]), isExpanded: $othersOpen) {
                                ForEach(others, id: \.self) { row($0, withTeachers: true) }
                            }
                        }
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
                        Button(t("examShare.upload.title"), systemImage: "square.and.arrow.up") {
                            uploadEntry = .subject(subject.subject)
                        }
                        .disabled(page.choices == nil)
                        Button(t("examShare.newRequest"), systemImage: "text.bubble") { showsRequest = true }
                            .disabled(page.choices == nil)
                        if let web = URL(string: subject.webUrl) {
                            Link(destination: web) { Label(t("examShare.openOnWeb"), systemImage: "safari") }
                        }
                    } label: {
                        Label(t("examShare.upload.title"), systemImage: "plus")
                    }
                }
            }
        }
        .refreshable { await load() }
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
                    await load()
                    page.notice = outcome.notice
                    if let id = outcome.documentId { push(ExamDocumentRoute(id: id)) }
                }
            }
        }
        .examPageFeedback(page)
    }

    private func row(_ set: ExamSet, withTeachers: Bool) -> some View {
        ExamSetRow(set: set, withTeachers: withTeachers, opening: opening) { id in Task { await open(id) } }
    }

    private func load() async {
        failed = false
        // A refresh keeps the class the page was ordered by.
        let action: ExamAction = subject.map { .subject(code: $0.subject.code, classId: $0.classId) } ?? route.action
        switch await page.call(model, action) {
        case let .subject(value)?:
            subject = value
            candidates = nil
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

/// One sitting: a label, its teachers when they are not the course's own,
/// and a pill per document. The row itself does not open; each pill does.
private struct ExamSetRow: View {
    let set: ExamSet
    let withTeachers: Bool
    let opening: String?
    let open: (String) -> Void

    var body: some View {
        HStack(spacing: Theme.space5) {
            Image(systemName: "doc.text").foregroundStyle(Theme.accent)
            VStack(alignment: .leading, spacing: Theme.space1) {
                let label = examSitting(year: set.academicYear, occasion: set.occasion,
                                        number: set.occasionNumber, variant: set.variant)
                Text(label.isEmpty ? t("examShare.occasion.unknown") : label).foregroundStyle(Theme.ink)
                if withTeachers {
                    Text(set.teachers.isEmpty ? t("examShare.unknownTeacher") : set.teachers.map(\.name).joined(separator: " / "))
                        .font(.footnote).foregroundStyle(Theme.muted).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            ForEach(set.documents, id: \.id) { document in
                pill(document)
            }
        }
        .padding(.vertical, Theme.space1)
    }

    private func pill(_ document: ExamDocument) -> some View {
        let answer = document.role == "answer"
        let tint = answer ? Theme.ok : Theme.accent
        // A borderless button answers only its own tap; a link would make
        // the whole row open and add a chevron.
        return Button { open(document.id) } label: {
            ZStack {
                // Core numbers several documents of one role in a sitting.
                Text([document.role.map { t("examShare.role.\($0)") } ?? document.fileName, document.number.map(String.init)]
                    .compactMap { $0 }.joined(separator: " "))
                    .opacity(opening == document.id ? 0 : 1)
                if opening == document.id { ProgressView().controlSize(.small) }
            }
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, Theme.space6)
            .padding(.vertical, 5)
            .background(answer ? Theme.okSoft : Theme.accentSoft, in: Capsule())
            .overlay { Capsule().strokeBorder(tint.opacity(0.35)) }
        }
        .buttonStyle(.borderless)
        .disabled(opening != nil)
    }
}
