import IsctCore
import SwiftUI

/// 授業 (iPad): a quarter's courses or its assignments, as desktop's
/// LectureDesk switches them, with the academic year and quarter picked above
/// the list. Courses are `WorkspaceOverview.termCourseIds`, assignments the
/// quarter's `AssignmentBoard.record`. Stacked, a row pushes its page; on the
/// workstation it selects the page shown beside the list.
struct LecturesScreen: View {
    /// The workstation's list, whose rows select rather than push.
    var selects = false
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var overview: WorkspaceOverview?
    @State private var board: AssignmentBoard?

    var body: some View {
        @Bindable var navigator = navigator
        List(selection: selection) {
            Section {
                TermPicker(term: term, terms: overview?.allTerms ?? []) { navigator.lectureTerm = $0 }
                Picker(t("Show"), selection: $navigator.lectureMode) {
                    Text(t("Courses")).tag(LectureMode.courses)
                    Text(t("Assignments")).tag(LectureMode.assignments)
                }
                .pickerStyle(.segmented)
            }
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0))
            .listRowSeparator(.hidden)
            switch navigator.lectureMode {
            case .courses:
                Section {
                    ForEach(overview?.termCourseIds ?? [], id: \.self) { id in
                        if selects {
                            row(id).tag(id)
                        } else {
                            NavigationLink(value: CourseRoute(id: id, zoom: "lecture-\(id)")) { row(id) }
                                .courseZoomSource("lecture-\(id)")
                        }
                    }
                }
            case .assignments:
                if let record = board?.record {
                    RecordSections(record: record, selectable: selects)
                }
            }
        }
        .navigationTitle(t("Courses"))
        .overlay {
            if isEmpty {
                ContentUnavailableView(
                    navigator.lectureMode == .courses ? t("No courses this term yet") : t("No assignments"),
                    systemImage: navigator.lectureMode == .courses ? "books.vertical" : "checklist")
            }
        }
        .task(id: "\(term ?? "")-\(model.contentRevision)") {
            guard let term else { return }
            async let courses = model.overview(term)
            async let assignments = model.board(term: term)
            (overview, board) = await (courses, assignments)
        }
        .onChange(of: term) {
            navigator.lecture = nil
            navigator.lectureAssignment = nil
        }
    }

    /// The quarter shown: the one picked, or this quarter.
    private var term: String? { navigator.lectureTerm ?? model.today?.term }

    /// On the workstation, the course or the assignment open beside the list.
    private var selection: Binding<String?>? {
        guard selects else { return nil }
        @Bindable var navigator = navigator
        return navigator.lectureMode == .courses ? $navigator.lecture : $navigator.lectureAssignment
    }

    private var isEmpty: Bool {
        switch navigator.lectureMode {
        case .courses: overview?.termCourseIds.isEmpty == true
        case .assignments: board.map { Self.isEmpty($0.record) } == true
        }
    }

    private static func isEmpty(_ record: DeadlineGroups) -> Bool {
        record.pinned.isEmpty && record.dueThisWeek.isEmpty && record.later.isEmpty && record.undated.isEmpty
            && record.pastRecent.isEmpty && record.pastEarlier.isEmpty && record.hidden.isEmpty
    }

    private func row(_ id: String) -> some View {
        let course = model.records.courses[id] ?? [:]
        return HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(Color(hex: overview?.colors[id] ?? model.colors[id]))
                .frame(width: 6, height: 32)
            VStack(alignment: .leading, spacing: 2) {
                Text(course.text("title") ?? id).font(.body.weight(.medium)).foregroundStyle(Theme.ink).lineLimit(2)
                if let code = course.text("code") {
                    Text(code).font(.footnote.monospaced()).foregroundStyle(Theme.muted)
                }
            }
        }
        .padding(.vertical, 2)
    }
}

/// The academic year as a menu of the years Core has quarters for, and its
/// four quarters as a segmented control beside it.
private struct TermPicker: View {
    let term: String?
    /// Core's `allTerms`: every quarter with courses, exams or grades.
    let terms: [String]
    let pick: (String) -> Void

    var body: some View {
        let shown = term.flatMap(Self.parts)
        HStack(spacing: 10) {
            Menu {
                ForEach(years, id: \.self) { year in
                    Button(t("AY {{year}}", ["year": String(year)])) {
                        pick("\(year)-\(shown?.quarter ?? 1)Q")
                    }
                }
            } label: {
                HStack(spacing: 4) {
                    Text(shown.map { t("AY {{year}}", ["year": String($0.year)]) } ?? "")
                    Image(systemName: "chevron.up.chevron.down").font(.caption2.weight(.semibold))
                }
                .font(.subheadline.weight(.semibold))
                .fixedSize()
            }
            Picker(t("Quarter"), selection: Binding(
                get: { shown?.quarter ?? 1 },
                set: { quarter in shown.map { pick("\($0.year)-\(quarter)Q") } }
            )) {
                ForEach(1...4, id: \.self) { Text(verbatim: "\($0)Q").tag($0) }
            }
            .pickerStyle(.segmented)
        }
    }

    /// The years of Core's quarters, and the shown one even before it has any.
    private var years: [Int] {
        var years = Set(terms.compactMap { Self.parts($0)?.year })
        if let year = term.flatMap(Self.parts)?.year { years.insert(year) }
        return years.sorted(by: >)
    }

    /// `2026-3Q` as (2026, 3).
    private static func parts(_ term: String) -> (year: Int, quarter: Int)? {
        let pieces = term.split(separator: "-")
        guard pieces.count == 2, let year = Int(pieces[0]), let quarter = Int(pieces[1].dropLast()) else { return nil }
        return (year, quarter)
    }
}

/// 授業 on the workstation: the list and the chosen course's or assignment's
/// page, the desktop LectureDesk's two panes.
struct LecturesSplit: View {
    @Environment(Navigator.self) private var navigator

    var body: some View {
        HStack(spacing: 0) {
            NavigationStack {
                LecturesScreen(selects: true)
            }
            .frame(width: 360)
            Divider()
            NavigationStack(path: Binding(
                get: { navigator.paths[.lectures] ?? NavigationPath() },
                set: { navigator.paths[.lectures] = $0 }
            )) {
                Group {
                    switch navigator.lectureMode {
                    case .courses:
                        if let id = navigator.lecture {
                            CourseDetailScreen(courseId: id).id(id)
                        } else {
                            ContentUnavailableView(t("Choose a course"), systemImage: "books.vertical")
                        }
                    case .assignments:
                        if let id = navigator.lectureAssignment {
                            AssignmentScreen(assignmentId: id).id(id)
                        } else {
                            ContentUnavailableView(t("Choose an assignment"), systemImage: "checklist")
                        }
                    }
                }
                .appDestinations()
            }
        }
        .onChange(of: navigator.lecture) { navigator.paths[.lectures] = NavigationPath() }
        .onChange(of: navigator.lectureAssignment) { navigator.paths[.lectures] = NavigationPath() }
        .onChange(of: navigator.lectureMode) { navigator.paths[.lectures] = NavigationPath() }
    }
}
