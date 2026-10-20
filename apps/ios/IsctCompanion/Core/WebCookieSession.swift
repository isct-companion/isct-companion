import Foundation
import IsctCore
import WebKit

/// Lets Core's HTTP requests (course files, LMS, 教務Web) use the sign-in of
/// the resident school web view: the cookies its data store holds for a URL,
/// and the cookies Core's responses set, written back to the same store.
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

    func storeSetCookies(url: String, setCookie: [String]) async throws {
        guard let url = URL(string: url) else {
            throw SdkError.InvalidInput(detail: "url")
        }
        // One header per value: folding them into one comma-joined field
        // breaks on the comma inside `Expires`.
        let cookies = setCookie.flatMap { value in
            HTTPCookie.cookies(withResponseHeaderFields: ["Set-Cookie": value], for: url)
        }
        await Self.store(cookies)
    }

    @MainActor private static func store(_ cookies: [HTTPCookie]) async {
        let store = WKWebsiteDataStore.default().httpCookieStore
        for cookie in cookies {
            await store.setCookie(cookie)
        }
    }
}
