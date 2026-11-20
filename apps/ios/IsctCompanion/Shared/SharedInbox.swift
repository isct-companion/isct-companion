import Foundation
import UniformTypeIdentifiers

/// The App Group only carries a share from the extension to the app. Each
/// transfer owns a distinct directory, which the app takes into temporary storage.
enum SharedInbox {
    struct File: Codable, Hashable, Identifiable {
        let id: String
        let name: String

        func url(in folder: URL) -> URL { folder.appendingPathComponent(id) }
    }

    struct Transfer: Identifiable {
        let folder: URL
        let files: [File]
        let choice: Choice?
        var id: URL { folder }

        func discard() { try? FileManager.default.removeItem(at: folder) }
    }

    struct Choice: Codable, Hashable, Identifiable {
        let id: String
        let title: String
        let course: String?
        let due: String
    }

    struct Choices: Codable {
        var language: String
        var assignments: [Choice]
    }

    private struct Handoff: Codable {
        let files: [File]
        let choice: Choice?
    }

    private static var container: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: "group.app.isctcompanion")
    }

    private static var transfers: URL? { container?.appendingPathComponent("HandIn", isDirectory: true) }

    static func temporaryFolder() throws -> URL {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }

    static func copy(_ source: URL, name: String, into folder: URL) throws -> File {
        let file = newFile(name: name)
        try FileManager.default.copyItem(at: source, to: file.url(in: folder))
        return file
    }

    static func newFile(name: String) -> File {
        let ext = (name as NSString).pathExtension
        let id = UUID().uuidString
        return File(id: ext.isEmpty ? id : "\(id).\(ext)", name: name)
    }

    /// Publish an entire selection so the app cannot consume a partial one.
    static func publish(_ transfer: Transfer, choice: Choice?) throws {
        guard let transfers else { throw CocoaError(.fileNoSuchFile) }
        let id = UUID().uuidString
        let staging = transfers.appendingPathComponent(".\(id)", isDirectory: true)
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: staging) }
        for file in transfer.files {
            try FileManager.default.copyItem(at: file.url(in: transfer.folder), to: file.url(in: staging))
        }
        let handoff = try JSONEncoder().encode(Handoff(files: transfer.files, choice: choice))
        try handoff.write(to: staging.appendingPathComponent("handoff.json"))
        try FileManager.default.moveItem(at: staging, to: transfers.appendingPathComponent(id, isDirectory: true))
    }

    static func take() -> Transfer? {
        guard let transfers, let folders = try? FileManager.default.contentsOfDirectory(
            at: transfers, includingPropertiesForKeys: nil, options: .skipsHiddenFiles
        ) else { return nil }
        for folder in folders {
            guard let data = try? Data(contentsOf: folder.appendingPathComponent("handoff.json")),
                let handoff = try? JSONDecoder().decode(Handoff.self, from: data)
            else { continue }
            let destination = FileManager.default.temporaryDirectory.appendingPathComponent(folder.lastPathComponent, isDirectory: true)
            if (try? FileManager.default.moveItem(at: folder, to: destination)) != nil {
                return Transfer(folder: destination, files: handoff.files, choice: handoff.choice)
            }
        }
        return nil
    }

    static func writeChoices(_ choices: Choices) {
        guard let url = container?.appendingPathComponent("hand-in-assignments.json"),
            let data = try? JSONEncoder().encode(choices)
        else { return }
        try? data.write(to: url, options: .atomic)
    }

    static func readChoices() -> Choices? {
        guard let url = container?.appendingPathComponent("hand-in-assignments.json"),
            let data = try? Data(contentsOf: url)
        else { return nil }
        return try? JSONDecoder().decode(Choices.self, from: data)
    }

    /// Starts every read before returning, while the native drop grant is live.
    @MainActor
    static func startCopy(
        _ providers: [NSItemProvider], into folder: URL,
        completion: @escaping @MainActor (Result<[File], Error>) -> Void
    ) {
        let group = DispatchGroup()
        var results: [Int: Result<File, Error>] = [:]
        for (index, provider) in providers.enumerated() {
            group.enter()
            copy(provider, into: folder) { result in
                DispatchQueue.main.async {
                    results[index] = result
                    group.leave()
                }
            }
        }
        group.notify(queue: .main) {
            completion(Result { try results.sorted { $0.key < $1.key }.map { try $0.value.get() } })
        }
    }

    @MainActor
    static func copy(_ providers: [NSItemProvider], into folder: URL) async throws -> [File] {
        try await withCheckedThrowingContinuation { continuation in
            startCopy(providers, into: folder) { continuation.resume(with: $0) }
        }
    }

    private static func copy(
        _ provider: NSItemProvider, into folder: URL,
        completion: @escaping (Result<File, Error>) -> Void
    ) {
        let suggested = provider.suggestedName
        let types = provider.registeredTypeIdentifiers.compactMap(UTType.init)
        if let type = types.first(where: { $0.conforms(to: .data) && !$0.conforms(to: .plainText) && !$0.conforms(to: .url) }) {
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                completion(Result {
                    guard let url else { throw error ?? CocoaError(.fileReadUnknown) }
                    return try store(url, suggested: suggested, type: type, into: folder)
                })
            }
        } else if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier) { item, error in
                completion(Result {
                    let url = (item as? URL) ?? (item as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) }
                    guard let url, url.isFileURL else { throw error ?? CocoaError(.fileReadUnknown) }
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    return try store(url, suggested: suggested, type: nil, into: folder)
                })
            }
        } else {
            completion(.failure(CocoaError(.fileReadUnsupportedScheme)))
        }
    }

    private static func store(_ source: URL, suggested: String?, type: UTType?, into folder: URL) throws -> File {
        let base = suggested ?? source.deletingPathExtension().lastPathComponent
        let ext = source.pathExtension.isEmpty ? (type?.preferredFilenameExtension ?? "") : source.pathExtension
        let name = ext.isEmpty || base.hasSuffix(".\(ext)") ? base : "\(base).\(ext)"
        return try copy(source, name: name, into: folder)
    }
}
