import IsctCore
import SwiftUI

/// The timetable's time picker: quarter chips that jump to a quarter's first
/// teaching week (how a student shows someone that quarter's courses), and the
/// academic year's weeks, each labelled with its quarter. Core decides every
/// label (`TimetableYear`); this only lays them out.
struct WeekPicker: View {
    let year: TimetableYear?
    let monday: Date
    let pick: (Date) -> Void
    @Environment(AppModel.self) private var model
    @State private var shown: TimetableYear?

    /// Nothing while the academic calendar is unknown: a week is only 休み
    /// when Core knows the quarters around it.
    static func label(_ week: TimetableWeek, in year: TimetableYear) -> String? {
        guard !year.quarters.isEmpty else { return nil }
        guard let term = week.term else { return t("timetable.breakBetweenQuarters") }
        let name = [Format.quarter(term), week.week.map { t("timetable.weekNumber", ["week": $0]) }].compactMap { $0 }.joined(separator: " ")
        return week.exams ? "\(name) · \(t("common.exams"))" : name
    }

    var body: some View {
        let year = shown ?? year
        NavigationStack {
            ScrollViewReader { proxy in
                List {
                    Group {
                        Section {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: Theme.space4) {
                                    ForEach(year?.quarters ?? [], id: \.term) { quarter in
                                        Button(Format.quarter(quarter.term)) {
                                            if let date = Format.civilDate(quarter.firstWeek) { pick(date) }
                                        }
                                        .buttonStyle(.bordered)
                                        .tint(isShown(quarter) ? Theme.accent : Theme.muted)
                                    }
                                }
                            }
                            .listRowBackground(Color.clear)
                            .listRowInsets(EdgeInsets(top: Theme.space2, leading: Theme.space8, bottom: Theme.space2, trailing: Theme.space8))
                        }
                        ForEach(months(year), id: \.title) { month in
                            Section(month.title) {
                                ForEach(month.weeks, id: \.monday) { week in
                                    if let year { row(week, in: year).id(week.monday) }
                                }
                            }
                        }
                    }
                    .themedRows()
                }
                .themedList()
                .onAppear { proxy.scrollTo(AppModel.civil(monday), anchor: .center) }
            }
            .navigationTitle(year.map { t("timetable.academicYear", ["year": String($0.year)]) } ?? t("timetable.chooseAWeek"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItemGroup(placement: .topBarLeading) {
                    Button(t("timetable.previousYear"), systemImage: "chevron.left") { Task { await step(-1) } }
                    Button(t("timetable.nextYear"), systemImage: "chevron.right") { Task { await step(1) } }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(t("timetable.backToToday")) { pick(TimetableScreen.monday(of: AppClock.now)) }
                }
            }
        }
    }

    private func row(_ week: TimetableWeek, in year: TimetableYear) -> some View {
        let selected = week.monday == AppModel.civil(monday)
        let tone: Color = week.exams ? Theme.warn : week.term == nil ? Theme.muted : Theme.accent
        return Button {
            if let date = Format.civilDate(week.monday) { pick(date) }
        } label: {
            HStack(spacing: Theme.space6) {
                Text(Self.label(week, in: year) ?? "")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(tone)
                    .frame(width: 110, alignment: .leading)
                Text(Format.civilDate(week.monday).map { weekDateRange(from: $0) } ?? week.monday)
                    .font(.body.monospacedDigit())
                    .foregroundStyle(selected ? Theme.accentInk : Theme.ink)
                    .padding(.horizontal, Theme.space5)
                    .padding(.vertical, Theme.space2)
                    .background(selected ? Theme.accent : .clear, in: Capsule())
                Spacer()
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(week.term == nil ? Theme.surface2 : Theme.surface)
    }

    private func isShown(_ quarter: TimetableQuarter) -> Bool {
        year?.weeks.first { $0.monday == AppModel.civil(monday) }?.term == quarter.term
    }

    private struct Month {
        let title: String
        var weeks: [TimetableWeek]
    }

    /// Weeks grouped under the month their Monday falls in.
    private func months(_ year: TimetableYear?) -> [Month] {
        var months: [Month] = []
        for week in year?.weeks ?? [] {
            let title = Format.civilDate(week.monday).map { Format.written($0, "yMMMM") } ?? week.monday
            if months.last?.title == title {
                months[months.count - 1].weeks.append(week)
            } else {
                months.append(Month(title: title, weeks: [week]))
            }
        }
        return months
    }

    private func step(_ by: Int32) async {
        guard let current = (shown ?? year)?.year else { return }
        if let next = await model.timetableYear(current + by) { shown = next }
    }
}
