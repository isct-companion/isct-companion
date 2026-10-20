import IsctCore
import WebKit

/// Signs the school web views out for Settings → Reset: every website data
/// record (cookies, storage, caches) of a school domain in the store the
/// views and `WebCookieSession` share. Core decides which domains are school ones.
final class SchoolWebSignOut: NativeWebSignOut {
    func signOutSchoolWeb() async throws {
        await Self.remove()
    }

    @MainActor private static func remove() async {
        let store = WKWebsiteDataStore.default()
        let types = WKWebsiteDataStore.allWebsiteDataTypes()
        let school = await store.dataRecords(ofTypes: types).filter { isSchoolCookieDomain(domain: $0.displayName) }
        await store.removeData(ofTypes: types, for: school)
    }
}
