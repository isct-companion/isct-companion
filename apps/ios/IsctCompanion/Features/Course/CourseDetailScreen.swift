import IsctCore
import SwiftUI
import TipKit

/// The course page as the desktop Lecture Desk lays it out: the course, then
/// its lessons (一般, 1, 2, 演習 1, …) one at a time, each with its own text (授業
/// 計画), its files, the pages, labels and announcements shown in place, and
/// the assignments set in it, which open in place. Content comes from Core's
/// `CourseContent`; links and the files folder from `CourseDetail`.
struct CourseDetailScreen: View {
    let courseId: String
    @Environment(AppModel.self) private var model
    @State private var detail: CourseDetail?
    @State private var content: CourseContent?
    @State private var assignments: [AssignmentDetail] = []
    @State private var lesson: Int?
    /// The way the last page turn went, which sets the pages' transition.
    @State private var step: PageStep = .next
    @State private var opened: Set<String> = []
    /// Bumped when a lesson mark is recorded, for its haptic.
    @State private var marks = 0

    var body: some View {
        let course = model.records.courses[courseId] ?? [:]
        let title = course.text("title") ?? ""
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(title)
                        .font(.largeTitle.bold())
                        .foregroundStyle(Theme.ink)
                    Text(subtitle(course))
                        .font(.subheadline)
                        .foregroundStyle(Theme.muted)
                    examLines
                    links(title: title)
                }
                if !lessons.isEmpty {
                    LessonChips(
                        lessons: lessons,
                        selection: lesson,
                        current: content?.currentSection.map(Int.init),
                        classAt: content?.classAt.flatMap(AppModel.date),
                        select: select,
                        mark: mark)
                    pages
                } else if detail != nil {
                    ContentUnavailableView(t("No materials yet"), systemImage: "doc.text")
                } else {
                    SkeletonRows(rows: 4)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
        }
        .background(Theme.bg)
        .refreshable {
            if let id = lmsCourseId { model.syncHint(.refresh(scope: .course(lmsCourseId: id))) }
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .sensoryFeedback(.success, trigger: marks)
        .task(id: "\(courseId)-\(model.contentRevision)") {
            guard let loaded = await model.courseDetail(courseId), !Task.isCancelled else { return }
            detail = loaded
            let lessonsRead = await model.courseContent(courseId)
            guard !Task.isCancelled else { return }
            // Core picks where to open: the marked lesson, else the last
            // lesson with material.
            let opening = lesson.flatMap { number in
                lessonsRead?.lessons.contains { Int($0.section) == number } == true ? number : nil
            } ?? lessonsRead?.currentSection.map(Int.init)
            warm(lessonsRead?.lessons ?? [], around: opening)
            content = lessonsRead
            lesson = opening
            var found: [AssignmentDetail] = []
            for id in loaded.assignmentIds {
                if let assignment = await model.assignmentDetail(id) { found.append(assignment) }
            }
            guard !Task.isCancelled else { return }
            assignments = found
        }
        // After a turn, the pages either side of the new one.
        .task(id: lesson) { warm(lessons, around: lesson) }
        // Runs on appear and once the LMS id is known: Core's planner moves
        // this course's reads ahead while it is on screen.
        .task(id: lmsCourseId) {
            if let id = lmsCourseId { model.syncHint(.open(view: .course(lmsCourseId: id))) }
        }
    }

    private var lmsCourseId: UInt64? { detail?.lmsCourseId }

    // MARK: Lessons

    /// Core's lessons: sections in order, with the rail's labels.
    private var lessons: [LessonContent] { content?.lessons ?? [] }

    private var index: Int? { lessons.firstIndex { Int($0.section) == lesson } }

    /// One lesson at a time; a page moves as one piece, leaving the way the
    /// finger (or the chip row) points.
    private var pages: some View {
        let number = lesson ?? 0
        let shown = lessons.first { Int($0.section) == number }
        let set = workIds(shown)
        let page = LessonPage(
            lesson: shown,
            work: assignments.filter { set.contains($0.assignmentId) },
            files: detail?.files,
            opened: $opened)
        let forward = step == .next
        return ZStack(alignment: .topLeading) {
            page
                .id(number)
                .transition(.asymmetric(
                    insertion: .move(edge: forward ? .trailing : .leading).combined(with: .opacity),
                    removal: .move(edge: forward ? .leading : .trailing).combined(with: .opacity)))
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .contentShape(Rectangle())
        .horizontalPager(canGoPrevious: (index ?? 0) > 0, canGoNext: (index ?? lessons.count) < lessons.count - 1) { turn in
            guard let index else { return }
            let target = lessons[turn == .next ? index + 1 : index - 1]
            select(Int(target.section))
        }
    }

    /// The assignments Core sets in a lesson; those outside any section go
    /// with the first page.
    private func workIds(_ shown: LessonContent?) -> [String] {
        guard let shown else { return [] }
        let others = shown.section == lessons.first?.section ? content?.otherAssignmentIds ?? [] : []
        return shown.assignmentIds + others
    }

    private func select(_ target: Int) {
        guard target != lesson else { return }
        let from = index ?? 0
        let to = lessons.firstIndex { Int($0.section) == target } ?? 0
        HTMLCache.warm(Self.html(of: lessons[to]))
        step = to < from ? .previous : .next
        // The leaving page takes its removal from `step`: let it render with
        // the new step once before it is removed.
        DispatchQueue.main.async {
            withAnimation(.smooth(duration: Theme.motionBase)) { lesson = target }
        }
    }

    /// Records the lesson as the one for Core's `classAt`, as the desktop's
    /// double click on the rail does, then re-reads where the course is.
    private func mark(_ marked: LessonContent) {
        guard let classAt = content?.classAt, let sectionId = marked.sectionId else { return }
        Task {
            guard await model.recordLesson(courseId: courseId, occurrence: classAt, sectionId: sectionId) else { return }
            marks += 1
            if let reread = await model.courseContent(courseId) { content = reread }
        }
    }

    /// Renders the text of a lesson and its neighbours ahead of display, so a
    /// page has its height before it moves in.
    private func warm(_ lessons: [LessonContent], around number: Int?) {
        guard let center = lessons.firstIndex(where: { Int($0.section) == number }) else { return }
        for index in max(center - 1, 0)...min(center + 1, lessons.count - 1) {
            HTMLCache.warm(Self.html(of: lessons[index]))
        }
    }

    private static func html(of lesson: LessonContent) -> [String] {
        [lesson.summaryHtml] + lesson.items.compactMap(\.bodyHtml)
    }

    /// Items in LMS order: text shown in place stands on its own; runs of
    /// files and links share one card.
    static func groups(_ items: [LessonItem]) -> [[LessonItem]] {
        var groups: [[LessonItem]] = []
        for item in items {
            if item.bodyHtml != nil || groups.last?.last?.bodyHtml != nil || groups.isEmpty {
                groups.append([item])
            } else {
                groups[groups.count - 1].append(item)
            }
        }
        return groups
    }

    static func kind(_ kind: LessonItemKind) -> String {
        switch kind {
        case .file: "resource"
        case .folder: "folder"
        case .link, .other: "link"
        case .page: "page"
        case .label: "label"
        case .forum: "forum"
        }
    }

    // MARK: Header

    private struct Meeting {
        let day: Int
        let start: String
        let end: String
        let room: String
    }

    private func meetings(_ course: [String: Any]) -> [Meeting] {
        (course["meetings"] as? [[String: Any]] ?? []).compactMap { row in
            guard let start = row.text("startTime"), let end = row.text("endTime") else { return nil }
            let day = (row["days"] as? [NSNumber])?.first?.intValue ?? 0
            return Meeting(day: day, start: start, end: end, room: row.text("room") ?? "")
        }
    }

    /// `CSC.T273 · 月 3–4限 · W5-101`, `CSC.T273 · Mon Periods 3–4 · W5-101`
    private func subtitle(_ course: [String: Any]) -> String {
        let days = Format.weekdays
        var parts = [course.text("code")].compactMap { $0 }
        if let meeting = meetings(course).first {
            let start = model.periods.first { $0.startTime == meeting.start }?.periods.first
            let end = model.periods.first { $0.endTime == meeting.end }?.periods.last
            let periods = start.flatMap { s in end.flatMap { Format.periods([s, $0]) } }
            parts.append([days[min(max(meeting.day, 0), 6)], periods ?? "\(meeting.start)–\(meeting.end)"].joined(separator: " "))
            if !meeting.room.isEmpty { parts.append(meeting.room) }
        }
        return parts.joined(separator: " · ")
    }

    /// `試験 · 11/27 5–6限 · W9-202`, one line per exam or quiz of this
    /// course; a cancelled one is struck through, a changed one warned.
    @ViewBuilder
    private var examLines: some View {
        let exams = (detail?.exams ?? []).filter { ["exam", "quiz"].contains($0.kind) }
        ForEach(exams, id: \.id) { exam in
            Text(Self.examLine(exam))
                .font(.subheadline.weight(.medium))
                .foregroundStyle(Theme.warn)
                .strikethrough(exam.status == "cancelled")
            if let conflict = exam.conflict {
                ExamConflictLabel(conflict: conflict)
            }
        }
    }

    static func examLine(_ exam: ExamView) -> String {
        let day = exam.date.split(separator: "-").compactMap { Int($0) }
        var when = day.count == 3 ? "\(day[1])/\(day[2])" : exam.date
        if let periods = Format.periods(exam.periods) {
            when += " \(periods)"
        } else if let start = exam.startTime, let end = exam.endTime {
            when += " \(start)–\(end)"
        }
        return ([exam.kind == "quiz" ? t("Quiz") : t("Exam"), when] + (exam.room.isEmpty ? [] : [exam.room])).joined(separator: " · ")
    }

    @ViewBuilder
    private func links(title: String) -> some View {
        HStack(spacing: 8) {
            if let lms = detail?.lmsUrl {
                NavigationLink(value: WebRouter.PushedPage(site: .scienceLms, target: lms, title: title)) {
                    LinkChip(title: "LMS", icon: "globe")
                }
            }
            if let syllabus = detail?.syllabusUrl {
                NavigationLink(value: WebRouter.PushedPage(site: .syllabus, target: syllabus, title: title)) {
                    LinkChip(title: t("Syllabus"), icon: "doc.text")
                }
            }
        }
        .buttonStyle(PressableStyle())
    }
}

/// Core's rail labels (一般, 1, 2, 演習 1, …), as on the desktop lesson rail;
/// the chosen one stays in view. A dot marks where the course is (Core's
/// `currentSection`); a long press marks a lesson as the one for the coming
/// (or today's) class.
private struct LessonChips: View {
    let lessons: [LessonContent]
    let selection: Int?
    let current: Int?
    let classAt: Date?
    let select: (Int) -> Void
    let mark: (LessonContent) -> Void

    /// `11/27` in Tokyo time.
    private var classDay: String? {
        classAt.map { date in
            let parts = Calendar.tokyo.dateComponents([.month, .day], from: date)
            return "\(parts.month ?? 0)/\(parts.day ?? 0)"
        }
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 6) {
                    ForEach(lessons, id: \.section) { lesson in
                        chip(lesson)
                    }
                }
            }
            .onAppear { proxy.scrollTo(selection, anchor: .center) }
            .onChange(of: selection) { withAnimation(.smooth(duration: Theme.motionBase)) { proxy.scrollTo(selection, anchor: .center) } }
        }
        .popoverTip(LessonMarkTip(), arrowEdge: .top)
    }

    private func chip(_ lesson: LessonContent) -> some View {
        let selected = Int(lesson.section) == selection
        return VStack(spacing: 3) {
            Button {
                select(Int(lesson.section))
            } label: {
                Text(lesson.label)
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .lineLimit(1)
                    .frame(minWidth: 34, maxWidth: 160)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 7)
                    .foregroundStyle(selected ? Theme.accentInk : Theme.ink)
                    .background(selected ? Theme.accent : Theme.surface, in: Capsule())
                    .contentShape(.contextMenuPreview, Capsule())
            }
            .buttonStyle(PressableStyle())
            .contextMenu {
                if let classDay, lesson.sectionId != nil {
                    Button(t("Make this the {{day}} class", ["day": classDay]), systemImage: "checkmark.circle") { mark(lesson) }
                }
            }
            Circle()
                .fill(Int(lesson.section) == current ? Theme.accent : .clear)
                .frame(width: 5, height: 5)
                .accessibilityHidden(true)
        }
        .id(Int(lesson.section))
        .accessibilityValue(Int(lesson.section) == current ? Text(t("Current lesson")) : Text(""))
    }
}

/// One lesson's whole content, so it moves as one piece: its title, its text
/// (授業計画), its items in LMS order, and the assignments set in it.
private struct LessonPage: View {
    let lesson: LessonContent?
    let work: [AssignmentDetail]
    let files: CourseKey?
    @Binding var opened: Set<String>

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            // Headed by the plan's first line, as a desktop lesson is, else
            // by its title.
            if let heading = lesson.flatMap({ $0.summary.isEmpty ? $0.title : .some($0.summary) }), !heading.isEmpty {
                Text(heading)
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(Theme.ink)
            }
            if let summary = lesson?.summaryHtml, !summary.isEmpty {
                HTMLText(html: summary)
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
            }
            ForEach(CourseDetailScreen.groups(lesson?.items ?? []), id: \.first?.materialId) { group in
                if let item = group.first, group.count == 1, item.bodyHtml != nil {
                    InlineItem(item: item)
                } else {
                    Card {
                        ForEach(Array(group.enumerated()), id: \.element.materialId) { index, item in
                            if index > 0 { RowDivider() }
                            MaterialButton(title: item.title, kind: CourseDetailScreen.kind(item.kind), source: item.source, files: files)
                        }
                    }
                }
            }
            if !work.isEmpty {
                // The stack's spacing is the only gap above it (SectionLabel
                // would add its own top padding).
                Text(t("Assignments"))
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Theme.muted)
                    .padding(.horizontal, 16)
                ForEach(work, id: \.assignmentId) { assignment in
                    AssignmentCard(
                        assignment: assignment,
                        expanded: Binding(
                            get: { opened.contains(assignment.assignmentId) },
                            set: { if $0 { opened.insert(assignment.assignmentId) } else { opened.remove(assignment.assignmentId) } }))
                }
            }
            if lesson?.items.isEmpty != false && lesson?.summaryHtml.isEmpty != false && work.isEmpty {
                ContentUnavailableView(t("No materials for this lesson"), systemImage: "doc.text")
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}

private struct LinkChip: View {
    let title: String
    let icon: String

    var body: some View {
        Label(title, systemImage: icon)
            .font(.footnote.weight(.semibold))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .foregroundStyle(Theme.accent)
            .background(Theme.accentSoft, in: Capsule())
    }
}

/// A page, label or announcement shown in place, as on the desktop.
private struct InlineItem: View {
    let item: LessonItem

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch item.kind {
            case .page:
                Text(item.title).font(.headline).foregroundStyle(Theme.ink)
            case .forum:
                Label(item.title, systemImage: "megaphone").font(.headline).foregroundStyle(Theme.ink)
            default:
                EmptyView()
            }
            if let html = item.bodyHtml { HTMLText(html: html) }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }
}
