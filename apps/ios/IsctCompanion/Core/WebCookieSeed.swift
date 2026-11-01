import Foundation
import IsctCore
import WebKit

/// `NativeWebCookies` over the store the school web views share. Core writes
/// the account's sign-in cookies into it before it opens a school page for
/// the person, and clears the school ones on sign-out, account switch and
/// reset. Core never reads them back.
///
/// Calls run one at a time, each after the previous one completed, and a
/// write older than the last clear is dropped, so a sign-out is never undone
/// by a write that was already on its way.
final class WebCookieSeed: NativeWebCookies, @unchecked Sendable {
    /// The last call queued; the next one starts once it has completed.
    @MainActor private var tail: Task<Void, Never>?
    @MainActor private var lastClearedGeneration: UInt64 = 0

    func write(generation: UInt64, cookies: [WebCookie]) async throws {
        // WebKit's own Set-Cookie parser for the URL each was received at:
        // a cookie built from properties loses the leading dot of `Domain`.
        let parsed = try cookies.flatMap { cookie in
            guard let url = URL(string: cookie.url) else { throw SdkError.InvalidInput(detail: "url") }
            let read = HTTPCookie.cookies(withResponseHeaderFields: ["Set-Cookie": cookie.setCookie], for: url)
            guard !read.isEmpty else { throw SdkError.InvalidInput(detail: "set-cookie") }
            return read
        }
        await serially { [self] in
            guard generation >= lastClearedGeneration else { return }
            let store = WKWebsiteDataStore.default().httpCookieStore
            for cookie in parsed {
                await store.setCookie(cookie)
            }
        }
    }

    /// Removes every cookie of a school domain, then the rest of those
    /// sites' website data (storage, caches). Core decides which domains are
    /// school ones.
    func clear(generation: UInt64) async throws {
        await serially { [self] in
            lastClearedGeneration = max(lastClearedGeneration, generation)
            let data = WKWebsiteDataStore.default()
            let store = data.httpCookieStore
            for cookie in await store.allCookies() where isSchoolCookieDomain(domain: cookie.domain) {
                await store.deleteCookie(cookie)
            }
            let types = WKWebsiteDataStore.allWebsiteDataTypes()
            let school = await data.dataRecords(ofTypes: types).filter { isSchoolCookieDomain(domain: $0.displayName) }
            await data.removeData(ofTypes: types, for: school)
        }
    }

    /// Runs `work` once every earlier call has completed, and returns when it has.
    @MainActor private func serially(_ work: @escaping @MainActor () async -> Void) async {
        let previous = tail
        let task = Task { @MainActor in
            await previous?.value
            await work()
        }
        tail = task
        await task.value
    }
}
