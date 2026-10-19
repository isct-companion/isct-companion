import IsctCore
import SwiftUI

/// 時間割: a dated week, read-only. Blocks come from Core's `ScheduleView`
/// for Monday–Friday, whatever quarter the week is in; the title opens a
/// picker that names each week's quarter (`TimetableYear`). Adding and
/// planning courses stay on desktop.
struct TimetableScreen: View {
    @Environment(AppModel.self) private var model
    @State private var monday = TimetableScreen.monday(of: Date())
    @State private var week: ScheduleView?
    @State private var year: TimetableYear?
    @State private var showsPicker = false
    @State private var dragOffset: CGFloat = 0

    /// Period rows share the height the screen leaves, so the week fits
    /// without scrolling (with a floor for very short windows).
    @State private var rowHeight: CGFloat = 52
    private static let minimumRow: CGFloat = 34
    /// Weekday letters and date circles above the columns, plus their gap.
    private static let dayHeaderHeight: CGFloat = 54
    private static let periodCount = 10

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            grid
                .frame(maxHeight: .infinity, alignment: .top)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                    let rows = (height - Self.dayHeaderHeight) / CGFloat(Self.periodCount)
                    rowHeight = max(Self.minimumRow, rows.rounded(.down))
                }
                .offset(x: dragOffset)
                .gesture(swipe)
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .task(id: "\(monday.timeIntervalSince1970)-\(model.contentRevision)") {
            // A newer revision cancels this read: its empty answer must not
            // replace what is shown.
            let loaded = await model.week(starting: monday)
            guard !Task.isCancelled else { return }
            week = loaded
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
                VStack(alignment: .leading, spacing: 2) {
                    Text(subtitle).font(.subheadline).foregroundStyle(Theme.muted)
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(range).font(.largeTitle.bold().monospacedDigit()).foregroundStyle(Theme.ink)
                            .countingFigure(monday, down: false)
                        Image(systemName: "chevron.down.circle.fill")
                            .font(.title3)
                            .foregroundStyle(Theme.accent)
                            .symbolEffect(.bounce, value: showsPicker)
                    }
                }
            }
            .buttonStyle(.plain)
            .accessibilityHint("週を選ぶ")
            Spacer()
            HStack(spacing: 8) {
                weekButton("chevron.left", label: "前の週", by: -1)
                weekButton("chevron.right", label: "次の週", by: 1)
            }
        }
        .padding(.top, 12)
        .padding(.horizontal, 4)
    }

    private var range: String {
        let friday = Calendar.tokyo.date(byAdding: .day, value: 4, to: monday) ?? monday
        return "\(short(monday)) – \(short(friday))"
    }

    /// "2026 · 3Q 第 2 週", from Core's week list; between quarters, 休み.
    private var subtitle: String {
        guard let year else { return "時間割" }
        let label = year.weeks.first { $0.monday == AppModel.civil(monday) }.flatMap { WeekPicker.label($0, in: year) }
        return [String(year.year) + " 年度", label].compactMap { $0 }.joined(separator: " · ")
    }

    private func short(_ date: Date) -> String {
        let parts = Calendar.tokyo.dateComponents([.month, .day], from: date)
        return "\(parts.month ?? 0)/\(parts.day ?? 0)"
    }

    private func weekButton(_ symbol: String, label: String, by weeks: Int) -> some View {
        Button {
            shift(weeks)
        } label: {
            Image(systemName: symbol)
                .font(.body.weight(.semibold))
                .foregroundStyle(Theme.accent)
                .frame(width: 40, height: 40)
                .background(Theme.accentSoft, in: Circle())
        }
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

    private var grid: some View {
        let height = rowHeight * CGFloat(Self.periodCount)
        return VStack(spacing: 6) {
            HStack(spacing: 4) {
                Color.clear.frame(width: 30, height: 1)
                ForEach(days, id: \.self) { day in dayHeader(day).frame(maxWidth: .infinity) }
            }
            HStack(alignment: .top, spacing: 4) {
                periodColumn.frame(width: 30, height: height)
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
            Text(["日", "月", "火", "水", "木", "金", "土"][(parts.weekday ?? 1) - 1])
                .font(.caption)
                .foregroundStyle(Theme.muted)
            Text("\(parts.day ?? 0)")
                .font(.headline.monospacedDigit())
                .foregroundStyle(today ? .white : Theme.ink)
                .frame(width: 30, height: 30)
                .background(today ? Theme.accent : .clear, in: Circle())
        }
    }

    private var periodColumn: some View {
        VStack(spacing: 0) {
            ForEach(1...Self.periodCount, id: \.self) { period in
                VStack(spacing: 0) {
                    Text("\(period)").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
                    if let start = startTime(of: period) {
                        Text(start).font(.system(size: 9).monospacedDigit()).foregroundStyle(Theme.muted)
                    }
                }
                .frame(height: rowHeight, alignment: .top)
            }
        }
    }

    private func agenda(_ day: Date) -> DatedAgenda? {
        let civil = AppModel.civil(day)
        return week?.agendas.first { $0.date == civil }
    }

    private func dayColumn(_ day: Date) -> some View {
        let agenda = agenda(day)
        let today = Calendar.tokyo.isDateInToday(day)
        let blocks = (agenda?.occurrences ?? []).filter { [.course, .exam].contains($0.kind) }
        return ZStack(alignment: .top) {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Theme.surface)
            PeriodLines(periods: model.periods, count: Self.periodCount, rowHeight: rowHeight)
            if agenda?.noClasses == true {
                Hatch().clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
            if today {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(Theme.accent.opacity(0.5), lineWidth: 1.5)
            }
            ForEach(blocks, id: \.key) { block in
                blockView(block, holiday: agenda?.noClasses == true)
            }
        }
    }

    private func blockView(_ block: Occurrence, holiday: Bool) -> some View {
        let top = slot(block.startTime) * rowHeight
        let bottom = slot(block.endTime) * rowHeight
        let planned = block.courseId.map { !(week?.enrolledCourseIds.contains($0) ?? true) } ?? false
        let faded = holiday || block.cancelled == true
        let current = isCurrent(block)
        let tint = Color(hex: block.color)
        let content = VStack(alignment: .leading, spacing: 2) {
            Text(block.title)
                .font(.caption.weight(.semibold))
                .lineLimit(3)
            Spacer(minLength: 0)
            Text(planned ? "計画" : block.room)
                .font(.caption2.weight(.semibold))
                .lineLimit(1)
        }
        .foregroundStyle(faded ? Theme.muted : Theme.ink)
        .padding(5)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(planned ? Theme.surface : tint.opacity(faded ? 0.08 : Theme.tintFill))
        }
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(current ? tint : planned ? Theme.line : .clear, lineWidth: current ? 2 : 1)
        }
        .frame(height: max(bottom - top - 4, 24))
        .padding(.horizontal, 2)
        .offset(y: top + 2)
        return Group {
            if let id = block.courseId {
                NavigationLink(value: CourseRoute(id: id, zoom: block.key)) { content }
                    .buttonStyle(.plain)
                    .courseZoomSource(block.key)
            } else {
                content
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private func isCurrent(_ block: Occurrence) -> Bool {
        guard let today = model.today, today.phase == .inClass, let hero = today.hero else { return false }
        return hero.occurrence.key == block.key
    }

    // MARK: Period geometry

    /// Fractional row (0-based) of a clock time, interpolated over the
    /// teaching-period table so blocks line up with their periods.
    private func slot(_ time: String) -> CGFloat {
        let minutes = Self.minutes(time)
        var row: CGFloat = 0
        for period in model.periods {
            let start = Self.minutes(period.startTime)
            let end = Self.minutes(period.endTime)
            let first = CGFloat(period.periods.first ?? 1) - 1
            let span = CGFloat((period.periods.last ?? 1) - (period.periods.first ?? 1) + 1)
            if minutes < start { return row }
            if minutes <= end {
                return first + span * CGFloat(minutes - start) / CGFloat(max(end - start, 1))
            }
            row = first + span
        }
        return min(row, CGFloat(Self.periodCount))
    }

    private func startTime(of period: Int) -> String? {
        model.periods.first { Int($0.periods.first ?? 0) == period }?.startTime
            .replacingOccurrences(of: "^0", with: "", options: .regularExpression)
    }

    private static func minutes(_ time: String) -> Int {
        let parts = time.split(separator: ":").compactMap { Int($0) }
        return parts.count == 2 ? parts[0] * 60 + parts[1] : 0
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

/// One rule after each teaching slot (1–2, 3–4, …), never inside a slot, so
/// a two-period class is not cut in half.
private struct PeriodLines: View {
    let periods: [TeachingPeriod]
    let count: Int
    let rowHeight: CGFloat

    var body: some View {
        let ends = periods.compactMap { $0.periods.last.map(Int.init) }.filter { (1..<count).contains($0) }
        Canvas { context, size in
            var path = Path()
            for end in ends {
                let y = CGFloat(end) * rowHeight
                path.move(to: CGPoint(x: 0, y: y))
                path.addLine(to: CGPoint(x: size.width, y: y))
            }
            context.stroke(path, with: .color(Theme.line), lineWidth: 1)
        }
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
