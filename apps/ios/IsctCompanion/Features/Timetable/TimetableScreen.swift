import IsctCore
import SwiftUI

/// 時間割: a dated week. Blocks come from Core's `ScheduleView` for
/// Monday–Friday, whatever quarter the week is in; the title opens a picker
/// that names each week's quarter (`TimetableYear`). A long press marks one
/// session as an exam, a quiz or 休講 (choosing the checked mark again makes
/// it a plain class); adding and planning courses stay on desktop.
struct TimetableScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var monday = TimetableScreen.monday(of: AppClock.now)
    @State private var yearSnapshot: (calendar: TimetableYear, revision: Int)?
    @State private var showsPicker = false
    @State private var dragOffset: CGFloat = 0
    @State private var gridWidth: CGFloat = 390
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Core's equal-row axis for the shown week.
    @State private var axis: TimetableAxis?
    @State private var gridHeight: CGFloat = 600
    @State private var headerHeight: CGFloat = 0
    @State private var weekdayRowHeight: CGFloat = 0

    /// Period rows share the height the screen leaves, so the week fits
    /// without scrolling (with a floor for very short windows).
    private var rowHeight: CGFloat {
        max(Self.minimumRow, ((gridHeight - gridHeaderHeight) / CGFloat(axis?.length ?? 12.5)).rounded(.down))
    }

    /// The page at least as tall as the header over a week at the row floor,
    /// so a short window (split view, a large text size) scrolls to the last
    /// period instead of cutting it off. Built from the floor, not the drawn
    /// grid, so the page's height never feeds back into the rows.
    private func pageHeight(_ window: CGFloat) -> CGFloat {
        let floor = gridHeaderHeight + Self.minimumRow * CGFloat(axis?.length ?? 12.5)
        return max(window, headerHeight + Theme.space6 + floor + Theme.space4)
    }

    private static let minimumRow: CGFloat = 30
    /// Budget for the weekday row and its gap, growing with Dynamic Type.
    private var gridHeaderHeight: CGFloat {
        max(54, weekdayRowHeight + Theme.space3)
    }

    var body: some View {
        let weekRevision = model.timetableWeeks.revision
        // A scroll view for the pull that refreshes from the school; the page
        // is one screen tall and scrolls only past the row floor.
        ScrollView {
            VStack(alignment: .leading, spacing: Theme.space6) {
                let thisWeek = Self.monday(of: AppClock.now)
                TimetableHeader(
                    monday: monday,
                    year: year,
                    exportTerm: shownTerm,
                    atThisWeek: monday == thisWeek,
                    openPicker: { showsPicker = true },
                    toToday: { withAnimation(Motion.base(reduceMotion)) { show(thisWeek) } }
                )
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { headerHeight = $0 }
                pager
                    .frame(maxHeight: .infinity, alignment: .top)
                    .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
                        gridHeight = size.height
                        gridWidth = size.width
                    }
                    // The pager offsets each week; the UIKit pan fails a block's
                    // tap once it begins, so a swipe never opens a course.
                    .gesture(PagePan(moved: follow, ended: release))
            }
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, Theme.space4)
            .containerRelativeFrame(.vertical, alignment: .top) { window, _ in pageHeight(window) }
        }
        .scrollBounceBehavior(.always, axes: .vertical)
        .refreshable { model.refreshFromSchool() }
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .onChange(of: shownTerm, initial: true) { _, term in navigator.timetableTerm = term }
        .onChange(of: navigator.timetableDate, initial: true) { _, date in
            if let date, let day = AppModel.date(date + "T00:00:00+09:00") {
                monday = Self.monday(of: day)
                navigator.timetableDate = nil
            }
        }
        .onAppear {
            model.syncHint(.open(view: .timetable))
            if axis == nil { axis = model.lastTimetableAxis }
        }
        .task(id: "\(monday.timeIntervalSince1970)-\(weekRevision)-\(model.today?.term ?? "")") { [monday] in
            guard self.monday == monday, model.timetableWeeks.revision == weekRevision else { return }
            guard let loaded = await model.week(starting: monday), !Task.isCancelled else { return }
            let blocks = loaded.agendas.flatMap(\.occurrences).filter { [.course, .exam].contains($0.kind) }
            let earliest = blocks.map(\.startTime).min { TimetableAxis.minutes($0) < TimetableAxis.minutes($1) }
            let latest = blocks.map(\.endTime).max { TimetableAxis.minutes($0) < TimetableAxis.minutes($1) }
            let shape = await model.timetableAxis(earliest: earliest, latest: latest)
            guard !Task.isCancelled, self.monday == monday, model.timetableWeeks.revision == weekRevision else { return }
            if let shape {
                axis = shape
                model.lastTimetableAxis = shape
            }
            guard let number = await model.academicYear(of: monday),
                year?.year != number || yearSnapshot?.revision != weekRevision || year?.quarters.isEmpty == true
            else { return }
            let calendar = await model.timetableYear(number)
            if !Task.isCancelled, self.monday == monday, model.timetableWeeks.revision == weekRevision, let calendar {
                yearSnapshot = (calendar, weekRevision)
            }
        }
        .sheet(isPresented: $showsPicker) {
            WeekPicker(year: year, monday: monday) { picked in
                withAnimation(Motion.base(reduceMotion)) { show(picked) }
                showsPicker = false
            }
            .presentationDetents([.medium, .large])
        }
    }

    private var year: TimetableYear? { yearSnapshot?.calendar }

    /// The shown week's quarter, once its year reflects the current weeks;
    /// none between quarters.
    private var shownTerm: String? {
        guard yearSnapshot?.revision == model.timetableWeeks.revision else { return nil }
        return year?.weeks.first { $0.monday == AppModel.civil(monday) }?.term
    }

    private func shift(_ weeks: Int) {
        show(Calendar.tokyo.date(byAdding: .day, value: 7 * weeks, to: monday) ?? monday)
    }

    /// The week and its blocks change in the same update. Set apart (the
    /// blocks a frame later, from `onChange`), the grid slid in empty and its
    /// cards popped in where they stand.
    private func show(_ next: Date) {
        monday = next
    }

    /// The weeks lie side by side and follow the finger; past the threshold
    /// the strip glides on to the neighbouring week, otherwise it settles
    /// back (slotledger `approach/motion-week-swipe`). With Reduce Motion the
    /// grid stays put and the week cross-fades.
    private func follow(_ dx: CGFloat) {
        if !reduceMotion { dragOffset = dx }
    }

    /// Past 60 pt, or thrown far enough to coast past half the grid, turns
    /// the week the grid was pulled; a throw back the other way settles it.
    private func release(_ dx: CGFloat, velocity: CGFloat) {
        let end = dx + Self.coast(velocity)
        guard abs(dx) > 60 || abs(end) > gridWidth / 2, (end < 0) == (dx < 0) else {
            withAnimation(Motion.base(reduceMotion)) { dragOffset = 0 }
            return
        }
        turn(dx < 0 ? 1 : -1)
    }

    /// How far a release at this velocity (pt/s) coasts at a scroll view's
    /// normal deceleration (the projection from WWDC18 "Designing Fluid
    /// Interfaces"), standing in for `DragGesture`'s predicted end.
    private static func coast(_ velocity: CGFloat) -> CGFloat {
        let rate = UIScrollView.DecelerationRate.normal.rawValue
        return velocity / 1000 * rate / (1 - rate)
    }

    private func turn(_ step: Int) {
        guard !reduceMotion else {
            withAnimation(Motion.fast(true)) { shift(step) }
            return
        }
        // The strip of weeks carries on to the neighbour; once it rests there,
        // the neighbour becomes the middle week without anything moving.
        let page = gridWidth + Self.weekGap
        withAnimation(.smooth(duration: Theme.motionSlow)) { dragOffset = -CGFloat(step) * page } completion: {
            var settle = Transaction()
            settle.disablesAnimations = true
            withTransaction(settle) {
                shift(step)
                dragOffset = 0
            }
        }
    }

    // MARK: Grid

    private func days(from start: Date) -> [Date] {
        (0..<5).compactMap { Calendar.tokyo.date(byAdding: .day, value: $0, to: start) }
    }

    /// Space between two weeks while they pass: past the gutter, so a
    /// neighbour never peeks in at rest.
    private static let weekGap: CGFloat = 2 * Theme.gutter

    /// The previous, shown and next weeks in a row, one page apart. The
    /// neighbours draw from the weeks already read around this one (a
    /// skeleton otherwise), so a swipe shows the next week as it comes in.
    private var pager: some View {
        let page = gridWidth + Self.weekGap
        return ZStack(alignment: .topLeading) {
            ForEach([-1, 0, 1], id: \.self) { slot in
                let start = Calendar.tokyo.date(byAdding: .day, value: 7 * slot, to: monday) ?? monday
                grid(start, week: model.timetableWeeks[start])
                    .offset(x: CGFloat(slot) * page + dragOffset)
                    .allowsHitTesting(slot == 0)
                    .accessibilityHidden(slot != 0)
            }
        }
        // Clipped at the screen's edge, not the grid's, so a passing week
        // shows all the way out and never draws over the rail beside the page.
        .padding(.horizontal, Theme.gutter)
        .clipped()
        .padding(.horizontal, -Theme.gutter)
    }

    private static let columnSpacing: CGFloat = 3
    private static let periodWidth: CGFloat = 26

    private func grid(_ start: Date, week: ScheduleView?) -> some View {
        let height = rowHeight * CGFloat(axis?.length ?? 12.5)
        let days = days(from: start)
        return VStack(spacing: Theme.space3) {
            // The weekday group steps to the weeks a swipe turns to. The
            // pager hides its neighbours from accessibility.
            HStack(spacing: Self.columnSpacing) {
                Color.clear.frame(width: Self.periodWidth, height: 1)
                ForEach(days, id: \.self) { day in dayHeader(day).frame(maxWidth: .infinity) }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                if start == monday { weekdayRowHeight = height }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(weekDateRange(from: start))
            .accessibilityAdjustableAction { direction in
                switch direction {
                case .increment: turn(1)
                case .decrement: turn(-1)
                @unknown default: break
                }
            }
            .accessibilityAction(named: Text(t("today.previousWeek"))) { turn(-1) }
            .accessibilityAction(named: Text(t("today.nextWeek"))) { turn(1) }
            HStack(alignment: .top, spacing: Self.columnSpacing) {
                periodColumn.frame(width: Self.periodWidth, height: height, alignment: .top)
                ForEach(days, id: \.self) { day in
                    dayColumn(day, week: week).frame(maxWidth: .infinity).frame(height: height)
                }
            }
        }
    }

    private func dayHeader(_ day: Date) -> some View {
        let parts = Calendar.tokyo.dateComponents([.weekday, .day], from: day)
        let today = AppClock.isToday(day)
        return VStack(spacing: Theme.space1) {
            Text(Format.weekdays[(parts.weekday ?? 1) - 1])
                .font(.caption)
                .foregroundStyle(Theme.muted)
            Text("\(parts.day ?? 0)")
                .font(.headline.monospacedDigit())
                .foregroundStyle(today ? Theme.accentInk : Theme.ink)
                .frame(width: 30, height: 30)
                .background(today ? Theme.accent : .clear, in: Circle())
        }
    }

    /// Period numbers at their rows; each block's start time under its first
    /// period and its end time under its last.
    private var periodColumn: some View {
        ZStack(alignment: .top) {
            ForEach(axis?.rows ?? [], id: \.top) { row in
                if let periods = row.periods, periods.count == 2 {
                    let first = Int(periods[0])
                    let last = max(first, Int(periods[1]))
                    ForEach(first...last, id: \.self) { period in
                        periodLabel(period, row: row, first: period == first, last: period == last)
                            .padding(.top, (row.top + Double(period - first)) * rowHeight)
                            .frame(maxHeight: .infinity, alignment: .top)
                    }
                }
            }
        }
    }

    private func periodLabel(_ period: Int, row: AxisRow, first: Bool, last: Bool) -> some View {
        VStack(spacing: 0) {
            Text("\(period)").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
            if first { periodTime(row.startTime) }
            if last { periodTime(row.endTime) }
        }
        .frame(maxWidth: .infinity)
    }

    private func periodTime(_ time: String) -> some View {
        Text(time.replacingOccurrences(of: "^0", with: "", options: .regularExpression))
            .font(.tick.monospacedDigit())
            .foregroundStyle(Theme.muted)
            .lineLimit(1)
            .minimumScaleFactor(0.8)
    }

    private func agenda(_ day: Date, in week: ScheduleView?) -> DatedAgenda? {
        let civil = AppModel.civil(day)
        return week?.agendas.first { $0.date == civil }
    }

    private func dayColumn(_ day: Date, week: ScheduleView?) -> some View {
        let agenda = agenda(day, in: week)
        let today = AppClock.isToday(day)
        let blocks = (agenda?.occurrences ?? []).filter { [.course, .exam].contains($0.kind) }
        let holiday = agenda?.noClasses == true
        let shape = RoundedRectangle(cornerRadius: Theme.radiusColumn, style: .continuous)
        // A day without classes is a flat grey column; stripes belong to the
        // classes that do not take place.
        return ZStack(alignment: .top) {
            shape.fill(holiday ? Theme.mix(Theme.muted, 0.08, into: Theme.surface) : Theme.surface)
            if let axis { RowRules(axis: axis, rowHeight: rowHeight).clipShape(shape) }
            if week == nil {
                skeleton(day).clipShape(shape)
            }
            if let axis {
                GeometryReader { column in
                    ForEach(blocks, id: \.key) { block in
                        // Core's grid: a block timed inside periods fills the
                        // periods it touches, overlapping blocks share lanes.
                        let top = axis.offset(block.grid?.startTime ?? block.startTime) * rowHeight
                        let bottom = axis.offset(block.grid?.endTime ?? block.endTime) * rowHeight
                        let lanes = CGFloat(block.grid?.lanes ?? 1)
                        let width = column.size.width / lanes
                        blockView(block, height: max(bottom - top - 4, 24), narrow: lanes > 1)
                            .frame(width: width)
                            // Placed by layout, not by offset, so where a block
                            // is drawn is where it answers a tap.
                            .padding(.leading, width * CGFloat(block.grid?.lane ?? 0))
                            .padding(.top, top + 2)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    }
                }
                if today { NowLine(axis: axis, rowHeight: rowHeight) }
            }
        }
    }

    /// Grey stand-ins for a week not read yet, two blocks a day at rows that
    /// shift by weekday so the grid does not read as a real timetable.
    private func skeleton(_ day: Date) -> some View {
        let weekday = Calendar.tokyo.component(.weekday, from: day)
        return ZStack(alignment: .topLeading) {
            ForEach([weekday % 3 * 2, weekday % 3 * 2 + 4], id: \.self) { row in
                RoundedRectangle(cornerRadius: Theme.radiusBlock, style: .continuous)
                    .fill(Theme.surface2)
                    .frame(height: max(rowHeight * 2 - 4, 24))
                    .padding(.horizontal, Theme.space1)
                    .padding(.top, CGFloat(row) * rowHeight + 2)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .transition(.opacity)
        .accessibilityHidden(true)
    }

    func isCurrent(_ block: Occurrence) -> Bool {
        guard let today = model.today, today.phase == .inClass, let hero = today.hero else { return false }
        return hero.occurrence.key == block.key
    }

    static func monday(of date: Date) -> Date {
        let calendar = Calendar.tokyo
        let start = calendar.startOfDay(for: date)
        let weekday = calendar.component(.weekday, from: start)
        // Saturday and Sunday look ahead to the coming week.
        let offset = weekday == 1 ? 1 : weekday == 7 ? 2 : 2 - weekday
        return calendar.date(byAdding: .day, value: offset, to: start) ?? start
    }
}

/// One rule after each teaching block (1–2, 3–4, …), never inside one, so a
/// two-period class is not cut in half; the lunch band is shaded.
private struct RowRules: View {
    let axis: TimetableAxis
    let rowHeight: CGFloat

    var body: some View {
        Canvas { context, size in
            var rules = Path()
            for row in axis.rows {
                let top = row.top * rowHeight
                let bottom = row.bottom * rowHeight
                if row.periods == nil {
                    context.fill(Path(CGRect(x: 0, y: top, width: size.width, height: bottom - top)), with: .color(Theme.surface2))
                } else if bottom < size.height - 1 {
                    rules.move(to: CGPoint(x: 0, y: bottom))
                    rules.addLine(to: CGPoint(x: size.width, y: bottom))
                }
            }
            context.stroke(rules, with: .color(Theme.line), lineWidth: 1)
        }
        .allowsHitTesting(false)
    }
}

/// The current time across today's column, moving with the clock.
private struct NowLine: View {
    let axis: TimetableAxis
    let rowHeight: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            if let offset = axis.now(AppClock.at(context.date)) {
                HStack(spacing: 0) {
                    Circle().fill(Theme.danger).frame(width: 8, height: 8)
                    Rectangle().fill(Theme.danger).frame(height: 2)
                }
                .padding(.top, offset * rowHeight - 4)
                .frame(maxHeight: .infinity, alignment: .top)
                .animation(reduceMotion ? nil : .smooth(duration: Theme.motionSlow), value: offset)
            }
        }
        .allowsHitTesting(false)
    }
}
