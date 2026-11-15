import AuthenticationServices
import IsctCore
import UIKit

/// Presents a system authentication session for a callback owned by Core.
final class BrowserAuthentication: NSObject, NativeBrowserAuth,
    ASWebAuthenticationPresentationContextProviding, @unchecked Sendable
{
    @MainActor private var session: ASWebAuthenticationSession?
    @MainActor private var generation = 0
    @MainActor private var completion: CheckedContinuation<String?, any Error>?

    func authenticate(url: String, callbackScheme: String?) async throws -> String? {
        guard let start = URL(string: url), let callbackScheme else {
            throw SdkError.InvalidInput(detail: "Sign-in address is invalid.")
        }
        return try await present(start, scheme: callbackScheme)
    }

    @MainActor private func present(_ url: URL, scheme: String) async throws -> String? {
        guard session == nil else { throw SdkError.Busy(detail: "A browser session is open.") }
        return try await withCheckedThrowingContinuation { continuation in
            generation += 1
            let generation = self.generation
            completion = continuation
            let session = ASWebAuthenticationSession(url: url, callbackURLScheme: scheme) {
                [weak self] callback, error in
                Task { @MainActor in
                    guard let self, self.generation == generation else { return }
                    if let callback {
                        self.finish(.success(callback.absoluteString))
                    } else if (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin {
                        self.finish(.failure(SdkError.Cancelled))
                    } else {
                        self.finish(.failure(SdkError.Callback(detail: "Browser authentication failed.")))
                    }
                }
            }
            session.presentationContextProvider = self
            self.session = session
            if !session.start() {
                finish(.failure(SdkError.Unavailable(detail: "Browser authentication could not start.")))
            }
        }
    }

    func cancel() async {
        await MainActor.run {
            let running = session
            finish(.failure(SdkError.Cancelled))
            running?.cancel()
        }
    }

    @MainActor private func finish(_ result: Result<String?, any Error>) {
        let completion = completion
        self.completion = nil
        session = nil
        completion?.resume(with: result)
    }

    @MainActor func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first ?? ASPresentationAnchor()
    }
}
