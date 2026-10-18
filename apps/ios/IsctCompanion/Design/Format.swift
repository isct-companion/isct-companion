import Foundation
import IsctCore
import SwiftUI

/// Text formatting of Core's timestamps. Urgency (`tone`) comes from Core; the
/// shell only turns times into words.
enum Format {
    private static let weekdays = ["日", "月", "火", "水", "木", "金", "土"]

    static func date(_ raw: String?) -> Date? {
        raw.flatMap(AppModel.date)
    }

    /// `10月15日（木）`
    static func dayTitle(_ civil: String) -> String {
        guard let date = civilDate(civil) else { return civil }
        let parts = Calendar.tokyo.dateComponents([.month, .day, .weekday], from: date)
        return "\(parts.month ?? 0)月\(parts.day ?? 0)日（\(weekdays[(parts.weekday ?? 1) - 1])）"
    }

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

    /// `今日 23:59`, `明日 23:59`, `10/18（日）23:59`
    static func due(_ raw: String?, now: Date = Date()) -> String {
        guard let due = date(raw) else { return "期限なし" }
        let calendar = Calendar.tokyo
        let time = self.time(due)
        if calendar.isDate(due, inSameDayAs: now) { return "今日 \(time)" }
        if let tomorrow = calendar.date(byAdding: .day, value: 1, to: now), calendar.isDate(due, inSameDayAs: tomorrow) {
            return "明日 \(time)"
        }
        let parts = calendar.dateComponents([.month, .day, .weekday], from: due)
        return "\(parts.month ?? 0)/\(parts.day ?? 0)（\(weekdays[(parts.weekday ?? 1) - 1])）\(time)"
    }

    /// Minutes until `date`, rounded up so "あと 1 分" lasts until the minute.
    static func minutes(until date: Date, now: Date) -> Int {
        Int((date.timeIntervalSince(now) / 60).rounded(.up))
    }

    /// Countdown words for a deadline chip.
    static func countdown(_ raw: String?, now: Date, overdueStyle: OverdueStyle = .label) -> String {
        guard let due = date(raw) else { return "期限なし" }
        let minutes = minutes(until: due, now: now)
        if minutes < 0 {
            switch overdueStyle {
            case .label: return "期限切れ"
            case .elapsed:
                let days = max(1, -minutes / (60 * 24))
                return "\(days) 日超過"
            }
        }
        if minutes < 60 { return "あと \(max(minutes, 1)) 分" }
        if minutes < 60 * 24 { return "あと \(minutes / 60) 時間" }
        if Calendar.tokyo.isDate(due, inSameDayAs: Calendar.tokyo.date(byAdding: .day, value: 1, to: now) ?? now) {
            return "明日"
        }
        return "あと \(Int((Double(minutes) / (60 * 24)).rounded())) 日"
    }

    enum OverdueStyle { case label, elapsed }

    /// `3–4限`
    static func periods<Periods: Collection>(_ value: Periods?) -> String? where Periods.Element == UInt8 {
        guard let value, value.count == 2, let first = value.first, let last = value.dropFirst().first else { return nil }
        return first == last ? "\(first)限" : "\(first)–\(last)限"
    }
}

/// Deadline chip: one per row, coloured by Core's tone.
struct DeadlineChip: View {
    let item: DeadlineItem
    var now: Date
    var overdueStyle: Format.OverdueStyle = .label

    var body: some View {
        let (ink, fill) = Self.colors(item.submitted ? "done" : item.tone)
        Text(item.submitted ? (item.grade ?? "提出済") : Format.countdown(item.dueAt, now: now, overdueStyle: overdueStyle))
            .font(.caption.weight(.semibold).monospacedDigit())
            .foregroundStyle(ink)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(fill, in: RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
            .fixedSize()
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

/// Small text chip (`今日`, `資料`, `採点済`).
struct TagChip: View {
    let text: String
    var ink: Color = Theme.accent
    var fill: Color = Theme.accentSoft
    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .foregroundStyle(ink)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(fill, in: RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
            .fixedSize()
    }
}

/// Assignment row: course square, title, course · due, one chip.
struct DeadlineRow: View {
    let item: DeadlineItem
    var now: Date
    var overdueStyle: Format.OverdueStyle = .label
    var showsCourse = true

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
