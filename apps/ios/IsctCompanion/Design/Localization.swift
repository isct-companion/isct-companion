import Foundation
import IsctWidgets
import Observation

/// The UI language, read from `settings.locale` like the desktop's.
///
/// Strings are keyed by namespaced message keys (`settings.reset.caches.title`)
/// and read from the catalogs the desktop uses
/// (`resources/locales/{en,ja,zh}.json`, i18next format with `{{name}}`
/// placeholders). `en.json` holds every key with its English copy, and a
/// language that lacks a key reads the English one. These catalogs are read
/// directly rather than through a String Catalog because the language is an
/// in-app setting, not the system's, and the placeholders must stay the
/// desktop's named ones so both shells share one translation.
///
/// A view that calls `t` reads `language` while its body runs, so Observation
/// re-renders it when the language changes; nothing else has to pass it down.
@Observable
final class Localizer: @unchecked Sendable {
    static let shared = Localizer()

    /// `en`, `ja` or `zh`.
    private var catalog = MessageCatalog(language: MessageCatalog.systemLanguage, bundle: .main)
    var language: String { catalog.language }

    private init() {}

    func use(_ stored: String?) {
        let next = MessageCatalog.language(stored)
        guard next != language else { return }
        catalog = MessageCatalog(language: next, bundle: .main)
    }

    func lookup(_ key: String, _ values: [String: Any]) -> String? {
        catalog.lookup(key, values)
    }

    func string(_ key: String, _ values: [String: Any]) -> String {
        catalog.string(key, values)
    }

    var locale: Locale { catalog.locale }

    /// School text in the content language: `settings.contentLanguage`, or on
    /// automatic the UI language (`zh` reads Japanese), as the desktop's
    /// `contentLanguage` and `pickLocalized`. The other language fills a gap.
    func pick(_ value: [String: String], content settings: [String: Any]) -> String? {
        let chosen = settings["contentLanguage"] as? String
        let first = ["ja", "en"].contains(chosen ?? "") ? chosen ?? "ja" : (language == "en" ? "en" : "ja")
        let order = first == "en" ? ["en", "ja"] : ["ja", "en"]
        return order.lazy.compactMap { value[$0].flatMap { $0.isEmpty ? nil : $0 } }.first
    }

}

/// The UI string for a catalog key, with `{{name}}` placeholders filled.
func t(_ key: String, _ values: [String: Any] = [:]) -> String {
    Localizer.shared.string(key, values)
}

/// The string for a key another component owns; `nil` when no catalog has it.
func lookup(_ key: String, _ values: [String: Any] = [:]) -> String? {
    Localizer.shared.lookup(key, values)
}
