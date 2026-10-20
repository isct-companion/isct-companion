import Foundation
import Observation

/// The UI language, read from `settings.locale` like the desktop's.
///
/// Strings are keyed by their English text and translated from the catalogs
/// the desktop uses (`resources/locales/{en,ja,zh}.json`, i18next format with
/// `{{name}}` placeholders); English is the key itself, except for the few
/// dotted keys (`settings.reset.…`) that `en.json` spells out. These catalogs are
/// read directly rather than through a String Catalog because the language is
/// an in-app setting, not the system's, and the placeholders must stay the
/// desktop's named ones so both shells share one translation.
///
/// A view that calls `t` reads `language` while its body runs, so Observation
/// re-renders it when the language changes; nothing else has to pass it down.
@Observable
final class Localizer: @unchecked Sendable {
    static let shared = Localizer()

    /// `en`, `ja` or `zh`.
    private(set) var language = "en"
    @ObservationIgnored private var catalog: [String: String] = [:]
    @ObservationIgnored private var loaded: String?

    /// Switches to a stored `settings.locale`; anything unknown reads English.
    func use(_ stored: String?) {
        let next = stored.flatMap { ["ja", "zh"].contains($0) ? $0 : nil } ?? "en"
        guard next != loaded else { return }
        catalog = Self.load(next)
        loaded = next
        if next != language { language = next }
    }

    func string(_ key: String, _ values: [String: Any]) -> String {
        _ = language
        var text = catalog[key] ?? key
        for (name, value) in values {
            text = text.replacingOccurrences(of: "{{\(name)}}", with: "\(value)")
        }
        return text
    }

    /// The locale dates and numbers are written in, as the desktop's `intlLocale`.
    var locale: Locale {
        Locale(identifier: ["ja": "ja_JP", "zh": "zh_Hans_CN"][language] ?? "en_GB")
    }

    /// School text in the content language: `settings.contentLanguage`, or on
    /// automatic the UI language (`zh` reads Japanese), as the desktop's
    /// `contentLanguage` and `pickLocalized`. The other language fills a gap.
    func pick(_ value: [String: String], content settings: [String: Any]) -> String? {
        let chosen = settings["contentLanguage"] as? String
        let first = ["ja", "en"].contains(chosen ?? "") ? chosen ?? "ja" : (language == "en" ? "en" : "ja")
        let order = first == "en" ? ["en", "ja"] : ["ja", "en"]
        return order.lazy.compactMap { value[$0].flatMap { $0.isEmpty ? nil : $0 } }.first
    }

    private static func load(_ language: String) -> [String: String] {
        guard let url = Bundle.main.url(forResource: language, withExtension: "json"),
            let data = try? Data(contentsOf: url),
            let catalog = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return catalog
    }
}

/// The UI string for an English key, with `{{name}}` placeholders filled.
func t(_ key: String, _ values: [String: Any] = [:]) -> String {
    Localizer.shared.string(key, values)
}
