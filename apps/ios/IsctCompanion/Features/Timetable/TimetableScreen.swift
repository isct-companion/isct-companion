import IsctCore
import SwiftUI

/// 時間割: the week grid, read-only. Blocks come from Core's `ScheduleView`
/// for Monday–Friday; adding and planning courses stay on desktop.
struct TimetableScreen: View {
    @Environment(AppModel.self) private var model
    @State private var monday = TimetableScreen.monday(of: Date())
    @State private var week: ScheduleView?
    @State private var dragOffset: CGFloat = 0

    private static let rowHeight: CGFloat = 64
    private static let periodCount = 10

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                header
                grid
                    .offset(x: dragOffset)
                    .gesture(swipe)
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 24)
        }
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .task(id: "\(monday.timeIntervalSince1970)-\(model.contentRevision)") {
            week = await model.week(starting: monday)
        }
    }

    // MARK: Header

    private var header: some View {
        HStack(alignment: .bottom) {
            VStack(alignment: .leading, spacing: 2) {
                Text(subtitle).font(.subheadline).foregroundStyle(Theme.muted)
                Text("時間割").font(.largeTitle.bold()).foregroundStyle(Theme.ink)
            }
            Spacer()
            HStack(spacing: 8) {
                weekButton("chevron.left", label: "前の週", by: -1)
                weekButton("chevron.right", label: "次の週", by: 1)
            }
        }
        .padding(.top, 12)
        .padding(.horizontal, 4)
    }

    private var subtitle: String {
        let friday = Calendar.tokyo.date(byAdding: .day, value: 4, to: monday) ?? monday
        let range = "\(short(monday)) – \(short(friday))"
        guard let term = model.today?.term else { return range }
        if isThisWeek, let week = model.today?.teachingWeek {
            return "\(Format.quarter(term)) 第\(week)週 · \(range)"
        }
        return "\(Format.quarter(term)) · \(range)"
    }

    private func short(_ date: Date) -> String {
        let parts = Calendar.tokyo.dateComponents([.month, .day], from: date)
        return "\(parts.month ?? 0)/\(parts.day ?? 0)"
    }

    private var isThisWeek: Bool {
        Calendar.tokyo.isDate(monday, inSameDayAs: Self.monday(of: Date()))
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
        let height = Self.rowHeight * CGFloat(Self.periodCount)
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
                .frame(height: Self.rowHeight, alignment: .top)
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
        let top = slot(block.startTime) * Self.rowHeight
        let bottom = slot(block.endTime) * Self.rowHeight
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
                NavigationLink(value: CourseRoute(id: id)) { content }.buttonStyle(.plain)
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
