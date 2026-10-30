import IsctCore
import SwiftUI

/// 今日: what Core chose for this moment (the class in progress or tonight's
/// deadline), the stack of the next classes, then the quarter's assignments.
/// The shell renders `TodayView` and `AssignmentBoard` in order and never
/// sorts or filters. A list rather than a scroll view, so assignment rows
/// keep the system swipes.
struct TodayScreen: View {
    /// The workstation's dashboard (slotledger `approach/mobile-layout-by-width`):
    /// the moment's classes, exams and 新着 on the left, the assignments in
    /// their own column on the right, the class cards laid out side by side
    /// instead of stacked.
    var columns = false
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    /// Core's upcoming exams for the next two weeks.
    @State private var exams: [ExamView] = []

    /// The dashboard's list margin, which its header lines up with.
    private static let margin = Theme.gutter

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            if columns {
                VStack(alignment: .leading, spacing: 0) {
                    header.padding(.horizontal, Self.margin)
                    HStack(alignment: .top, spacing: 0) {
                        page { overview(context.date) }
                        page {
                            if model.today != nil { TodayAssignmentSections(now: context.date) }
                        }
                        .frame(width: 420)
                    }
                    .contentMargins(.horizontal, Self.margin, for: .scrollContent)
                    // Both columns start right under the header, level.
                    .contentMargins(.top, 4, for: .scrollContent)
                }
                .frame(maxWidth: 1180)
                .frame(maxWidth: .infinity)
            } else {
                page {
                    overview(context.date)
                    if model.today != nil {
                        TodayAssignmentSections(now: context.date)
                    }
                }
            }
        }
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .safeAreaInset(edge: .bottom) { UndoToast() }
        // What the page points at opens beside it on the workstation.
        .environment(\.push, PushAction { navigator.open($0) })
        .onAppear { model.syncHint(.open(view: .today)) }
        .task(id: "\(model.contentRevision)-\(model.today?.date ?? "")") {
            let loaded = await model.upcomingExams()
            if !Task.isCancelled { exams = loaded }
        }
    }

    private func page<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        List { content() }
            .listStyle(.insetGrouped)
            .listSectionSpacing(.compact)
            .scrollContentBackground(.hidden)
            .refreshable { model.refreshFromSchool() }
    }

    /// The moment: the header (above the columns on the dashboard), what needs
    /// the person, then Core's `TodayView` in order.
    private func overview(_ now: Date) -> some View {
        Group {
            if !columns { header }
            if let problem = model.problem {
                ProblemBanner(text: problem)
            } else if let problem = model.syncSnapshot?.problem {
                SyncProblemBanner(problem: problem)
            }
            if model.needsAccount { ConnectCard() }
            if let snapshot = model.syncSnapshot, !snapshot.activeRuns.isEmpty || snapshot.resetting,
                model.records.courses.isEmpty, model.records.assignments.isEmpty
            {
                FirstLoadCard(snapshot: snapshot)
            }
            if let today = model.today {
                TodayContent(today: today, now: now, exams: exams, spread: columns)
                    .transition(.opacity)
            } else if !model.needsAccount {
                SkeletonRows(rows: 4).transition(.opacity)
            }
        }
        .animation(.smooth(duration: Theme.motionBase), value: model.today == nil)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0))
    }

    private var header: some View {
        HStack(alignment: .bottom) {
            VStack(alignment: .leading, spacing: 2) {
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
                Text(t("common.today"))
                    .font(.largeTitle.bold())
                    .foregroundStyle(Theme.ink)
            }
            Spacer()
            // The workstation's status bar carries the ring.
            if !navigator.workbench { SyncRingButton() }
        }
        .padding(.top, 12)
    }

    private var subtitle: String {
        guard let today = model.today else { return "" }
        var parts = [Format.dayTitle(today.date)]
        if today.noClasses {
            parts.append(today.calendarNotice ?? t("common.holiday"))
        } else if let week = today.teachingWeek {
            parts.append(t("today.quarterWeek", ["quarter": Format.quarter(today.term), "week": week]))
        }
        return parts.joined(separator: " · ")
    }
}

/// 新着's label, with read all at its trailing end.
private struct RecentLabel: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            SectionLabel(t("today.new"))
            Spacer()
            Button(t("today.markAllAsRead")) {
                Task { await model.markAllRecentSeen() }
            }
            .font(.subheadline.weight(.semibold))
            .buttonStyle(.borderless)
            .padding(.horizontal, 16)
        }
    }
}

private struct TodayContent: View {
    let today: TodayView
    let now: Date
    let exams: [ExamView]
    /// The class cards side by side rather than stacked.
    var spread = false

    var body: some View {
        switch today.phase {
        case .beforeClasses, .betweenClasses:
            ClassStack(items: today.upcoming, now: now, spread: spread)
            UpcomingExams(exams: exams)
        case .inClass:
            if let hero = today.hero { CurrentClassHero(item: hero, now: now) }
            if !today.classMaterials.isEmpty || !today.classAssignments.isEmpty {
                SectionLabel(t("today.thisClass"))
                ThisClassCard(materials: today.classMaterials, assignments: today.classAssignments, now: now)
            }
            ClassStack(items: today.upcoming, now: now, spread: spread)
        case .afterClasses:
            if let hero = today.deadlineHero { TonightHero(item: hero, now: now) }
            ClassStack(items: today.upcoming, now: now, spread: spread)
            UpcomingExams(exams: exams)
        case .noClasses:
            UpcomingExams(exams: exams)
            ClassStack(items: today.upcoming, now: now, spread: spread)
            if !today.recent.isEmpty {
                RecentLabel()
                RecentCard(items: today.recent)
            }
        }
    }
}

// MARK: Class stack

/// The next class and up to two after it (Core's `upcoming`), stacked like
/// the lock screen's grouped notifications: swipe up for the next card, down
/// to go back. The stack keeps one height, so the page never moves while it
/// turns, and it owns every vertical swipe that starts on it: past either end
/// the front card only stretches, so the swipe can never become the page's
/// scroll or its pull to refresh.
private struct ClassStack: View {
    let items: [TodayClass]
    let now: Date
    /// The dashboard has room for every card: the first in full, the rest
    /// side by side under it, each under its own day.
    var spread = false
    @State private var front = 0
    @State private var pull: CGFloat = 0

    private static let cardHeight: CGFloat = 150
    /// How far each card behind shows below the one in front.
    private static let step: CGFloat = 16
    /// How much narrower each card behind is.
    private static let shrink: CGFloat = 0.07
    /// The pull that turns a card.
    private static let turn: CGFloat = 60

    var body: some View {
        if spread {
            spreadCards
        } else {
            stack
        }
    }

    @ViewBuilder
    private var spreadCards: some View {
        if let first = items.first {
            SectionLabel(Self.title(first, first: true))
            StackCard(item: first, now: now, leads: true)
                .frame(height: Self.cardHeight)
                .opensCourse(first.occurrence.courseId, zoom: "stack-0")
            if items.count > 1 {
                // A card's day is said only where it changes; the row keeps
                // one line for it when any of its cards needs one.
                let labelled = items.indices.dropFirst().filter { items[$0].occurrence.date != items[$0 - 1].occurrence.date }
                HStack(alignment: .top, spacing: 12) {
                    ForEach(Array(items.enumerated().dropFirst()), id: \.offset) { position, item in
                        VStack(alignment: .leading, spacing: 6) {
                            if labelled.contains(position) {
                                SectionLabel(Self.title(item, first: false))
                            } else if !labelled.isEmpty {
                                SectionLabel(" ").hidden()
                            }
                            StackCard(item: item, now: now, leads: false)
                                .frame(height: Self.cardHeight)
                                .opensCourse(item.occurrence.courseId, zoom: "stack-\(position)")
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var stack: some View {
        if !items.isEmpty {
            let index = min(front, items.count - 1)
            // 0…1 while a pull brings the next (up) or previous (down) card in.
            let forward = index < items.count - 1 ? min(max(-pull / Self.turn, 0), 1) : 0
            let back = index > 0 ? min(max(pull / Self.turn, 0), 1) : 0
            HStack {
                SectionLabel(Self.title(items[index], first: index == 0))
                Spacer()
                if items.count > 1 {
                    Text(verbatim: "\(index + 1) / \(items.count)")
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 16)
                        .padding(.top, 8)
                }
            }
            ZStack(alignment: .top) {
                ForEach(Array(items.enumerated()), id: \.offset) { position, item in
                    // Depth moves toward 0 as the pull brings the next card forward.
                    let depth = CGFloat(position - index) - forward + back
                    StackCard(item: item, now: now, leads: position == 0)
                        .frame(height: Self.cardHeight)
                        .opensCourse(item.occurrence.courseId, zoom: "stack-\(position)")
                        .scaleEffect(depth <= 0 ? 1 : 1 - depth * Self.shrink, anchor: .top)
                        .brightness(depth > 0 ? -0.04 * depth : 0)
                        .offset(y: offset(depth, isFront: position == index))
                        .opacity(depth <= -1 ? 0 : depth < 0 ? 1 + depth : 1)
                        .zIndex(Double(items.count) - Double(abs(depth)))
                        .allowsHitTesting(position == index)
                        .accessibilityHidden(position != index)
                }
            }
            .frame(height: Self.cardHeight + Self.step * CGFloat(min(items.count - 1, 2)), alignment: .top)
            .contentShape(Rectangle())
            .gesture(StackPan(
                changed: { travel in
                    let open = travel < 0 ? index < items.count - 1 : index > 0
                    // Past an end the card stretches a little and springs back.
                    pull = open ? travel : travel * 0.2
                },
                ended: { travel, velocity in
                    withAnimation(.smooth(duration: Theme.motionBase)) {
                        if travel < -Self.turn / 2 || velocity < -500 { front = min(index + 1, items.count - 1) }
                        if travel > Self.turn / 2 || velocity > 500 { front = max(index - 1, 0) }
                        pull = 0
                    }
                }))
            .accessibilityAction(named: t("today.nextClass")) { withAnimation(.smooth(duration: Theme.motionBase)) { front = min(index + 1, items.count - 1) } }
            .onChange(of: items.first?.startsAt) { front = 0 }
        }
    }

    /// The front card follows the finger; cards behind sit a step lower per
    /// level; a card already turned waits above, out of sight.
    private func offset(_ depth: CGFloat, isFront: Bool) -> CGFloat {
        if isFront { return pull < 0 ? pull : min(pull, Self.cardHeight / 3) }
        if depth < 0 { return depth * Self.cardHeight / 3 }
        return depth * Self.step
    }

    /// The card's day, said once above the stack: 次の授業 for the first
    /// class still to come today, then today, tomorrow or the date.
    private static func title(_ item: TodayClass, first: Bool) -> String {
        let exam = item.occurrence.kind == .exam
        let date = Format.civilDate(item.occurrence.date)
        if date.map(Calendar.tokyo.isDateInToday) == true {
            if first { return exam ? t("today.nextExam") : t("today.nextClass") }
            return t("today.laterToday")
        }
        if date.map(Calendar.tokyo.isDateInTomorrow) == true {
            return exam ? t("today.tomorrowsExam") : t("today.tomorrowsClass")
        }
        return Format.dayTitle(item.occurrence.date)
    }
}

/// One class in the stack: periods, start time and room in large figures,
/// and how long until it starts while that is today.
private struct StackCard: View {
    let item: TodayClass
    let now: Date
    let leads: Bool

    var body: some View {
        let tint = Color(hex: item.occurrence.color)
        let exam = item.occurrence.kind == .exam
        let quiz = exam && item.occurrence.quiz == true
        let starts = Format.date(item.startsAt)
        let today = starts.map { Calendar.tokyo.isDate($0, inSameDayAs: now) } == true
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(([quiz ? t("common.quiz") : exam ? t("common.exam") : nil, Format.periods(item.periods)] as [String?]).compactMap { $0 }.joined(separator: " · "))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(exam ? Theme.warn : tint)
                Spacer()
                if leads, today, let starts {
                    let minutes = max(Format.minutes(until: starts, now: now), 0)
                    TagChip(text: minutes < 60 ? t("today.countMinLeft", ["count": minutes]) : Format.time(starts),
                        ink: exam ? Theme.warn : tint, fill: Theme.surface)
                        .countingFigure(minutes)
                }
            }
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(item.occurrence.startTime)
                    .font(.display.monospacedDigit())
                Text(item.occurrence.room)
                    .font(.title2.bold())
            }
            .foregroundStyle(Theme.ink)
            .minimumScaleFactor(0.6)
            .lineLimit(1)
            Text(item.occurrence.title)
                .font(.title3.weight(.semibold))
                .lineLimit(1)
                .foregroundStyle(Theme.ink)
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .heroCard(tint: tint)
        // The mesh is translucent; an opaque base keeps the cards behind
        // from showing through, so the stack reads as separate cards. Its
        // shadow falls below only, onto the cards behind: none over the
        // label above the stack, and none at the top corners, where the side
        // shadow fades in only once the corners have turned.
        .background {
            let shape = RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous)
            ZStack {
                shape.fill(Theme.surface)
                    .shadow(color: .black.opacity(0.10), radius: 6, y: 2)
                    .mask {
                        VStack(spacing: 0) {
                            LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom)
                                .frame(height: Theme.radiusPanel * 2)
                            Rectangle()
                        }
                        .padding([.horizontal, .bottom], -16)
                    }
                shape.fill(Theme.surface)
            }
        }
    }
}

/// A vertical pan the page's own scrolling waits for, so a swipe that starts
/// on the stack turns its cards and never scrolls or refreshes the page.
/// SwiftUI's DragGesture cannot make the list's pan wait for it; a UIKit
/// failure requirement can. Horizontal swipes are left to the page.
private struct StackPan: UIGestureRecognizerRepresentable {
    var changed: (CGFloat) -> Void
    var ended: (_ travel: CGFloat, _ velocity: CGFloat) -> Void

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let pan = UIPanGestureRecognizer()
        pan.delegate = context.coordinator
        return pan
    }

    func handleUIGestureRecognizerAction(_ pan: UIPanGestureRecognizer, context: Context) {
        let travel = pan.translation(in: pan.view).y
        switch pan.state {
        case .changed: changed(travel)
        case .ended, .cancelled: ended(travel, pan.velocity(in: pan.view).y)
        default: break
        }
    }

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator { Coordinator() }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
            guard let pan = recognizer as? UIPanGestureRecognizer else { return false }
            let velocity = pan.velocity(in: pan.view)
            return abs(velocity.y) >= abs(velocity.x)
        }

        /// The list's pan, its pull to refresh among it, waits for this one.
        func gestureRecognizer(
            _ recognizer: UIGestureRecognizer, shouldBeRequiredToFailBy other: UIGestureRecognizer
        ) -> Bool {
            other.view is UIScrollView
        }

        func gestureRecognizer(
            _ recognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool {
            false
        }
    }
}

// MARK: Heroes

/// In class: minutes left, progress and end time.
private struct CurrentClassHero: View {
    let item: TodayClass
    let now: Date

    var body: some View {
        let tint = Color(hex: item.occurrence.color)
        let starts = Format.date(item.startsAt) ?? now
        let ends = Format.date(item.endsAt) ?? now
        let total = max(ends.timeIntervalSince(starts), 1)
        let progress = min(max(now.timeIntervalSince(starts) / total, 0), 1)
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                // The first room only, so the line never wraps; every room is
                // on the course page.
                Text(([inProgress(item.occurrence), Format.periods(item.periods), BlockText.firstRoom(item.occurrence.room)] as [String?]).compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(tint)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
                Spacer()
                Text(t("today.untilTime", ["time": item.occurrence.endTime]))
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(tint)
            }
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                // The figure sits where the language puts the count: あと 45 分,
                // 45 min left, 还剩 45 分钟.
                let words = t("today.countMinLeft").components(separatedBy: "{{count}}")
                let minutes = max(Format.minutes(until: ends, now: now), 0)
                if let before = words.first?.trimmingCharacters(in: .whitespaces), !before.isEmpty {
                    Text(before).font(.title3.weight(.semibold))
                }
                Text("\(minutes)")
                    .font(.display.monospacedDigit())
                    .countingFigure(minutes)
                if words.count > 1, case let after = words[1].trimmingCharacters(in: .whitespaces), !after.isEmpty {
                    Text(after).font(.title3.weight(.semibold))
                }
            }
            .foregroundStyle(Theme.ink)
            ProgressView(value: progress)
                .tint(tint)
                .animation(.smooth(duration: Theme.motionSlow), value: progress)
            Text(item.occurrence.title)
                .font(.title3.weight(.semibold))
                .lineLimit(2)
                .multilineTextAlignment(.leading)
                .foregroundStyle(Theme.ink)
        }
        .heroCard(tint: tint)
        .opensCourse(item.occurrence.courseId)
    }

    private func inProgress(_ occurrence: Occurrence) -> String {
        guard occurrence.kind == .exam else { return t("today.inClass") }
        return occurrence.quiz == true ? t("today.quizInProgress") : t("today.examInProgress")
    }
}

/// After classes: tonight's nearest deadline on the danger tint.
private struct TonightHero: View {
    let item: DeadlineItem
    let now: Date
    @Environment(\.push) private var push

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(t("today.dueTonight"))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Theme.danger)
                Spacer()
                DeadlineChip(item: item, now: now)
            }
            Text(Format.date(item.dueAt).map(Format.time) ?? Format.due(item.dueAt, now: now))
                .font(.display.monospacedDigit())
                .foregroundStyle(Theme.ink)
            Text(item.title).font(.title3.weight(.semibold)).foregroundStyle(Theme.ink)
            if let course = item.courseTitle {
                Text(course).font(.subheadline).foregroundStyle(Theme.muted)
            }
            if let url = item.sourceUrl {
                Button {
                    push(WebRouter.PushedPage(site: .scienceLms, target: url, title: item.title))
                } label: {
                    Label(t("today.openSubmissionPage"), systemImage: "arrow.up.forward.square")
                        .font(.body.weight(.semibold))
                        .padding(.horizontal, 16)
                        .padding(.vertical, 10)
                }
                .buttonStyle(CapsuleButtonStyle(level: .danger))
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}

// MARK: Sections

/// The whole hero card is the way into its course: it dips while pressed and
/// the course page grows out of it. A button that pushes rather than a
/// NavigationLink, which 今日's list would give a disclosure chevron.
private struct OpensCourse: ViewModifier {
    let id: String?
    var zoom = "hero"
    @Environment(\.push) private var push

    func body(content: Content) -> some View {
        if let id {
            Button {
                push(CourseRoute(id: id, zoom: zoom))
            } label: {
                content.contentShape(RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
            }
            .buttonStyle(PressableStyle())
            .courseZoomSource(zoom)
        } else {
            content
        }
    }
}

private extension View {
    func opensCourse(_ id: String?, zoom: String = "hero") -> some View { modifier(OpensCourse(id: id, zoom: zoom)) }
}

/// Opens the assignment's page, read from LMS by Core; long press offers
/// quick actions, among them the LMS submission page.
struct DeadlineLink<Content: View>: View {
    let item: DeadlineItem
    @ViewBuilder var label: Content
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push

    var body: some View {
        CardLink(route: AssignmentRoute(id: item.assignmentId)) {
            label
        }
        .contextMenu {
            // A NavigationLink inside a menu does not push reliably.
            if let course = item.courseId {
                Button(t("common.openCourse"), systemImage: "book") { push(CourseRoute(id: course)) }
            }
            if let url = item.sourceUrl {
                Button(t("common.submitInLMS"), systemImage: "arrow.up.forward.square") {
                    push(WebRouter.PushedPage(site: .scienceLms, target: url, title: item.title))
                }
            }
            Button {
                Task { await model.setPinned(item, !item.pinned) }
            } label: {
                Label(item.pinned ? t("common.unpin") : t("common.pin"), systemImage: "pin")
            }
            Button(role: .destructive) {
                Task { await model.setHidden(item, true) }
            } label: {
                Label(t("common.hide"), systemImage: "eye.slash")
            }
        }
    }
}

/// In class: the newest material and open work of this course.
private struct ThisClassCard: View {
    let materials: [MaterialItem]
    let assignments: [DeadlineItem]
    let now: Date

    var body: some View {
        Card {
            ForEach(Array(materials.enumerated()), id: \.element.materialId) { index, item in
                if index > 0 { RowDivider() }
                MaterialRow(item: item)
            }
            ForEach(Array(assignments.enumerated()), id: \.element.assignmentId) { index, item in
                if index > 0 || !materials.isEmpty { RowDivider() }
                DeadlineLink(item: item) {
                    Image(systemName: "doc.text").foregroundStyle(Theme.muted).frame(width: 16)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(item.title).foregroundStyle(Theme.ink).lineLimit(2)
                        Text(Format.due(item.dueAt, now: now)).font(.subheadline).foregroundStyle(Theme.muted)
                    }
                    Spacer(minLength: 8)
                    DeadlineChip(item: item, now: now)
                }
            }
        }
    }
}

struct MaterialRow: View {
    let item: MaterialItem

    var body: some View {
        MaterialOpener(title: item.title, source: item.url, files: nil, courseId: item.courseId) { opening in
            HStack(spacing: 12) {
                Image(systemName: icon).foregroundStyle(Theme.muted).frame(width: 16)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title).foregroundStyle(Theme.ink).lineLimit(2).multilineTextAlignment(.leading)
                    if let detail {
                        Text(detail).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                if opening {
                    ProgressView().controlSize(.small)
                } else if item.addedToday {
                    TagChip(text: t("common.addedToday"))
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
    }

    private var icon: String {
        switch item.kind {
        case "pdf": return "doc"
        case "video": return "play.rectangle"
        default: return "link"
        }
    }

    private var detail: String? { item.sectionLabel }
}

/// Exams in the next two weeks, as Core lists them; a row opens its course.
private struct UpcomingExams: View {
    let exams: [ExamView]

    var body: some View {
        if !exams.isEmpty {
            SectionLabel(t("common.exam"))
            Card {
                ForEach(Array(exams.enumerated()), id: \.element.key) { index, exam in
                    if index > 0 { RowDivider() }
                    row(exam)
                }
            }
        }
    }

    private func row(_ exam: ExamView) -> some View {
        CardLink(route: exam.courseId.map { CourseRoute(id: $0) }) {
            CourseSquare(color: exam.color)
            VStack(alignment: .leading, spacing: 2) {
                Text(exam.title.isEmpty ? exam.code : exam.title).foregroundStyle(Theme.ink).lineLimit(1)
                Text(detail(exam)).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(1)
                if let conflict = exam.conflict {
                    ExamConflictLabel(conflict: conflict)
                }
            }
            Spacer(minLength: 8)
        }
    }

    /// `10月26日（月） · 3–4限 · W531`, or the time when no period is given;
    /// a quiz says 小テスト first.
    private func detail(_ exam: ExamView) -> String {
        let time = Format.periods(exam.periods)
            ?? exam.startTime.map { start in [start, exam.endTime].compactMap { $0 }.joined(separator: "–") }
        return [exam.kind == "quiz" ? t("common.quiz") : nil, Format.dayTitle(exam.date), time, exam.room].compactMap { $0 }.filter { !$0.isEmpty }
            .joined(separator: " · ")
    }
}

/// Day off: grades and materials that changed recently. Opening one tells
/// Core it has been seen, so it leaves the list until it changes again.
private struct RecentCard: View {
    let items: [RecentItem]
    @Environment(AppModel.self) private var model

    var body: some View {
        Card {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                if index > 0 { RowDivider() }
                switch item {
                case let .grade(row):
                    recentRow(color: row.color, title: [row.title, row.grade].compactMap { $0 }.joined(separator: " · "),
                              detail: row.courseTitle, tag: TagChip(text: t("common.graded"), ink: Theme.ok, fill: Theme.okSoft),
                              courseId: row.courseId, seen: (.grade, row.assignmentId))
                case let .material(row):
                    recentRow(color: row.color, title: row.title, detail: row.courseTitle,
                              tag: TagChip(text: t("today.material")), courseId: row.courseId, seen: (.material, row.materialId))
                }
            }
        }
    }

    private func recentRow(color: String?, title: String, detail: String?, tag: TagChip, courseId: String?,
                           seen: (RecentKind, String)) -> some View {
        CardLink(route: courseId.map { CourseRoute(id: $0) }, opened: {
            Task { await model.markRecentSeen(seen.0, seen.1) }
        }) {
            CourseSquare(color: color)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).foregroundStyle(Theme.ink).lineLimit(2)
                if let detail { Text(detail).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(1) }
            }
            Spacer(minLength: 8)
            tag
        }
    }
}

// MARK: Feedback

struct ProblemBanner: View {
    let text: String
    @Environment(AppModel.self) private var model

    var body: some View {
        // One line and one way out (slotledger `approach/feedback-banner-toast`).
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.danger)
            Text(text).font(.subheadline).foregroundStyle(Theme.ink).lineLimit(2)
            Spacer()
            Button(t("common.retry")) {
                model.dismissProblem()
                model.refreshFromSchool()
            }
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(Theme.accent)
        }
        .padding(14)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }
}

/// First launch: the real empty Today with one sign-in card.
private struct ConnectCard: View {
    @Environment(\.push) private var push

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(t("common.connectSchoolAccounts")).font(.title3.weight(.bold)).foregroundStyle(Theme.ink)
            Text(t("today.readsYourTimetableAssignmentsAnd"))
                .font(.subheadline)
                .foregroundStyle(Theme.muted)
            Button {
                push(SettingsRoute.account(.science))
            } label: {
                Text(t("today.connectWithScienceTokyo"))
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
            }
            .buttonStyle(CapsuleButtonStyle())
        }
        .padding(18)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}

struct UndoToast: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let undo = model.undo {
            HStack {
                Text(t("app.hidTitle", ["title": undo.title])).lineLimit(1)
                Spacer()
                Button(t("common.undo")) { Task { await model.undoHide() } }.fontWeight(.semibold)
            }
            .font(.subheadline)
            .foregroundStyle(Theme.ink)
            .tint(Theme.accent)
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            // A floating layer takes the system material, not a black pill.
            .floatingGlass(in: Capsule())
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, 8)
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .task(id: undo) {
                try? await Task.sleep(for: .seconds(4))
                model.dismissUndo()
            }
        }
    }
}
