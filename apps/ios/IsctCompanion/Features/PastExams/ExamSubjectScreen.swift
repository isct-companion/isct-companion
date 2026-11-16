import IsctCore
import SwiftUI

/// One course's sittings as the desktop's panel shows them: the person's own
/// teachers first, the others folded away, a button per document.
struct ExamSubjectScreen: View {
    @Environment(ExamShareStore.self) private var store
    let route: ExamSubjectRoute
    @State private var page: ExamSubjectPage?
    @State private var failed = false
    @State private var othersOpen = false
    @State private var uploading = false
    @State private var requesting = false

    var body: some View {
        List {
            if let page {
                let grouped = !page.myTeachers.isEmpty
                let mine = page.sets.filter { !grouped || $0.mine }
                let others = grouped ? page.sets.filter { !$0.mine } : []
                if page.sets.isEmpty {
                    Text(t("examShare.noDocuments")).foregroundStyle(Theme.muted)
                } else {
                    Section {
                        if mine.isEmpty {
                            Text(t("examShare.noOwnTeacher")).foregroundStyle(Theme.muted)
                        }
                        ForEach(mine, id: \.documents.first?.id) { ExamSetRow(set: $0, withTeachers: !$0.mine) }
                    }
                    if !others.isEmpty {
                        Section {
                            DisclosureGroup(t("examShare.otherTeachers", ["count": others.count]), isExpanded: $othersOpen) {
                                ForEach(others, id: \.documents.first?.id) { ExamSetRow(set: $0, withTeachers: true) }
                            }
                        }
                    }
                }
            } else if failed {
                Button(t("common.retry")) { Task { await load() } }
            }
        }
        .themedRows()
        .themedList()
        .overlay { if page == nil && !failed { ProgressView() } }
        .navigationTitle(route.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let page {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button(t("examShare.uploadHere"), systemImage: "square.and.arrow.up") { uploading = true }
                        Button(t("examShare.newRequest"), systemImage: "text.bubble") { requesting = true }
                        if let url = URL(string: "\(ExamSite.base)/search?course=\(page.subject.code)") {
                            Link(destination: url) { Label(t("examShare.openInBrowser"), systemImage: "safari") }
                        }
                    } label: {
                        Label(t("examShare.uploadHere"), systemImage: "plus")
                    }
                }
            }
        }
        .sheet(isPresented: $uploading) {
            if let page {
                ExamUploadSheet(target: ExamUploadTarget(
                    subjectCode: page.subject.code,
                    subjectName: route.title,
                    teachers: uniqueTeachers(page.rows.flatMap(\.teachers)),
                    mine: page.myTeachers)) { await load() }
            }
        }
        .sheet(isPresented: $requesting) {
            if let page {
                ExamRequestForm(subject: page) { draft in
                    if await store.call(.createRequest(draft: draft)) != nil {
                        requesting = false
                        store.notice = t("examShare.requestCreated")
                    }
                }
            }
        }
        .refreshable { await load() }
        .task { if page == nil { await load() } }
        .examShareFeedback(store)
    }

    private func load() async {
        failed = false
        if case let .subject(value)? = await store.call(.subject(code: route.code, instructor: route.instructor)) {
            page = value
        } else {
            failed = page == nil
        }
    }
}

func uniqueTeachers(_ teachers: [ExamTeacher]) -> [ExamTeacher] {
    var seen = Set<String>()
    return teachers.filter { seen.insert($0.id).inserted }
}

/// One sitting: a label, and a button per document (question, answer) —
/// the buttons are what opens, as on the desktop.
private struct ExamSetRow: View {
    @Environment(ExamShareStore.self) private var store
    @Environment(\.push) private var push
    let set: ExamSet
    let withTeachers: Bool

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "doc.text").foregroundStyle(Theme.accent)
            VStack(alignment: .leading, spacing: 2) {
                Text(store.sitting(year: set.academicYear, occasion: set.occasion,
                                   number: set.occasionNumber, variant: set.variant))
                    .foregroundStyle(Theme.ink)
                if withTeachers {
                    Text(set.teachers.isEmpty ? t("examShare.unknownTeacher") : set.teachers.map(\.name).joined(separator: " / "))
                        .font(.footnote).foregroundStyle(Theme.muted).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            ForEach(set.documents, id: \.id) { document in
                let answer = document.role == "answer"
                // A borderless button answers only its own tap; a link would
                // make the whole row open and add a chevron.
                Button { push(ExamDocumentRoute(id: document.id)) } label: {
                    Text(document.role.map { t("examShare.role.\($0)") } ?? t("examShare.file"))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(answer ? Theme.ok : Theme.accent)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 5)
                        .background(answer ? Theme.okSoft : Theme.accentSoft, in: Capsule())
                        .overlay { Capsule().strokeBorder((answer ? Theme.ok : Theme.accent).opacity(0.35)) }
                }
                .buttonStyle(.borderless)
            }
        }
        .padding(.vertical, 2)
    }
}
