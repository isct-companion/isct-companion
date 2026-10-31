import UIKit
import UniformTypeIdentifiers

/// What the share and action extensions both do: read what was shared into
/// files of their own, and bring the app forward on its inbox.
enum ExtensionInput {
    /// Copies each attachment out of its provider while the provider's
    /// temporary file still exists; plain text and links are not files to
    /// hand in.
    @MainActor
    static func files(_ context: NSExtensionContext?) async -> [SharedFile] {
        let providers = (context?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
        let staging = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        var files: [SharedFile] = []
        for provider in providers {
            if let file = await copy(provider, into: staging) { files.append(file) }
        }
        return files
    }

    /// Brings the app forward on its inbox. Extensions have no API for this
    /// (`UIApplication.open` is unavailable to them), so the URL goes to the
    /// application found up the responder chain through the Objective-C
    /// runtime. Should that ever stop working, the files still wait in the
    /// inbox for the next time the app opens.
    static func openApp(from start: UIResponder) {
        guard let url = URL(string: "isctcompanion://inbox") else { return }
        let selector = NSSelectorFromString("openURL:options:completionHandler:")
        var responder: UIResponder? = start
        while let current = responder {
            if current.isKind(of: UIApplication.self), current.responds(to: selector) {
                typealias Open = @convention(c) (AnyObject, Selector, NSURL, NSDictionary, AnyObject?) -> Void
                let open = unsafeBitCast(current.method(for: selector), to: Open.self)
                open(current, selector, url as NSURL, NSDictionary(), nil)
                return
            }
            responder = current.next
        }
    }

    /// The file types the provider offers, most specific first.
    private static func fileTypes(_ provider: NSItemProvider) -> [UTType] {
        provider.registeredTypeIdentifiers
            .compactMap(UTType.init)
            .filter { $0.conforms(to: .data) && !$0.conforms(to: .plainText) && !$0.conforms(to: .url) }
    }

    /// The first file type whose content loads; a type can be offered and
    /// still fail. An app that shares a file only by its URL offers no
    /// content type at all, so that URL is read last.
    private static func copy(_ provider: NSItemProvider, into folder: URL) async -> SharedFile? {
        for type in fileTypes(provider) {
            if let file = await copyContent(provider, type: type, into: folder) { return file }
        }
        guard provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) else { return nil }
        return await copyFileURL(provider, into: folder)
    }

    private static func copyContent(_ provider: NSItemProvider, type: UTType, into folder: URL) async -> SharedFile? {
        let suggested = provider.suggestedName
        return await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                continuation.resume(returning: url.flatMap { store($0, suggested: suggested, type: type, into: folder) })
            }
        }
    }

    private static func copyFileURL(_ provider: NSItemProvider, into folder: URL) async -> SharedFile? {
        let suggested = provider.suggestedName
        return await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier) { item, _ in
                let url = (item as? URL) ?? (item as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) }
                guard let url, url.isFileURL else { return continuation.resume(returning: nil) }
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                continuation.resume(returning: store(url, suggested: suggested, type: nil, into: folder))
            }
        }
    }

    /// Copies `url` into `folder` while the provider's file still exists.
    private static func store(_ url: URL, suggested: String?, type: UTType?, into folder: URL) -> SharedFile? {
        let base = suggested ?? url.deletingPathExtension().lastPathComponent
        let ext = url.pathExtension.isEmpty ? (type?.preferredFilenameExtension ?? "") : url.pathExtension
        let name = ext.isEmpty || base.hasSuffix(".\(ext)") ? base : "\(base).\(ext)"
        let target = folder.appendingPathComponent(UUID().uuidString).appendingPathExtension(ext)
        do {
            try FileManager.default.copyItem(at: url, to: target)
            return SharedFile(url: target, name: name)
        } catch {
            return nil
        }
    }
}

/// A received file, copied where the extension can keep it until it is added.
struct SharedFile: Identifiable, Hashable {
    let url: URL
    let name: String
    var id: URL { url }
}
