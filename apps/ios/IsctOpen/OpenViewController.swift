import UIKit

/// "Open in IsctCompanion" in the share sheet's actions: what was shared goes
/// to the inbox and the app comes forward with it in the corner stack, as a
/// file chosen in the app would. Nothing is picked or submitted here; the
/// share extension is the one with a sheet.
final class OpenViewController: UIViewController {
    private var started = false

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // Once on screen, so the responder chain reaches the application.
        guard !started else { return }
        started = true
        Task {
            for file in await ExtensionInput.files(extensionContext) {
                _ = try? SharedInbox.add(file.url, name: file.name, assignmentId: nil)
            }
            ExtensionInput.openApp(from: self)
            extensionContext?.completeRequest(returningItems: nil)
        }
    }
}
