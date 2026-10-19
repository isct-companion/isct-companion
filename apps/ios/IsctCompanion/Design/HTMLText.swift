import SwiftUI
import UIKit

/// LMS text (Core's sanitized `bodyHtml`) as native text: paragraphs, lists,
/// emphasis and links keep their shape; colours follow the app, so dark mode
/// and Dynamic Type apply. Links open in the in-app school page.
///
/// The text is there on first layout, so a card never grows under an
/// animation: screens warm `HTMLCache` ahead of showing it, and a miss
/// renders on the spot.
struct HTMLText: View {
    let html: String
    @Environment(Navigator.self) private var navigator

    var body: some View {
        Text(HTMLCache.text(html))
            .font(.body)
            .foregroundStyle(Theme.ink)
            .tint(Theme.accent)
            .textSelection(.enabled)
            .environment(\.openURL, OpenURLAction { url in
                guard url.host == "lms.s.isct.ac.jp" else { return .systemAction }
                navigator.push(WebRouter.PushedPage(site: .scienceLms, target: url.absoluteString, title: ""))
                return .handled
            })
    }

    /// WebKit's HTML import runs on the main thread; the body's own colours
    /// and fonts are dropped so the app's text style applies.
    @MainActor
    static func render(_ html: String) -> AttributedString {
        let styled = "<style>body{font-family:-apple-system;font-size:16px}</style>" + html
        guard let data = styled.data(using: .utf8),
            let imported = try? NSMutableAttributedString(
                data: data,
                options: [.documentType: NSAttributedString.DocumentType.html, .characterEncoding: String.Encoding.utf8.rawValue],
                documentAttributes: nil)
        else { return AttributedString(html) }
        let whole = NSRange(location: 0, length: imported.length)
        imported.removeAttribute(.foregroundColor, range: whole)
        imported.removeAttribute(.backgroundColor, range: whole)
        // HTML import ends with a paragraph break the text does not need, and
        // empty leading blocks (`<div></div>`, `<p><br></p>`) would pad the
        // card's top.
        while let last = imported.string.unicodeScalars.last, CharacterSet.whitespacesAndNewlines.contains(last) {
            imported.deleteCharacters(in: NSRange(location: imported.length - 1, length: 1))
        }
        while let first = imported.string.unicodeScalars.first, CharacterSet.whitespacesAndNewlines.contains(first) {
            imported.deleteCharacters(in: NSRange(location: 0, length: 1))
        }
        return (try? AttributedString(imported, including: \.uiKit)) ?? AttributedString(imported.string)
    }
}

/// Rendered LMS text by its HTML. `NSCache` drops entries under memory
/// pressure; a dropped one renders again when next shown.
@MainActor
enum HTMLCache {
    private final class Entry {
        let text: AttributedString
        init(_ text: AttributedString) { self.text = text }
    }

    private static let entries: NSCache<NSString, Entry> = {
        let cache = NSCache<NSString, Entry>()
        cache.countLimit = 300
        return cache
    }()

    static func text(_ html: String) -> AttributedString {
        if let hit = entries.object(forKey: html as NSString) { return hit.text }
        let text = HTMLText.render(html)
        entries.setObject(Entry(text), forKey: html as NSString)
        return text
    }

    /// Renders ahead of display, outside any view update.
    static func warm(_ htmls: some Sequence<String>) {
        for html in htmls where !html.isEmpty { _ = text(html) }
    }
}
