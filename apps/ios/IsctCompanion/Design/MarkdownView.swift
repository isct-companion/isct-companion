import SwiftUI
import UIKit
import WebKit

/// A Markdown file read whole and shown in `MarkdownView`; while it loads, a
/// spinner, and if it cannot be read as text, `fallback`.
struct MarkdownFileView<Fallback: View>: View {
    let path: String
    @ViewBuilder let fallback: () -> Fallback
    @State private var text: String?
    @State private var failed = false

    var body: some View {
        Group {
            if let text {
                MarkdownView(text: text)
            } else if failed {
                fallback()
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: path) {
            let url = URL(fileURLWithPath: path)
            let loaded = await Task.detached { try? String(contentsOf: url, encoding: .utf8) }.value
            text = loaded
            failed = loaded == nil
        }
    }
}

/// A Markdown document, read-only, with its math set by KaTeX: the page in
/// `resources/markdown-viewer` (built from the desktop's reader rules), which
/// the app bundles. It loads nothing over the network but images the document
/// links; a link opens in the browser. Raw HTML in the document is shown as
/// text, never run.
struct MarkdownView: UIViewRepresentable {
    let text: String
    @Environment(\.self) private var environment

    func makeCoordinator() -> MarkdownPage { MarkdownPage() }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        view.isOpaque = false
        view.backgroundColor = .clear
        view.scrollView.backgroundColor = .clear
        if let page = Bundle.main.url(forResource: "markdown-viewer", withExtension: "html") {
            view.loadFileURL(page, allowingReadAccessTo: page.deletingLastPathComponent())
        }
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        context.coordinator.show(script, in: view)
    }

    /// `isctMarkdown.show(text, look)`, with the theme's colours and the
    /// reader's text size.
    private var script: String {
        let look = MarkdownLook(
            background: css(Theme.bg), text: css(Theme.ink), muted: css(Theme.muted),
            line: css(Theme.line), accent: css(Theme.accent), surface: css(Theme.surface2),
            fontSize: Double(UIFont.preferredFont(forTextStyle: .body).pointSize),
            dark: environment.colorScheme == .dark)
        let encoder = JSONEncoder()
        guard let textJSON = try? encoder.encode(text), let lookJSON = try? encoder.encode(look) else { return "" }
        return "isctMarkdown.show(\(String(decoding: textJSON, as: UTF8.self)), \(String(decoding: lookJSON, as: UTF8.self)))"
    }

    private func css(_ color: Color) -> String {
        let resolved = color.resolve(in: environment)
        let channel = { (value: Float) in Int((min(max(value, 0), 1) * 255).rounded()) }
        return String(format: "#%02X%02X%02X", channel(resolved.red), channel(resolved.green), channel(resolved.blue))
    }
}

private struct MarkdownLook: Encodable {
    let background, text, muted, line, accent, surface: String
    let fontSize: Double
    let dark: Bool
}

/// The page's state: the latest call, and whether the page can take it yet.
@MainActor
final class MarkdownPage: NSObject, WKNavigationDelegate {
    private var script = ""
    private var shown = ""
    private var loaded = false

    func show(_ script: String, in view: WKWebView) {
        self.script = script
        guard loaded, shown != script else { return }
        shown = script
        view.evaluateJavaScript(script)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loaded = true
        shown = script
        webView.evaluateJavaScript(script)
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        // The bundled page and jumps within it load; a link out opens in the
        // browser, and nothing else replaces the page (a relative link would
        // otherwise open another file of the app's bundle in its place).
        guard let url = action.request.url else { return decisionHandler(.cancel) }
        if url.isFileURL, url.lastPathComponent == "markdown-viewer.html" { return decisionHandler(.allow) }
        if action.navigationType == .linkActivated, ["http", "https", "mailto"].contains(url.scheme?.lowercased() ?? "") {
            UIApplication.shared.open(url)
        }
        decisionHandler(.cancel)
    }
}
