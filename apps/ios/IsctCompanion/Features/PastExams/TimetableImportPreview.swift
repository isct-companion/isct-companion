import IsctCore
import SwiftUI

struct TimetableImportPreview: View {
    @Environment(AppModel.self) private var model
    @State private var term: String?
    @State private var terms: [String] = []
    @State private var preview: ExamCourseImportPreview?
    @State private var failed = false
    @State private var retry = 0

    private struct Read: Equatable {
        let term: String?
        let currentTerm: String?
        let revision: String
        let retry: Int
    }

    var body: some View {
        let currentTerm = model.records.currentTerm
        let read = Read(term: term, currentTerm: currentTerm, revision: model.contentRevision, retry: retry)
        let shown = preview.flatMap { $0.term == (term ?? currentTerm) ? $0 : nil }

        List {
            Section(t("examShare.importPreview")) {
                Picker(t("common.quarter"), selection: $term) {
                    Text(t("examShare.currentQuarter")).tag(nil as String?)
                    ForEach(terms, id: \.self) { id in
                        Text(verbatim: id).tag(Optional(id))
                    }
                }
            }
            Section {
                if let shown {
                    if shown.courses.isEmpty {
                        ContentUnavailableView(t("examShare.emptyTimetable"), systemImage: "calendar")
                    } else {
                        ForEach(shown.courses, id: \.courseId) { course in
                            VStack(alignment: .leading, spacing: 3) {
                                Text(course.title).foregroundStyle(Theme.ink)
                                if !course.instructor.isEmpty {
                                    Text(course.instructor).font(.subheadline).foregroundStyle(Theme.muted)
                                }
                                Text(course.subjectCodes.isEmpty
                                     ? t("examShare.noSubjectCode") : course.subjectCodes.joined(separator: " / "))
                                    .font(.footnote.monospaced())
                                    .foregroundStyle(Theme.muted)
                            }
                        }
                    }
                } else if failed {
                    ContentUnavailableView {
                        Label(t("failure.thisIsNotAvailableRight"), systemImage: "exclamationmark.triangle")
                    } actions: {
                        Button(t("common.retry")) { retry += 1 }
                    }
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
        }
        .themedRows()
        .themedList()
        .navigationTitle(t("examShare.importPreview"))
        .task(id: read) {
            failed = false
            let answer = await model.examCourseImport(term: read.term)
            guard !Task.isCancelled else { return }
            preview = answer
            failed = answer == nil
        }
        .task(id: TermsRead(term: currentTerm, revision: model.contentRevision)) {
            guard let currentTerm = read.currentTerm else { return }
            let overview = await model.overview(currentTerm)
            guard !Task.isCancelled, let overview else { return }
            terms = overview.allTerms
        }
    }

    private struct TermsRead: Equatable {
        let term: String?
        let revision: String
    }
}
