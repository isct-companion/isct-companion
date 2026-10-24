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
        guard let term = week.term else { return t("Break between quarters") }
        let name = [Format.quarter(term), week.week.map { t("Week {{week}}", ["week": $0]) }].compactMap { $0 }.joined(separator: " ")
        return week.exams ? "\(name) · \(t("Exams"))" : name
    }

    var body: some View {
        let year = shown ?? year
        NavigationStack {
            ScrollViewReader { proxy in
                List {
                    Section {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 8) {
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
                        .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                    }
                    ForEach(months(year), id: \.title) { month in
                        Section(month.title) {
                            ForEach(month.weeks, id: \.monday) { week in
                                if let year { row(week, in: year).id(week.monday) }
                            }
                        }
                    }
                }
                .onAppear { proxy.scrollTo(AppModel.civil(monday), anchor: .center) }
            }
            .navigationTitle(year.map { t("AY {{year}}", ["year": String($0.year)]) } ?? t("Choose a week"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItemGroup(placement: .topBarLeading) {
                    Button(t("Previous year"), systemImage: "chevron.left") { Task { await step(-1) } }
                    Button(t("Next year"), systemImage: "chevron.right") { Task { await step(1) } }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(t("Back to today")) { pick(TimetableScreen.monday(of: Date())) }
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
            HStack(spacing: 12) {
                Text(Self.label(week, in: year) ?? "")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(tone)
                    .frame(width: 110, alignment: .leading)
                Text(days(week.monday))
                    .font(.body.monospacedDigit())
                    .foregroundStyle(selected ? .white : Theme.ink)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
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

    /// `10/19 – 10/23`
    private func days(_ monday: String) -> String {
        guard let start = Format.civilDate(monday),
            let end = Calendar.tokyo.date(byAdding: .day, value: 4, to: start)
        else { return monday }
        let format = { (date: Date) in
            let parts = Calendar.tokyo.dateComponents([.month, .day], from: date)
            return "\(parts.month ?? 0)/\(parts.day ?? 0)"
        }
        return "\(format(start)) – \(format(end))"
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
