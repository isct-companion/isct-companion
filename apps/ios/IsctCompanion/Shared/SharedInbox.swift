import Foundation

/// The App Group folder the share extension and the app both see. The
/// extension puts what was shared there; the app hands each file to an
/// assignment's workspace through Core and then removes it. Nothing here
/// decides anything: which assignment a file belongs to is the person's
/// choice, and the hand-in rules are Core's.
///
/// ```text
/// Inbox/<id>/item.json      name, the assignment chosen in the share sheet
/// Inbox/<id>/<file>
/// assignments.json          the open assignments the share sheet offers
/// ```
enum SharedInbox {
    static let group = "group.app.isctcompanion"

    /// One shared file.
    struct Item: Codable, Hashable, Identifiable {
        let id: String
        /// The name it is handed in under.
        let name: String
        /// Picked in the share sheet; the app hands it in there on its next open.
        var assignmentId: String?
        let addedAt: Date

        /// Where the file itself is.
        var url: URL? { SharedInbox.folder(id)?.appendingPathComponent(name) }
    }

    /// One open assignment of the quarter, as the app last saw it.
    struct Choice: Codable, Hashable, Identifiable {
        let id: String
        let title: String
        let course: String?
        /// The deadline as the app wrote it, in the app's language.
        let due: String
    }

    /// The share sheet's list, written by the app with its language.
    struct Choices: Codable {
        var language: String
        var assignments: [Choice]
    }

    private static var container: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
    }

    private static var inbox: URL? { container?.appendingPathComponent("Inbox", isDirectory: true) }

    private static func folder(_ id: String) -> URL? { inbox?.appendingPathComponent(id, isDirectory: true) }

    /// Copies a received file in; the source is left as it is.
    @discardableResult
    static func add(_ source: URL, name: String, assignmentId: String?) throws -> Item {
        guard let base = inbox else { throw CocoaError(.fileNoSuchFile) }
        let item = Item(id: UUID().uuidString, name: safe(name), assignmentId: assignmentId, addedAt: Date())
        let folder = base.appendingPathComponent(item.id, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        do {
            try FileManager.default.copyItem(at: source, to: folder.appendingPathComponent(item.name))
            try JSONEncoder().encode(item).write(to: folder.appendingPathComponent("item.json"), options: .atomic)
        } catch {
            try? FileManager.default.removeItem(at: folder)
            throw error
        }
        return item
    }

    /// Every complete item, oldest first.
    static func items() -> [Item] {
        guard let base = inbox,
            let folders = try? FileManager.default.contentsOfDirectory(at: base, includingPropertiesForKeys: nil)
        else { return [] }
        return folders.compactMap { folder in
            guard let data = try? Data(contentsOf: folder.appendingPathComponent("item.json")),
                let item = try? JSONDecoder().decode(Item.self, from: data),
                let url = item.url, FileManager.default.fileExists(atPath: url.path)
            else { return nil }
            return item
        }
        .sorted { $0.addedAt < $1.addedAt }
    }

    static func remove(_ item: Item) {
        if let folder = folder(item.id) { try? FileManager.default.removeItem(at: folder) }
    }

    static func writeChoices(_ choices: Choices) {
        guard let url = container?.appendingPathComponent("assignments.json"),
            let data = try? JSONEncoder().encode(choices)
        else { return }
        try? data.write(to: url, options: .atomic)
    }

    static func readChoices() -> Choices? {
        guard let url = container?.appendingPathComponent("assignments.json"),
            let data = try? Data(contentsOf: url)
        else { return nil }
        return try? JSONDecoder().decode(Choices.self, from: data)
    }

    /// A file name without folders; Core makes it safe for every platform.
    private static func safe(_ name: String) -> String {
        let last = (name as NSString).lastPathComponent.trimmingCharacters(in: .whitespaces)
        return last.isEmpty || last == "." || last == ".." ? "file" : last
    }
}
