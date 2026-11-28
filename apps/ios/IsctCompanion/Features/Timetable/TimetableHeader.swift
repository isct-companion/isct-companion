import IsctCore
import SwiftUI
import TipKit

/// The week chip, the quarter export and 今日 above the week's date range,
/// which rolls on one line across the full width. Weeks change by swiping the
/// grid; the chip and the range open the week picker.
struct TimetableHeader: View {
    let monday: Date
    let year: TimetableYear?
    /// The shown week's quarter; none between quarters.
    let exportTerm: String?
    let atThisWeek: Bool
    let openPicker: () -> Void
    let toToday: () -> Void
    @Environment(\.exportCalendar) private var exportCalendar

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.space3) {
            // One row while the shown year's widest chip fits beside the
            // actions, stacked otherwise. The widest label, laid out hidden
            // under the chip, makes the choice follow the window and the text
            // size rather than the week on screen, so a longer exam-week label
            // does not reshape the header, or the grid's rows, mid-swipe.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: Theme.space4) {
                    ZStack(alignment: .leading) {
                        ForEach(chipTitles, id: \.self) { chipLabel($0).hidden() }
                        weekPicker
                    }
                    Spacer(minLength: 0)
                    actions
                }
                VStack(alignment: .leading, spacing: Theme.space2) {
                    weekPicker
                    actions
                }
            }
            Button(action: openPicker) {
                Text(weekDateRange(from: monday))
                    .font(.largeTitle.bold().monospacedDigit())
                    .foregroundStyle(Theme.ink)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .countingFigure(monday, down: false)
            }
            .buttonStyle(.plain)
            .accessibilityHint(t("timetable.chooseAWeek"))
            // Over the grid so the tip does not take space from the timetable.
            .popoverTip(TimetableMarkTip(), arrowEdge: .top)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, Theme.space6)
    }

    private var weekPicker: some View {
        Button(action: openPicker) {
            chipLabel(chipTitle(year?.weeks.first { $0.monday == AppModel.civil(monday) }))
        }
        .buttonStyle(.plain)
        .accessibilityHint(t("timetable.chooseAWeek"))
    }

    /// One line, so the chip keeps its height whichever week it names.
    private func chipLabel(_ title: String) -> some View {
        Label(title, systemImage: "calendar")
            .font(.footnote.weight(.semibold))
            .lineLimit(1)
            .minimumScaleFactor(0.8)
            .foregroundStyle(Theme.accent)
            .padding(.horizontal, Theme.space5)
            .padding(.vertical, Theme.space3)
            .background(Theme.accentSoft, in: Capsule())
    }

    /// "2026年度 · 3Q 第 2 週", from Core's week list; between quarters, 休み.
    private func chipTitle(_ week: TimetableWeek?) -> String {
        guard let year else { return t("common.timetable") }
        let label = week.flatMap { WeekPicker.label($0, in: year) }
        return [t("timetable.academicYear", ["year": String(year.year)]), label].compactMap { $0 }.joined(separator: " · ")
    }

    /// Every label the chip can show in the shown year, once each.
    private var chipTitles: [String] {
        var seen = Set<String>()
        return ((year?.weeks ?? []).map { chipTitle($0) } + [chipTitle(nil)]).filter { seen.insert($0).inserted }
    }

    private var actions: some View {
        HStack(spacing: Theme.space4) {
            // Shown but disabled for a week outside a quarter, so its place
            // stays and 今日 never moves.
            Menu {
                if let exportTerm {
                    Section(CalendarExportPick.title(exportTerm)) {
                        Button(t("common.coursesOnly"), systemImage: "calendar") { exportCalendar(.courses) }
                        Button(t("common.coursesWeeklyEvents"), systemImage: "calendar.badge.plus") { exportCalendar(.coursesAndEvents) }
                    }
                }
            } label: {
                Image(systemName: "square.and.arrow.up")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(exportTerm == nil ? Theme.muted : Theme.accent)
                    .frame(width: 44, height: 44)
                    .contentShape(.rect)
            }
            .disabled(exportTerm == nil)
            .accessibilityLabel(t("common.exportICS"))
            Button(action: toToday) {
                Text(t("common.today"))
                    .font(.subheadline.weight(.semibold))
                    .padding(.horizontal, Theme.space8)
                    .padding(.vertical, Theme.space5)
            }
            .buttonStyle(CapsuleButtonStyle(level: .secondary))
            .disabled(atThisWeek)
            .opacity(atThisWeek ? 0.35 : 1)
            .accessibilityLabel(t("timetable.backToThisWeek"))
        }
    }
}

func weekDateRange(from monday: Date) -> String {
    let friday = Calendar.tokyo.date(byAdding: .day, value: 4, to: monday) ?? monday
    func short(_ date: Date) -> String {
        let parts = Calendar.tokyo.dateComponents([.month, .day], from: date)
        return "\(parts.month ?? 0)/\(parts.day ?? 0)"
    }
    return "\(short(monday)) – \(short(friday))"
}
