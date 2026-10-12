import Foundation
import GoogleSignIn
import SwiftRs
import Tauri
import UIKit

private struct SignInArgs: Decodable {
    let clientId: String
}

private struct HandleUrlArgs: Decodable {
    let url: String
}

private let driveAppDataScope = "https://www.googleapis.com/auth/drive.appdata"

final class GoogleDriveAuthPlugin: Plugin {
    @objc func signIn(_ invoke: Invoke) {
        do {
            let _: SignInArgs = try invoke.parseArgs(SignInArgs.self)
        } catch {
            invoke.reject("Google OAuth configuration is invalid.")
            return
        }

        DispatchQueue.main.async {
            guard let presenter = self.manager.viewController else {
                invoke.reject("Main view controller is unavailable.")
                return
            }
            guard let configuredClientId = Bundle.main.object(forInfoDictionaryKey: "GIDClientID") as? String,
                  !configuredClientId.isEmpty else {
                invoke.reject("Google OAuth is missing GIDClientID in Info.plist.")
                return
            }
            let callbackScheme = configuredClientId.split(separator: ".").reversed().joined(separator: ".")
            let registeredSchemes = (Bundle.main.object(forInfoDictionaryKey: "CFBundleURLTypes") as? [[String: Any]] ?? [])
                .flatMap { $0["CFBundleURLSchemes"] as? [String] ?? [] }
            guard registeredSchemes.contains(callbackScheme) else {
                invoke.reject("Google OAuth callback scheme is not registered for this iOS client.")
                return
            }
            GIDSignIn.sharedInstance.configuration = GIDConfiguration(clientID: configuredClientId)
            GIDSignIn.sharedInstance.signIn(withPresenting: presenter) { result, error in
                guard error == nil, let result else {
                    invoke.reject(error?.localizedDescription ?? "Google sign-in failed.")
                    return
                }
                if result.user.grantedScopes?.contains(driveAppDataScope) == true {
                    self.resolve(invoke, user: result.user)
                    return
                }
                result.user.addScopes([driveAppDataScope], presenting: presenter) { scoped, error in
                    guard error == nil, let scoped else {
                        invoke.reject(error?.localizedDescription ?? "Google Drive access was not granted.")
                        return
                    }
                    self.resolve(invoke, user: scoped.user)
                }
            }
        }
    }

    @objc func signOut(_ invoke: Invoke) {
        GIDSignIn.sharedInstance.signOut()
        invoke.resolve()
    }

    @objc func handleUrl(_ invoke: Invoke) {
        let args: HandleUrlArgs
        do {
            args = try invoke.parseArgs(HandleUrlArgs.self)
        } catch {
            invoke.reject("Google OAuth callback is invalid.")
            return
        }
        guard let url = URL(string: args.url) else {
            invoke.reject("Google OAuth callback URL is invalid.")
            return
        }
        invoke.resolve(GIDSignIn.sharedInstance.handle(url))
    }

    private func resolve(_ invoke: Invoke, user: GIDGoogleUser) {
        guard let clientId = GIDSignIn.sharedInstance.configuration?.clientID else {
            invoke.reject("Google OAuth native configuration is unavailable.")
            return
        }
        let expiresIn = max(60, Int(user.accessToken.expirationDate?.timeIntervalSinceNow ?? 3600))
        invoke.resolve([
            "clientId": clientId,
            "accessToken": user.accessToken.tokenString,
            "refreshToken": user.refreshToken.tokenString,
            "expiresIn": expiresIn
        ])
    }
}

@_cdecl("init_plugin_google_drive_auth")
func initPlugin() -> Plugin {
    GoogleDriveAuthPlugin()
}
