import IsctCore
import SwiftUI

/// What the 時間割 tab's long press offers: the shown quarter as an .ics.
enum CalendarExportPick: Hashable {
    case courses, coursesAndEvents

    var scope: CalendarExportScope { self == .courses ? .courses : .coursesAndEvents }
}

/// The export panel's rows; the workstation's rail row shows the same in a menu.
struct CalendarExportRows: View {
    let term: String
    var highlighted: CalendarExportPick?
    var report: ((CalendarExportPick, CGRect) -> Void)?
    let pick: (CalendarExportPick) -> Void

    var body: some View {
        Text(t("common.exportICS") + " · " + Format.quarter(term))
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Theme.space7)
            .padding(.top, Theme.space3)
            .padding(.bottom, Theme.space1)
        TabPressRow(choice: CalendarExportPick.courses, title: t("common.coursesOnly"), icon: "calendar",
                    highlighted: highlighted, report: report, pick: pick)
        TabPressRow(choice: CalendarExportPick.coursesAndEvents, title: t("common.coursesWeeklyEvents"),
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
        let suffix = pick == .coursesAndEvents ? "-with-events" : ""
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("IsctCompanion-\(term)\(suffix).ics")
        try Data(text.utf8).write(to: url, options: .atomic)
        return Self(url: url)
    }
}
