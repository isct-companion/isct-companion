import SwiftUI
import UIKit

/// LMS text (Core's sanitized `bodyHtml`) as native text: paragraphs, lists,
/// emphasis and links keep their shape; colours follow the app, so dark mode
/// and Dynamic Type apply. Links open in the in-app school page.
struct HTMLText: View {
    let html: String
    @Environment(Navigator.self) private var navigator
    @State private var text: AttributedString?

    var body: some View {
        Group {
            if let text {
                Text(text)
                    .font(.body)
                    .foregroundStyle(Theme.ink)
                    .tint(Theme.accent)
                    .textSelection(.enabled)
            } else {
                ProgressView().frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .environment(\.openURL, OpenURLAction { url in
            guard url.host == "lms.s.isct.ac.jp" else { return .systemAction }
            navigator.push(WebRouter.PushedPage(site: .scienceLms, target: url.absoluteString, title: ""))
            return .handled
        })
        .task(id: html) { text = Self.render(html) }
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
        // HTML import ends with a paragraph break the text does not need.
        while imported.string.hasSuffix("\n") {
            imported.deleteCharacters(in: NSRange(location: imported.length - 1, length: 1))
        }
        return (try? AttributedString(imported, including: \.uiKit)) ?? AttributedString(imported.string)
    }
}
