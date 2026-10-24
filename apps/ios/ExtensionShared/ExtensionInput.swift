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
            guard let type = fileType(provider) else { continue }
            if let file = await copy(provider, type: type, into: staging) { files.append(file) }
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

    /// The most specific file type the provider offers.
    private static func fileType(_ provider: NSItemProvider) -> UTType? {
        provider.registeredTypeIdentifiers
            .compactMap(UTType.init)
            .first { $0.conforms(to: .data) && !$0.conforms(to: .plainText) && !$0.conforms(to: .url) }
    }

    private static func copy(_ provider: NSItemProvider, type: UTType, into folder: URL) async -> SharedFile? {
        let suggested = provider.suggestedName
        return await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                guard let url else { return continuation.resume(returning: nil) }
                let base = suggested ?? url.deletingPathExtension().lastPathComponent
                let ext = url.pathExtension.isEmpty ? (type.preferredFilenameExtension ?? "") : url.pathExtension
                let name = ext.isEmpty || base.hasSuffix(".\(ext)") ? base : "\(base).\(ext)"
                let target = folder.appendingPathComponent(UUID().uuidString).appendingPathExtension(ext)
                do {
                    try FileManager.default.copyItem(at: url, to: target)
                    continuation.resume(returning: SharedFile(url: target, name: name))
                } catch {
                    continuation.resume(returning: nil)
                }
            }
        }
    }
}

/// A received file, copied where the extension can keep it until it is added.
struct SharedFile: Identifiable, Hashable {
    let url: URL
    let name: String
    var id: URL { url }
}
