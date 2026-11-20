import UIKit

/// "Open in IsctCompanion" in the share sheet's actions: what was shared goes
/// to the app's hand-in picker. The person chooses a target in the app. Nothing is picked or submitted here; the
/// share extension is the one with a sheet.
final class OpenViewController: UIViewController {
    private var started = false

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // Once on screen, so the responder chain reaches the application.
        guard !started else { return }
        started = true
        Task {
            do {
                if let transfer = try await ExtensionInput.transfer(extensionContext) {
                    defer { transfer.discard() }
                    try SharedInbox.publish(transfer, choice: nil)
                }
            } catch {
                extensionContext?.cancelRequest(withError: error)
                return
            }
            ExtensionInput.openApp(from: self)
            extensionContext?.completeRequest(returningItems: nil)
        }
    }
}
