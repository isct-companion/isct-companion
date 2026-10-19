import Foundation
import IsctCore
import WebKit

/// Lets Core download LMS files with the sign-in of the resident school web
/// view: the cookies its data store holds for the file's URL.
final class WebCookieSession: NativeDownloadSession {
    func cookieHeader(url: String) async throws -> String {
        guard let url = URL(string: url), let host = url.host?.lowercased() else {
            throw SdkError.InvalidInput(detail: "url")
        }
        let store = await MainActor.run { WKWebsiteDataStore.default().httpCookieStore }
        let cookies = await store.allCookies().filter { cookie in
            let domain = cookie.domain.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "."))
            let path = url.path.isEmpty ? "/" : url.path
            return (host == domain || host.hasSuffix("." + domain))
                && path.hasPrefix(cookie.path)
                && (!cookie.isSecure || url.scheme == "https")
                && (cookie.expiresDate.map { $0 > Date() } ?? true)
        }
        return HTTPCookie.requestHeaderFields(with: cookies)["Cookie"] ?? ""
    }
}
