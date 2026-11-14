import Foundation

public struct MessageCatalog: Sendable {
    public let language: String
    private let messages: [String: String]
    private let english: [String: String]

    public static var systemLanguage: String? {
        Locale.preferredLanguages.first.flatMap { Locale(identifier: $0).language.languageCode?.identifier }
    }

    public static func language(_ value: String?) -> String {
        switch value {
        case "ja": return "ja"
        case "zh": return "zh"
        default: return "en"
        }
    }

    public init(language: String?, bundle: Bundle) {
        self.language = Self.language(language)
        english = Self.load("en", in: bundle)
        messages = self.language == "en" ? english : Self.load(self.language, in: bundle)
    }

    public var locale: Locale {
        Locale(identifier: ["ja": "ja_JP", "zh": "zh_Hans_CN"][language] ?? "en_GB")
    }

    public func lookup(_ key: String, _ values: [String: Any]) -> String? {
        guard var text = messages[key] ?? english[key] else { return nil }
        for (name, value) in values {
            text = text.replacingOccurrences(of: "{{\(name)}}", with: "\(value)")
        }
        return text
    }

    public func string(_ key: String, _ values: [String: Any] = [:]) -> String {
        lookup(key, values) ?? key
    }

    private static func load(_ language: String, in bundle: Bundle) -> [String: String] {
        guard let url = bundle.url(forResource: language, withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let catalog = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return catalog
    }
}
