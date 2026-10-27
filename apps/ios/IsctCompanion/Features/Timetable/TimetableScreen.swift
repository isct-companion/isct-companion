import IsctCore
import SwiftUI
import TipKit

/// 時間割: a dated week. Blocks come from Core's `ScheduleView` for
/// Monday–Friday, whatever quarter the week is in; the title opens a picker
/// that names each week's quarter (`TimetableYear`). A long press marks one
/// session as an exam, a quiz or 休講 (choosing the checked mark again makes
/// it a plain class); adding and planning courses stay on desktop.
struct TimetableScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var monday = TimetableScreen.monday(of: Date())
    @State private var week: ScheduleView?
    @State private var year: TimetableYear?
    @State private var showsPicker = false
    @State private var dragOffset: CGFloat = 0

    /// Core's equal-row axis for the shown week.
    @State private var axis: TimetableAxis?
    @State private var gridHeight: CGFloat = 600

    /// Period rows share the height the screen leaves, so the week fits
    /// without scrolling (with a floor for very short windows).
    private var rowHeight: CGFloat {
        max(Self.minimumRow, ((gridHeight - Self.dayHeaderHeight) / CGFloat(axis?.length ?? 12.5)).rounded(.down))
    }
    private static let minimumRow: CGFloat = 30
    /// Weekday letters and date circles above the columns, plus their gap.
    private static let dayHeaderHeight: CGFloat = 54

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            TipView(TimetableMarkTip())
            grid
                .frame(maxHeight: .infinity, alignment: .top)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { gridHeight = $0 }
                .offset(x: dragOffset)
                // Alongside taps, so a block answers the moment it is touched.
                .simultaneousGesture(swipe)
        }
        .padding(.horizontal, Theme.gutter)
        .padding(.bottom, 8)
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .onChange(of: shownTerm, initial: true) { _, term in navigator.timetableTerm = term }
        .onAppear {
            model.syncHint(.open(view: .timetable))
            if axis == nil { axis = model.lastTimetableAxis }
            if week == nil { week = model.weeks[AppModel.civil(monday)] }
        }
        // A week read with its neighbours draws at once; the task below
        // still checks it against the current revision.
        .onChange(of: monday) { _, next in
            if let cached = model.weeks[AppModel.civil(next)] { week = cached }
        }
        .task(id: "\(monday.timeIntervalSince1970)-\(model.contentRevision)") {
            // A newer revision cancels this read: its empty answer must not
            // replace what is shown.
            let loaded = await model.week(starting: monday)
            guard !Task.isCancelled else { return }
            week = loaded
            let blocks = (loaded?.agendas ?? []).flatMap(\.occurrences).filter { [.course, .exam].contains($0.kind) }
            let earliest = blocks.map(\.startTime).min { TimetableAxis.minutes($0) < TimetableAxis.minutes($1) }
            let latest = blocks.map(\.endTime).max { TimetableAxis.minutes($0) < TimetableAxis.minutes($1) }
            let shape = await model.timetableAxis(earliest: earliest, latest: latest)
            guard !Task.isCancelled else { return }
            if let shape { axis = shape }
            guard let number = await model.academicYear(of: monday),
                year?.year != number || year?.quarters.isEmpty == true
            else { return }
            let calendar = await model.timetableYear(number)
            if !Task.isCancelled, let calendar { year = calendar }
        }
        .sheet(isPresented: $showsPicker) {
            WeekPicker(year: year, monday: monday) { picked in
                withAnimation(.snappy) { monday = picked }
                showsPicker = false
            }
            .presentationDetents([.medium, .large])
        }
    }

    // MARK: Header

    /// The week chip on its own row, free to take the full width; below it
    /// the date range and 今日. Weeks change by swiping the grid; the quarter's
    /// calendar export is a long press on the 時間割 tab.
    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                showsPicker = true
            } label: {
                // A chip, so the line reads as the way into the week picker.
                Label(subtitle, systemImage: "calendar")
                    .font(.footnote.weight(.semibold))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                    .foregroundStyle(Theme.accent)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(Theme.accentSoft, in: Capsule())
            }
            .buttonStyle(.plain)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityHint(t("Choose a week"))
            HStack(alignment: .center) {
                Button {
                    showsPicker = true
                } label: {
                    Text(range)
                        .font(.largeTitle.bold().monospacedDigit())
                        .foregroundStyle(Theme.ink)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .countingFigure(monday, down: false)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 8)
                todayButton
            }
        }
        .padding(.top, 12)
    }

    private var range: String {
        let friday = Calendar.tokyo.date(byAdding: .day, value: 4, to: monday) ?? monday
        return "\(short(monday)) – \(short(friday))"
    }

    /// "2026年度 · 3Q 第 2 週", from Core's week list; between quarters, 休み.
    private var subtitle: String {
        guard let year else { return t("Timetable") }
        let label = year.weeks.first { $0.monday == AppModel.civil(monday) }.flatMap { WeekPicker.label($0, in: year) }
        return [t("AY {{year}}", ["year": String(year.year)]), label].compactMap { $0 }.joined(separator: " · ")
    }

    private func short(_ date: Date) -> String {
        let parts = Calendar.tokyo.dateComponents([.month, .day], from: date)
        return "\(parts.month ?? 0)/\(parts.day ?? 0)"
    }

    /// The shown week's quarter, or this quarter between quarters.
    private var shownTerm: String? {
        year?.weeks.first { $0.monday == AppModel.civil(monday) }?.term ?? week?.term
    }

    /// 今日 rests while this week is shown, which doubles as "you are here".
    private var todayButton: some View {
        let thisWeek = TimetableScreen.monday(of: Date())
        let atThisWeek = monday == thisWeek
        return Button {
            withAnimation(.snappy) { monday = thisWeek }
        } label: {
            Text(t("Today"))
                .font(.subheadline.weight(.semibold))
                .padding(.horizontal, 16)
                .frame(height: 40)
        }
        .buttonStyle(CapsuleButtonStyle(level: .secondary))
        .disabled(atThisWeek)
        .opacity(atThisWeek ? 0.35 : 1)
        .accessibilityLabel(t("Back to this week"))
    }

    private func shift(_ weeks: Int) {
        monday = Calendar.tokyo.date(byAdding: .day, value: 7 * weeks, to: monday) ?? monday
    }

    private var swipe: some Gesture {
        DragGesture(minimumDistance: 24)
            .onChanged { value in
                if abs(value.translation.width) > abs(value.translation.height) {
                    dragOffset = value.translation.width / 3
                }
            }
            .onEnded { value in
                withAnimation(.snappy) { dragOffset = 0 }
                guard abs(value.translation.width) > abs(value.translation.height),
                    abs(value.translation.width) > 60
                else { return }
                shift(value.translation.width < 0 ? 1 : -1)
            }
    }

    // MARK: Grid

    private var days: [Date] {
        (0..<5).compactMap { Calendar.tokyo.date(byAdding: .day, value: $0, to: monday) }
    }

    private static let columnSpacing: CGFloat = 3
    private static let periodWidth: CGFloat = 26

    private var grid: some View {
        let height = rowHeight * CGFloat(axis?.length ?? 12.5)
        return VStack(spacing: 6) {
            HStack(spacing: Self.columnSpacing) {
                Color.clear.frame(width: Self.periodWidth, height: 1)
                ForEach(days, id: \.self) { day in dayHeader(day).frame(maxWidth: .infinity) }
            }
            HStack(alignment: .top, spacing: Self.columnSpacing) {
                periodColumn.frame(width: Self.periodWidth, height: height, alignment: .top)
                ForEach(days, id: \.self) { day in
                    dayColumn(day).frame(maxWidth: .infinity).frame(height: height)
                }
            }
        }
    }

    private func dayHeader(_ day: Date) -> some View {
        let parts = Calendar.tokyo.dateComponents([.weekday, .day], from: day)
        let today = Calendar.tokyo.isDateInToday(day)
        return VStack(spacing: 2) {
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

    /// Period numbers at their rows; each block's start time under its first.
    private var periodColumn: some View {
        ZStack(alignment: .top) {
            ForEach(axis?.rows ?? [], id: \.top) { row in
                if let periods = row.periods, periods.count == 2 {
                    let first = Int(periods[0])
                    ForEach(first...max(first, Int(periods[1])), id: \.self) { period in
                        periodLabel(period, row: row, first: period == first)
                            .padding(.top, (row.top + Double(period - first)) * rowHeight)
                            .frame(maxHeight: .infinity, alignment: .top)
                    }
                }
            }
        }
    }

    private func periodLabel(_ period: Int, row: AxisRow, first: Bool) -> some View {
        VStack(spacing: 0) {
            Text("\(period)").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
            if first {
                Text(row.startTime.replacingOccurrences(of: "^0", with: "", options: .regularExpression))
                    .font(.tick.monospacedDigit())
                    .foregroundStyle(Theme.muted)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private func agenda(_ day: Date) -> DatedAgenda? {
        let civil = AppModel.civil(day)
        return week?.agendas.first { $0.date == civil }
    }

    private func dayColumn(_ day: Date) -> some View {
        let agenda = agenda(day)
        let today = Calendar.tokyo.isDateInToday(day)
        let blocks = (agenda?.occurrences ?? []).filter { [.course, .exam].contains($0.kind) }
        let holiday = agenda?.noClasses == true
        let shape = RoundedRectangle(cornerRadius: Theme.radiusColumn, style: .continuous)
        // A day without classes is a flat grey column; stripes belong to the
        // classes that do not take place.
        return ZStack(alignment: .top) {
            shape.fill(holiday ? Theme.mix(Theme.muted, 0.08, into: Theme.surface) : Theme.surface)
            if let axis { RowRules(axis: axis, rowHeight: rowHeight).clipShape(shape) }
            if let axis {
                GeometryReader { column in
                    ForEach(blocks, id: \.key) { block in
                        // Core's grid: a block timed inside periods fills the
                        // periods it touches, overlapping blocks share lanes.
                        let top = axis.offset(block.grid?.startTime ?? block.startTime) * rowHeight
                        let bottom = axis.offset(block.grid?.endTime ?? block.endTime) * rowHeight
                        let lanes = CGFloat(block.grid?.lanes ?? 1)
                        let width = column.size.width / lanes
                        blockView(block, height: max(bottom - top - 4, 24), holiday: holiday, narrow: lanes > 1)
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

    /// One block (slotledger `approach/timetable-*`): a solid course fill,
    /// deeper for an exam; a ring only for the class in progress; an exam or
    /// quiz tag above the title; a class that does not take place striped.
    private func blockView(_ block: Occurrence, height: CGFloat, holiday: Bool, narrow: Bool) -> some View {
        let planned = block.planned == true
        let cancelled = block.cancelled == true || holiday
        let exam = block.kind == .exam && !cancelled
        let quiz = exam && block.quiz == true
        let current = isCurrent(block)
        let tint = Color(hex: block.color)
        let shape = RoundedRectangle(cornerRadius: Theme.radiusBlock, style: .continuous)
        let tagged = exam && !narrow
        let content = VStack(alignment: .leading, spacing: BlockText.spacing) {
            if tagged {
                HStack(spacing: 3) {
                    ExamTag(quiz: quiz, tint: tint)
                    if block.sourceChanged == true {
                        Image(systemName: "exclamationmark.circle.fill")
                            .foregroundStyle(Theme.warn)
                            .accessibilityLabel(t("Timetable changed"))
                    }
                }
                .font(BlockText.titleFont)
            }
            Text(block.title)
                .strikethrough(cancelled)
                .font(BlockText.titleFont)
                .lineLimit(BlockText.titleLines(height: height, tagged: tagged, footer: !narrow))
            if !narrow {
                Spacer(minLength: 0)
                bottomLine(block, planned: planned, cancelled: cancelled)
                    .font(BlockText.roomFont)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        }
        .foregroundStyle(cancelled ? Theme.muted : Theme.courseInk(tint))
        .padding(BlockText.padding)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .frame(height: height)
        .background {
            if cancelled {
                shape.fill(Theme.mix(Theme.muted, 0.10, into: Theme.surface))
                    .overlay { Stripes().clipShape(shape) }
            } else if planned {
                shape.fill(Theme.surface)
            } else {
                shape.fill(Theme.mix(tint, exam && !quiz ? Theme.tintExam : Theme.tintFill, into: Theme.surface))
            }
        }
        .overlay {
            if current && !cancelled {
                shape.stroke(tint, lineWidth: 2)
            } else if planned && !cancelled {
                shape.stroke(Theme.mix(tint, 0.45, into: Theme.surface), lineWidth: 1.5)
            }
        }
        .contentShape(shape)
        .padding(.horizontal, 2)
        return Group {
            if let id = block.courseId {
                NavigationLink(value: CourseRoute(id: id, zoom: block.key)) { content }
                    .buttonStyle(PressableStyle())
                    .courseZoomSource(block.key)
            } else {
                content
            }
        }
        // Planned courses have no sessions of their own to mark.
        .sessionMarks(block, enabled: !planned)
    }

    /// 計画, 休講, the real times of a block drawn over whole periods, or the
    /// first room; every room stays on the course page.
    @ViewBuilder
    private func bottomLine(_ block: Occurrence, planned: Bool, cancelled: Bool) -> some View {
        if planned {
            Text(t("Planned"))
        } else if cancelled {
            Text(t("No class"))
        } else if let grid = block.grid,
            grid.startTime != block.startTime || grid.endTime != block.endTime
        {
            Text("\(Self.clock(block.startTime))–\(Self.clock(block.endTime))").monospacedDigit()
        } else {
            Text(BlockText.firstRoom(block.room))
        }
    }

    /// "09:30" as 9:30, as the period column writes times.
    private static func clock(_ time: String) -> String {
        time.replacingOccurrences(of: "^0", with: "", options: .regularExpression)
    }

    private func isCurrent(_ block: Occurrence) -> Bool {
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

/// What the 時間割 tab's long press offers: the shown quarter as an .ics.
enum CalendarExportPick: Hashable {
    case courses, coursesAndPlans

    var scope: CalendarExportScope { self == .courses ? .courses : .coursesAndPlans }
}

/// The export panel's rows; the workstation's rail row shows the same in a menu.
struct CalendarExportRows: View {
    let term: String
    var highlighted: CalendarExportPick?
    var report: ((CalendarExportPick, CGRect) -> Void)?
    let pick: (CalendarExportPick) -> Void

    var body: some View {
        Text(t("Export ICS") + " · " + Format.quarter(term))
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 14)
            .padding(.top, 6)
            .padding(.bottom, 2)
        TabPressRow(choice: CalendarExportPick.courses, title: t("Courses only"), icon: "calendar",
                    highlighted: highlighted, report: report, pick: pick)
        TabPressRow(choice: CalendarExportPick.coursesAndPlans, title: t("Courses + personal plans"),
                    icon: "calendar.badge.plus", highlighted: highlighted, report: report, pick: pick)
    }
}

/// A quarter's calendar, written by Core as the desktop's ICSを書き出す, in a
/// temporary file for the share sheet (Files, AirDrop, Mail; opened from
/// Files it adds to Calendar).
struct QuarterCalendarFile: Identifiable {
    let url: URL
    var id: URL { url }

    static func write(model: AppModel, term: String, pick: CalendarExportPick) async throws -> Self {
        let text = try await model.quarterIcs(term: term, scope: pick.scope)
        let suffix = pick == .coursesAndPlans ? "-with-plans" : ""
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("IsctCompanion-\(term)\(suffix).ics")
        try Data(text.utf8).write(to: url, options: .atomic)
        return Self(url: url)
    }
}

/// The system share sheet for a file, presented from the root.
struct ShareSheet: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
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

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            if let offset = axis.now(context.date) {
                HStack(spacing: 0) {
                    Circle().fill(Theme.danger).frame(width: 8, height: 8)
                    Rectangle().fill(Theme.danger).frame(height: 2)
                }
                .padding(.top, offset * rowHeight - 4)
                .frame(maxHeight: .infinity, alignment: .top)
                .animation(.smooth, value: offset)
            }
        }
        .allowsHitTesting(false)
    }
}

/// Diagonal stripes on a class that does not take place.
private struct Stripes: View {
    var body: some View {
        Canvas { context, size in
            var path = Path()
            var x: CGFloat = -size.height
            while x < size.width {
                path.move(to: CGPoint(x: x, y: size.height))
                path.addLine(to: CGPoint(x: x + size.height, y: 0))
                x += 8
            }
            context.stroke(path, with: .color(Theme.line), lineWidth: 3)
        }
        .allowsHitTesting(false)
    }
}
