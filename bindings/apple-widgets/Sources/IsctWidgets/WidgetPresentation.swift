import Foundation
import SwiftUI
import WidgetKit

public struct ClassWidget: Widget {
    public init() {}
    public var body: some WidgetConfiguration { Configuration(purpose: .courses).body }
}

public struct AssignmentWidget: Widget {
    public init() {}
    public var body: some WidgetConfiguration { Configuration(purpose: .assignments).body }
}

@MainActor
private struct Configuration {
    let purpose: WidgetPurpose
    private let messages = MessageCatalog(language: MessageCatalog.systemLanguage, bundle: .main)

    var body: some WidgetConfiguration {
        let container = (Bundle.main.object(forInfoDictionaryKey: "IsctWidgetGroup") as? String)
            .flatMap { FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: $0) }
        return StaticConfiguration(kind: purpose.kind, provider: Provider(purpose: purpose, container: container)) { entry in
            ContentView(entry: entry)
                .containerBackground(.background, for: .widget)
        }
        .configurationDisplayName(messages.string("widget." + purpose.rawValue))
        .description(messages.string("widget." + purpose.rawValue + ".description"))
        .supportedFamilies(families)
    }

    private var families: [WidgetFamily] {
        #if os(iOS)
        if purpose == .courses { return [.systemSmall, .systemMedium, .systemLarge, .accessoryRectangular] }
        #endif
        return [.systemSmall, .systemMedium, .systemLarge]
    }
}

private enum Content {
    case course(WidgetFile.Course)
    case assignments(WidgetFile.Assignments)
    case unavailable(String, WidgetDestination)
    case placeholder(String)

    var destination: WidgetDestination? {
        switch self {
        case .course(let content): content.destination
        case .assignments(let content): content.destination
        case .unavailable(_, let destination): destination
        case .placeholder: nil
        }
    }
}

private struct DisplayEntry: TimelineEntry {
    let date: Date
    let content: Content
    let messages: MessageCatalog
    let zone: TimeZone
}

private struct Provider: TimelineProvider {
    let purpose: WidgetPurpose
    let container: URL?

    func placeholder(in context: Context) -> DisplayEntry {
        DisplayEntry(date: .now, content: .placeholder(purpose == .courses ? "widget.courses" : "widget.assignments"), messages: .init(language: MessageCatalog.systemLanguage, bundle: .main), zone: .current)
    }

    func getSnapshot(in context: Context, completion: @escaping (DisplayEntry) -> Void) {
        completion(timeline(at: .now).entries.first ?? placeholder(in: context))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<DisplayEntry>) -> Void) {
        completion(timeline(at: .now))
    }

    private func timeline(at now: Date) -> Timeline<DisplayEntry> {
        let file: WidgetFile
        do {
            guard let container else { throw CocoaError(.fileNoSuchFile) }
            file = try WidgetFile.read(from: WidgetLocation.file(in: container))
        } catch {
            let content: Content
            #if os(iOS)
            if (error as? CocoaError)?.code == .fileReadNoPermission {
                content = .placeholder(purpose == .courses ? "widget.courses" : "widget.assignments")
            } else {
                content = .unavailable("widget.openToUpdate", .sync)
            }
            #else
            content = .unavailable("widget.openToUpdate", .sync)
            #endif
            let entry = DisplayEntry(date: now, content: content, messages: .init(language: MessageCatalog.systemLanguage, bundle: .main), zone: .current)
            if case .placeholder = content {
                // After first unlock the protected file can become readable
                // even if the main app has not run to send a reload.
                return Timeline(entries: [entry], policy: .atEnd)
            }
            return Timeline(entries: [entry], policy: .never)
        }
        guard let zone = TimeZone(identifier: file.schoolTimeZone) else {
            return Timeline(entries: [DisplayEntry(date: now,
                content: .unavailable("widget.openToUpdate", .sync),
                messages: .init(language: file.locale, bundle: .main), zone: .current)], policy: .never)
        }
        let messages = MessageCatalog(language: file.locale, bundle: .main)
        func entry(_ date: Date, _ content: Content) -> DisplayEntry {
            DisplayEntry(date: date, content: content, messages: messages, zone: zone)
        }
        let terminal = Content.unavailable(file.terminalContent.messageKey, file.terminalContent.destination)
        guard file.coverageStart <= now, now < file.coverageEnd else {
            return Timeline(entries: [entry(now, terminal)], policy: .never)
        }
        let dated = switch purpose {
        case .courses: file.courseEntries.map { entry($0.effectiveAt, .course($0.content)) }
        case .assignments: file.assignmentEntries.map { entry($0.effectiveAt, .assignments($0.content)) }
        }
        let current = dated.last(where: { $0.date <= now }).map(\.content) ?? terminal
        return Timeline(
            entries: [entry(now, current)] + dated.filter { $0.date > now } + [entry(file.coverageEnd, terminal)],
            policy: .never
        )
    }

}

private struct ContentView: View {
    @Environment(\.widgetFamily) private var family
    @Environment(\.dynamicTypeSize) private var type
    @Environment(\.widgetRenderingMode) private var rendering
    let entry: DisplayEntry

    private var usesLinks: Bool { family == .systemMedium || family == .systemLarge }
    private var isAccessory: Bool {
        #if os(iOS)
        family == .accessoryRectangular
        #else
        false
        #endif
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            switch entry.content {
            case .course(let value): classes(value)
            case .assignments(let value): assignments(value)
            case .unavailable(let message, _): status(message, icon: "arrow.clockwise")
            case .placeholder(let message):
                status(message, icon: "calendar").redacted(reason: .placeholder)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(entry.content.destination?.url)
    }

    @ViewBuilder private func classes(_ value: WidgetFile.Course) -> some View {
        if let primary = value.primary {
            VStack(alignment: .leading, spacing: 3) {
                Text(entry.messages.string("widget.phase." + value.phase.rawValue))
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                link(primary.destination) {
                    course(primary, hero: true, showDate: primary.date != value.date)
                }
                if !isAccessory { freshness(value.freshness) }
                if usesLinks && !type.isAccessibilitySize {
                    let rows = family == .systemLarge ? value.today : value.upcoming
                    if !rows.isEmpty {
                        Spacer(minLength: 3)
                        ViewThatFits(in: .vertical) {
                            classRows(rows.prefix(family == .systemLarge ? 5 : 2), date: value.date)
                            classRows(rows.prefix(1), date: value.date)
                            Color.clear.frame(height: 0)
                        }
                    }
                }
            }
            .privacySensitive()
        } else {
            status(value.freshness.coverageComplete ? "widget.noClasses" : "widget.openToUpdate", icon: "calendar")
            if !isAccessory { freshness(value.freshness) }
        }
    }

    private func classRows(_ rows: ArraySlice<WidgetFile.ClassRow>, date: String) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                link(row.destination) { course(row, hero: false, showDate: row.date != date) }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func course(_ row: WidgetFile.ClassRow, hero: Bool, showDate: Bool) -> some View {
        VStack(alignment: .leading, spacing: hero ? 2 : 0) {
            Text(row.title)
                .font(hero && !isAccessory ? .headline : .caption.weight(.semibold))
                .lineLimit(hero && !isAccessory ? 2 : 1)
                .widgetAccentable()
            if hero && !isAccessory {
                if !row.room.isEmpty {
                    Text(row.room).font(.subheadline.weight(.semibold)).lineLimit(1)
                }
                Text(interval(row, date: showDate))
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            } else {
                Text([row.room, interval(row, date: showDate)].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, isAccessory ? 0 : 2)
        .padding(.horizontal, isAccessory ? 0 : 5)
        .background {
            if !isAccessory && rendering == .fullColor {
                RoundedRectangle(cornerRadius: IsctTokens.radiusBlock)
                    .fill(color(row.color).opacity(0.10))
                    .accessibilityHidden(true)
            }
        }
    }

    @ViewBuilder private func assignments(_ value: WidgetFile.Assignments) -> some View {
        if value.rows.isEmpty {
            status(value.freshness.coverageComplete ? "widget.noAssignments" : "widget.openToUpdate", icon: "checklist")
        } else {
            Text(entry.messages.string("widget.assignments"))
                .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            ViewThatFits(in: .vertical) {
                assignmentRows(value.rows.prefix(type.isAccessibilitySize ? 1 : family == .systemLarge ? 6 : usesLinks ? 3 : 1))
                assignmentRows(value.rows.prefix(1))
            }
            .privacySensitive()
        }
        freshness(value.freshness)
    }

    private func assignmentRows(_ rows: ArraySlice<WidgetFile.AssignmentRow>) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                link(row.destination) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(row.title)
                            .font(index == 0 ? .headline : .caption.weight(.semibold))
                            .lineLimit(index == 0 ? 2 : 1)
                        if let title = row.courseTitle {
                            Text(title).font(.caption).lineLimit(1)
                                .foregroundStyle(rendering == .fullColor ? color(row.color) : .secondary)
                        }
                        if let deadline = row.dueAt {
                            Text(row.overdue
                                 ? entry.messages.string("widget.overdue", ["time": formatted(deadline, date: true)])
                                 : formatted(deadline, date: true))
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder private func link<V: View>(_ destination: WidgetDestination, @ViewBuilder content: () -> V) -> some View {
        if usesLinks, let url = destination.url {
            Link(destination: url, label: content)
        } else {
            content()
        }
    }

    private func status(_ key: String, icon: String) -> some View {
        Label(entry.messages.string(key), systemImage: icon)
            .font(.subheadline.weight(.medium))
            .foregroundStyle(.secondary)
    }

    @ViewBuilder private func freshness(_ value: WidgetFile.Freshness) -> some View {
        if value.availability != .fresh, let at = value.updatedAt {
            Text(entry.messages.string("widget.updated", ["time": formatted(at, date: true)]))
                .font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
    }

    private func interval(_ course: WidgetFile.ClassRow, date: Bool) -> String {
        formatted(course.startsAt, date: date) + "–" + formatted(course.endsAt, date: false)
    }

    private func formatted(_ at: Date, date: Bool) -> String {
        let formatter = DateFormatter()
        formatter.locale = entry.messages.locale
        formatter.timeZone = entry.zone
        formatter.setLocalizedDateFormatFromTemplate(date ? "MMMdHm" : "Hm")
        return formatter.string(from: at)
    }

    private func color(_ value: String?) -> Color {
        guard let value, let rgb = UInt32(value.dropFirst(), radix: 16) else { return .secondary }
        return Color(red: Double((rgb >> 16) & 255) / 255, green: Double((rgb >> 8) & 255) / 255, blue: Double(rgb & 255) / 255)
    }
}
