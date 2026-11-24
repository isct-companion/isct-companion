import UIKit

/// What the share and action extensions both do: read what was shared into
/// files of their own, and bring the app forward on its inbox.
enum ExtensionInput {
    @MainActor
    static func transfer(_ context: NSExtensionContext?) async throws -> SharedInbox.Transfer? {
        let providers = (context?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
        guard !providers.isEmpty else { return nil }
        let folder = try SharedInbox.temporaryFolder()
        do {
            let files = try await SharedInbox.copy(providers, into: folder)
            guard !files.isEmpty else {
                try? FileManager.default.removeItem(at: folder)
                return nil
            }
            return SharedInbox.Transfer(folder: folder, files: files, choice: nil)
        } catch {
            try? FileManager.default.removeItem(at: folder)
            throw error
        }
    }

    /// Brings the app forward on its inbox, or on another `page` it opens
    /// from a link (`print`, the campus print screen). Extensions have no API for this
    /// (`UIApplication.open` is unavailable to them), so the URL goes to the
    /// application found up the responder chain through the Objective-C
    /// runtime. Should that ever stop working, the files still wait in the
    /// inbox for the next time the app opens.
    static func openApp(from start: UIResponder, to page: String = "inbox") {
        guard let url = URL(string: "isctcompanion://\(page)") else { return }
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
}
