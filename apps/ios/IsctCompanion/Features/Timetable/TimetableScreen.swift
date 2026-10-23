import IsctCore
import SwiftUI
import TipKit

/// 時間割: a dated week. Blocks come from Core's `ScheduleView` for
/// Monday–Friday, whatever quarter the week is in; the title opens a picker
/// that names each week's quarter (`TimetableYear`). A long press marks one
/// session as an exam, 休講 or a normal class; adding and planning courses
/// stay on desktop.
struct TimetableScreen: View {
    @Environment(AppModel.self) private var model
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
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .onAppear {
            model.syncHint(.open(view: .timetable))
            if axis == nil { axis = model.lastTimetableAxis }
            if week == nil { week = model.weeks[AppModel.civil(monday)] }
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

    private var header: some View {
        HStack(alignment: .bottom) {
            Button {
                showsPicker = true
            } label: {
                VStack(alignment: .leading, spacing: 6) {
                    // A chip, so the line reads as the way into the week picker.
                    Label(subtitle, systemImage: "calendar")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Theme.accent)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(Theme.accentSoft, in: Capsule())
                    Text(range)
                        .font(.largeTitle.bold().monospacedDigit())
                        .foregroundStyle(Theme.ink)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .countingFigure(monday, down: false)
                }
            }
            .buttonStyle(.plain)
            .accessibilityHint(t("Choose a week"))
            Spacer(minLength: 8)
            weekStepper
        }
        .padding(.top, 12)
        .padding(.horizontal, 4)
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

    /// ‹ · 今日 · › in one capsule; 今日 rests while this week is shown.
    private var weekStepper: some View {
        let thisWeek = TimetableScreen.monday(of: Date())
        let atThisWeek = monday == thisWeek
        return HStack(spacing: 0) {
            stepperSegment(label: t("Previous week")) { shift(-1) } content: {
                Image(systemName: "chevron.left").font(.body.weight(.semibold)).frame(width: 40)
            }
            stepperSegment(label: t("Back to this week")) {
                withAnimation(.snappy) { monday = thisWeek }
            } content: {
                Text(t("Today")).font(.subheadline.weight(.semibold)).padding(.horizontal, 6)
            }
            .disabled(atThisWeek)
            .opacity(atThisWeek ? 0.35 : 1)
            stepperSegment(label: t("Next week")) { shift(1) } content: {
                Image(systemName: "chevron.right").font(.body.weight(.semibold)).frame(width: 40)
            }
        }
        .foregroundStyle(Theme.accent)
        .frame(height: 44)
        .background(Theme.accentSoft, in: Capsule())
    }

    private func stepperSegment<Content: View>(
        label: String, action: @escaping () -> Void, @ViewBuilder content: () -> Content
    ) -> some View {
        Button(action: action) {
            content()
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
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
                .foregroundStyle(today ? .white : Theme.ink)
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
                    .font(.system(size: 9).monospacedDigit())
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
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        return ZStack(alignment: .top) {
            shape.fill(Theme.surface)
            if let axis { RowRules(axis: axis, rowHeight: rowHeight).clipShape(shape) }
            if agenda?.noClasses == true {
                Hatch().clipShape(shape)
            }
            if today {
                shape.stroke(Theme.accent.opacity(0.5), lineWidth: 1.5)
            }
            if let axis {
                ForEach(blocks, id: \.key) { block in
                    let top = axis.offset(block.startTime) * rowHeight
                    let bottom = axis.offset(block.endTime) * rowHeight
                    blockView(block, height: max(bottom - top - 4, 24), holiday: agenda?.noClasses == true)
                        // Placed by layout, not by offset, so where a block is
                        // drawn is where it answers a tap.
                        .padding(.top, top + 2)
                        .frame(maxHeight: .infinity, alignment: .top)
                }
                if today { NowLine(axis: axis, rowHeight: rowHeight) }
            }
        }
    }

    private func blockView(_ block: Occurrence, height: CGFloat, holiday: Bool) -> some View {
        let planned = block.courseId.map { !(week?.enrolledCourseIds.contains($0) ?? true) } ?? false
        let cancelled = block.cancelled == true
        let exam = block.kind == .exam && !cancelled
        let faded = holiday || cancelled
        let current = isCurrent(block)
        let tint = Color(hex: block.color)
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        let content = VStack(alignment: .leading, spacing: BlockText.spacing) {
            Text(block.title)
                .strikethrough(cancelled)
                .font(BlockText.titleFont)
                .lineLimit(BlockText.titleLines(height: height))
            Spacer(minLength: 0)
            bottomLine(block, planned: planned, exam: exam, cancelled: cancelled)
                .font(BlockText.roomFont)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .foregroundStyle(faded ? Theme.muted : Theme.ink)
        .padding(BlockText.padding)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .frame(height: height)
        .background {
            shape.fill(planned ? Theme.surface : tint.opacity(faded ? 0.08 : Theme.tintFill))
        }
        .overlay {
            // The class in progress outranks the exam border.
            if current {
                shape.stroke(tint, lineWidth: 2)
            } else if exam {
                shape.stroke(Theme.warn, lineWidth: 1.5)
            } else if planned {
                shape.stroke(Theme.line, lineWidth: 1)
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

    /// 計画, 休講, or the first room (an exam says 試験 before it); every room
    /// stays on the course page.
    @ViewBuilder
    private func bottomLine(_ block: Occurrence, planned: Bool, exam: Bool, cancelled: Bool) -> some View {
        let room = BlockText.firstRoom(block.room)
        if planned {
            Text(t("Planned"))
        } else if cancelled {
            Text(t("No class"))
        } else if exam {
            Text("\(Text(t("Exam")).foregroundStyle(Theme.warn)) \(room)")
        } else {
            Text(room)
        }
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

/// Diagonal hatch for a day without classes.
private struct Hatch: View {
    var body: some View {
        Canvas { context, size in
            var path = Path()
            var x: CGFloat = -size.height
            while x < size.width {
                path.move(to: CGPoint(x: x, y: size.height))
                path.addLine(to: CGPoint(x: x + size.height, y: 0))
                x += 10
            }
            context.stroke(path, with: .color(Theme.line), lineWidth: 4)
        }
    }
}
