import AuthenticationServices
import IsctCore
import UIKit

/// Google sign-in for Drive, in a web authentication session. Core builds the
/// PKCE request, checks the redirect and keeps the token in the Keychain; this
/// only shows Google's page.
final class GoogleSignIn: NSObject, NativeWebAuthentication, ASWebAuthenticationPresentationContextProviding,
    @unchecked Sendable
{
    /// The running session; ASWebAuthenticationSession must stay alive until it ends.
    private var session: ASWebAuthenticationSession?

    func googleClient() -> GoogleClient {
        // The iOS client in config/oauth-clients.yaml, set by project.yml.
        let id = Bundle.main.object(forInfoDictionaryKey: "IsctGoogleClientID") as? String ?? ""
        // An iOS client accepts its reversed ID as the redirect scheme.
        let scheme = id.split(separator: ".").reversed().joined(separator: ".")
        return GoogleClient(clientId: id, redirectUri: "\(scheme):/oauthredirect")
    }

    func authenticate(url: String, callbackScheme: String) async throws -> String {
        guard let start = URL(string: url) else {
            throw SdkError.InvalidInput(detail: "Sign-in address is invalid.")
        }
        return try await withCheckedThrowingContinuation { continuation in
            Task { @MainActor in
                let session = ASWebAuthenticationSession(url: start, callbackURLScheme: callbackScheme) {
                    [weak self] callback, error in
                    self?.session = nil
                    if let callback {
                        continuation.resume(returning: callback.absoluteString)
                    } else if (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin {
                        continuation.resume(throwing: SdkError.Cancelled)
                    } else {
                        continuation.resume(throwing: SdkError.Callback(
                            detail: error?.localizedDescription ?? "Google sign-in failed."))
                    }
                }
                session.presentationContextProvider = self
                self.session = session
                if !session.start() {
                    self.session = nil
                    continuation.resume(throwing: SdkError.Unavailable(detail: "Google sign-in could not start."))
                }
            }
        }
    }

    @MainActor func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first ?? ASPresentationAnchor()
    }
}
