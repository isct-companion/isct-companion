import Foundation

public enum WidgetPurpose: String, CaseIterable, Sendable {
    case courses, assignments

    public var kind: String { "app.isctcompanion.widgets." + rawValue }
}

public enum WidgetDestination: Sendable, Decodable {
    case course(key: String, date: String)
    case assignment(id: String)
    case timetable, assignments, sync

    private enum Key: String, CodingKey { case kind, key, date, id }
    private enum Kind: String, Decodable { case course, assignment, timetable, assignments, sync }

    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: Key.self)
        switch try fields.decode(Kind.self, forKey: .kind) {
        case .course:
            self = .course(key: try fields.decode(String.self, forKey: .key),
                           date: try fields.decode(String.self, forKey: .date))
        case .assignment: self = .assignment(id: try fields.decode(String.self, forKey: .id))
        case .timetable: self = .timetable
        case .assignments: self = .assignments
        case .sync: self = .sync
        }
    }

    public var url: URL? {
        var components = URLComponents()
        components.scheme = "isctcompanion"
        components.host = "widget"
        switch self {
        case let .course(key, date):
            components.path = "/course"
            components.queryItems = [URLQueryItem(name: "key", value: key), URLQueryItem(name: "date", value: date)]
        case let .assignment(id):
            components.path = "/assignment"
            components.queryItems = [URLQueryItem(name: "id", value: id)]
        case .timetable: components.path = "/timetable"
        case .assignments: components.path = "/assignments"
        case .sync: components.path = "/sync"
        }
        return components.url
    }
}

struct WidgetFile: Decodable {
    static let schemaVersion = 1

    let schemaVersion: Int
    let locale: String
    let schoolTimeZone: String
    let coverageStart: Date
    let coverageEnd: Date
    let terminalContent: Terminal
    let courseEntries: [Entry<Course>]
    let assignmentEntries: [Entry<Assignments>]

    struct Entry<Content: Decodable>: Decodable {
        let effectiveAt: Date
        let content: Content
    }

    struct Terminal: Decodable {
        let messageKey: String
        let destination: WidgetDestination
    }

    struct Freshness: Decodable {
        enum Availability: String, Decodable { case fresh, cached, unknown }
        let availability: Availability
        let coverageComplete: Bool
        let updatedAt: Date?
    }

    struct Course: Decodable {
        enum Phase: String, Decodable { case beforeClasses, betweenClasses, inClass, afterClasses, noClasses }
        let date: String
        let phase: Phase
        let freshness: Freshness
        let primary: ClassRow?
        let upcoming: [ClassRow]
        let today: [ClassRow]
        let destination: WidgetDestination
    }

    struct ClassRow: Decodable {
        let title: String
        let room: String
        let color: String
        let startsAt: Date
        let endsAt: Date
        let date: String
        let destination: WidgetDestination
    }

    struct Assignments: Decodable {
        let freshness: Freshness
        let rows: [AssignmentRow]
        let destination: WidgetDestination
    }

    struct AssignmentRow: Decodable {
        let title: String
        let courseTitle: String?
        let color: String?
        let dueAt: Date?
        let overdue: Bool
        let pinned: Bool
        let destination: WidgetDestination
    }

    static func read(from file: URL) throws -> Self {
        struct Version: Decodable { let schemaVersion: Int }
        let data = try Data(contentsOf: file)
        let decoder = JSONDecoder()
        guard try decoder.decode(Version.self, from: data).schemaVersion == schemaVersion else {
            throw CocoaError(.fileReadUnknown)
        }
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            if let instant = try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(text) {
                return instant
            }
            return try Date.ISO8601FormatStyle().parse(text)
        }
        return try decoder.decode(Self.self, from: data)
    }
}

public enum WidgetLocation {
    public static func directory(in container: URL) -> URL {
        container.appendingPathComponent("Widgets", isDirectory: true)
    }

    public static func file(in container: URL) -> URL {
        directory(in: container).appendingPathComponent("current.json", isDirectory: false)
    }
}
