import Foundation
import IsctCore
import WebKit

/// `NativeWebCookies` over the store the school web views share. Core writes
/// an account's verified sign-in cookies into it before it opens a school
/// page for the person, and clears an account's school websites on sign-out,
/// account switch and reset. Core never reads them back.
///
/// Core runs the calls one at a time and drops stale writes; each call here
/// answers once WebKit has called back, however long that takes.
final class WebCookieSeed: NativeWebCookies, @unchecked Sendable {
    func write(cookies: [WebCookie]) async throws {
        // WebKit's own Set-Cookie parser for the URL each was received at:
        // a cookie built from properties loses the leading dot of `Domain`.
        let parsed = try cookies.flatMap { cookie in
            guard let url = URL(string: cookie.url) else { throw SdkError.InvalidInput(detail: "url") }
            let read = HTTPCookie.cookies(withResponseHeaderFields: ["Set-Cookie": cookie.setCookie], for: url)
            guard !read.isEmpty else { throw SdkError.InvalidInput(detail: "set-cookie") }
            return read
        }
        await onMain {
            let store = WKWebsiteDataStore.default().httpCookieStore
            for cookie in parsed {
                await store.setCookie(cookie)
            }
        }
    }

    /// Removes every cookie of a domain `scope` covers, then the rest of
    /// those sites' website data (storage, caches). Core decides which
    /// domains those are (`webClearCovers`).
    func clear(scope: WebClearScope) async throws {
        await onMain {
            let data = WKWebsiteDataStore.default()
            let store = data.httpCookieStore
            for cookie in await store.allCookies() where webClearCovers(scope: scope, domain: cookie.domain) {
                await store.deleteCookie(cookie)
            }
            let types = WKWebsiteDataStore.allWebsiteDataTypes()
            let school = await data.dataRecords(ofTypes: types).filter { webClearCovers(scope: scope, domain: $0.displayName) }
            await data.removeData(ofTypes: types, for: school)
        }
    }

    /// WebKit removes one site's data by its record.
    func clearsBySite() -> Bool { true }

    /// Runs `work` on the main actor, where WebKit's store lives, and returns
    /// when it has.
    private func onMain(_ work: @escaping @MainActor () async -> Void) async {
        await Task { @MainActor in await work() }.value
    }
}
