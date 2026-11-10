import Foundation

/// Rows of the `records(fields)` view query, indexed by id. Core read
/// models return ids (course detail, timetable blocks); screens look the rows
/// up here to render them. No decision is made on these rows.
struct WorkspaceRecords {
    typealias Row = [String: Any]
    private(set) var courses: [String: Row] = [:]
    private(set) var assignments: [String: Row] = [:]
    /// The complete settings object; forms edit a copy and replace it.
    private(set) var settings: Row = [:]

    init() {}

    init(json: String) {
        guard let data = json.data(using: .utf8),
            let root = (try? JSONSerialization.jsonObject(with: data)) as? Row
        else { return }
        courses = Self.index(root["courses"])
        assignments = Self.index(root["assignments"])
        settings = root["settings"] as? Row ?? [:]
    }

    private static func index(_ value: Any?) -> [String: Row] {
        var rows: [String: Row] = [:]
        for row in value as? [Row] ?? [] {
            if let id = row["id"] as? String { rows[id] = row }
        }
        return rows
    }
}

extension Dictionary where Key == String, Value == Any {
    func text(_ key: String) -> String? {
        (self[key] as? String).flatMap { $0.isEmpty ? nil : $0 }
    }
}
