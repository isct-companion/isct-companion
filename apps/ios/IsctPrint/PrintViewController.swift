import UIKit

/// "Print via campus printer" in the share sheet's actions: the PDF or
/// photo shared goes to the app's own print screen, where the person picks
/// pages and options and Core sends it. Nothing is printed here; like
/// "Hand in assignment", this only hands the file over and opens the app.
final class PrintViewController: UIViewController {
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
                    try SharedInbox.publish(transfer, choice: nil, to: .print)
                }
            } catch {
                extensionContext?.cancelRequest(withError: error)
                return
            }
            ExtensionInput.openApp(from: self, to: "print")
            extensionContext?.completeRequest(returningItems: nil)
        }
    }
}
