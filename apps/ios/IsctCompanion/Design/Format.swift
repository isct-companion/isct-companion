import Foundation
import IsctCore
import SwiftUI

/// Text formatting of Core's timestamps. Urgency (`tone`) comes from Core; the
/// shell only turns times into words, in the UI language.
enum Format {

    static func date(_ raw: String?) -> Date? {
        raw.flatMap(AppModel.date)
    }

    /// `10月15日(木)`, `Thu 15 Oct`
    static func dayTitle(_ civil: String) -> String {
        guard let date = civilDate(civil) else { return civil }
        return written(date, "MMMdEEE")
    }

    /// Short weekday names from Sunday, as Core numbers days (`日`, `Sun`, `周日`).
    static var weekdays: [String] {
        let formatter = DateFormatter()
        formatter.locale = Localizer.shared.locale
        return formatter.shortStandaloneWeekdaySymbols
    }

    /// A Tokyo date written from a template in the UI language.
    static func written(_ date: Date, _ template: String) -> String {
        let locale = Localizer.shared.locale
        let key = "\(locale.identifier) \(template)"
        formatterLock.lock()
        defer { formatterLock.unlock() }
        if let formatter = formatters[key] { return formatter.string(from: date) }
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.timeZone = .tokyo
        formatter.setLocalizedDateFormatFromTemplate(template)
        formatters[key] = formatter
        return formatter.string(from: date)
    }

    private nonisolated(unsafe) static var formatters: [String: DateFormatter] = [:]
    private static let formatterLock = NSLock()

    static func civilDate(_ civil: String) -> Date? {
        let parts = civil.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3 else { return nil }
        return Calendar.tokyo.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
    }

    /// `3Q` from a term id such as `2026-3Q`.
    static func quarter(_ term: String) -> String {
        term.split(separator: "-").last.map(String.init) ?? term
    }

    /// `HH:MM` in Tokyo time.
    static func time(_ date: Date) -> String {
        let parts = Calendar.tokyo.dateComponents([.hour, .minute], from: date)
        return String(format: "%d:%02d", parts.hour ?? 0, parts.minute ?? 0)
    }

    /// `今日 23:59`, `Tomorrow 23:59`, `10/18(日) 23:59`
    static func due(_ raw: String?, now: Date = Date()) -> String {
        guard let due = date(raw) else { return t("common.noDeadline") }
        let calendar = Calendar.tokyo
        let time = self.time(due)
        if calendar.isDate(due, inSameDayAs: now) { return t("format.todayTime", ["time": time]) }
        if let tomorrow = calendar.date(byAdding: .day, value: 1, to: now), calendar.isDate(due, inSameDayAs: tomorrow) {
            return t("format.tomorrowTime", ["time": time])
        }
        return "\(written(due, "MdEEE")) \(time)"
    }

    /// Minutes until `date`, rounded up so "1 min left" lasts until the minute.
    static func minutes(until date: Date, now: Date) -> Int {
        Int((date.timeIntervalSince(now) / 60).rounded(.up))
    }

    /// Countdown words for a deadline chip.
    static func countdown(_ raw: String?, now: Date, overdueStyle: OverdueStyle = .label) -> String {
        guard let due = date(raw) else { return t("common.noDeadline") }
        let minutes = minutes(until: due, now: now)
        if minutes < 0 {
            switch overdueStyle {
            case .label: return t("common.overdue")
            case .elapsed:
                return t("format.countDOverdue", ["count": max(1, -minutes / (60 * 24))])
            }
        }
        if minutes < 60 { return t("today.countMinLeft", ["count": max(minutes, 1)]) }
        if minutes < 60 * 24 { return t("format.countHLeft", ["count": minutes / 60]) }
        if Calendar.tokyo.isDate(due, inSameDayAs: Calendar.tokyo.date(byAdding: .day, value: 1, to: now) ?? now) {
            return t("common.tomorrow")
        }
        return t("format.countDLeft", ["count": Int((Double(minutes) / (60 * 24)).rounded())])
    }

    enum OverdueStyle { case label, elapsed }

    /// `3–4限`, `Periods 3–4`
    static func periods<Periods: Collection>(_ value: Periods?) -> String? where Periods.Element == UInt8 {
        guard let value, value.count == 2, let first = value.first, let last = value.dropFirst().first else { return nil }
        return t("format.periodsRange", ["range": first == last ? "\(first)" : "\(first)–\(last)"])
    }
}

/// The one tag (slotledger `approach/chip-variants`): soft by default, an
/// outline (a quiz), or solid (an exam only), in a role's colours.
struct Tag: View {
    enum Style { case soft, outline, solid }
    let text: String
    var style: Style = .soft
    /// The role's strong colour and its soft fill.
    var ink: Color = Theme.accent
    var fill: Color = Theme.accentSoft

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous)
        Text(text)
            .font(.block.monospacedDigit())
            .lineLimit(1)
            .foregroundStyle(style == .solid ? Theme.onSolid : ink)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background {
                switch style {
                case .soft: shape.fill(fill)
                case .solid: shape.fill(ink)
                case .outline: shape.strokeBorder(ink, lineWidth: 1)
                }
            }
            .fixedSize()
    }
}

/// Deadline chip: one per row, coloured by Core's tone.
struct DeadlineChip: View {
    let item: DeadlineItem
    var now: Date
    var overdueStyle: Format.OverdueStyle = .label

    var body: some View {
        let (ink, fill) = Self.colors(item.submitted ? "done" : item.tone)
        Tag(text: item.submitted ? (item.grade ?? t("common.submitted")) : Format.countdown(item.dueAt, now: now, overdueStyle: overdueStyle),
            ink: ink, fill: fill)
    }

    static func colors(_ tone: String) -> (Color, Color) {
        switch tone {
        case "past", "urgent": return (Theme.danger, Theme.dangerSoft)
        case "soon": return (Theme.warn, Theme.warnSoft)
        case "done": return (Theme.ok, Theme.okSoft)
        default: return (Theme.muted, Theme.surface2)
        }
    }
}

/// Small text tag (`Today`, `Material`, `Graded`).
struct TagChip: View {
    let text: String
    var ink: Color = Theme.accent
    var fill: Color = Theme.accentSoft
    var body: some View { Tag(text: text, ink: ink, fill: fill) }
}

/// Core's `ExamConflict` on an exam row: the published exam changed after the
/// user corrected it, or left the timetable. Resolving it stays on desktop.
struct ExamConflictLabel: View {
    let conflict: ExamConflict

    var body: some View {
        Label(conflict.removed ? t("exam.noLongerInThePublished") : t("common.timetableChanged"),
            systemImage: "exclamationmark.circle.fill")
            .font(.caption.weight(.semibold))
            .foregroundStyle(Theme.warn)
            .lineLimit(1)
    }
}

/// Assignment row: course square, title, course · due, one chip.
struct DeadlineRow: View {
    let item: DeadlineItem
    var now: Date
    var overdueStyle: Format.OverdueStyle = .label
    var showsCourse = true
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 12) {
            CourseSquare(color: item.color)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title)
                    .font(.body)
                    .foregroundStyle(item.submitted ? Theme.muted : Theme.ink)
                    .lineLimit(2)
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
                    .lineLimit(1)
                // Files put in its upload card wait for the person to hand
                // them in; Core counts them until LMS shows the work submitted.
                if let waiting = model.waiting[item.assignmentId] {
                    Label(t("components.countFilesToHandIn", ["count": waiting]), systemImage: "arrow.up.doc")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Theme.accent)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            DeadlineChip(item: item, now: now, overdueStyle: overdueStyle)
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 16)
        .contentShape(Rectangle())
    }

    private var subtitle: String {
        let due = Format.due(item.dueAt, now: now)
        // The deadline leads: a long course name truncates, the date must not.
        guard showsCourse, let course = item.courseTitle else { return due }
        return "\(due) · \(course)"
    }
}
