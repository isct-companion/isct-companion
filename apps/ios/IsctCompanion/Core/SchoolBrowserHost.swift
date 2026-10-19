import Foundation
import IsctCore
import OSLog
import UIKit
import WebKit

/// `NativeBrowser` over one resident WKWebView per school site. Core drives
/// sign-in, navigation, extraction and history; this adapter creates, shows
/// and hides the views, executes dispatched commands on the main thread and
/// forwards WebKit callbacks. Port methods arrive on Core workers and only
/// enqueue main-thread work.
final class SchoolBrowserHost: NSObject, NativeBrowser, @unchecked Sendable {
    fileprivate static let log = Logger(subsystem: "app.isctcompanion", category: "school-web")
    private let lock = NSLock()
    private var events: BrowserEvents?
    /// Last committed URL per view, readable from Core workers.
    private var urls: [BrowserId: String] = [:]
    private var closedForGood = false

    // Main-thread state.
    @MainActor private var views: [SchoolSite: ResidentView] = [:]
    @MainActor private lazy var parking: UIView = {
        let view = UIView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        view.isUserInteractionEnabled = false
        view.alpha = 0.01
        return view
    }()
    @MainActor var onTitleChange: ((SchoolSite, String) -> Void)?
    @MainActor var onOpenExternal: ((URL) -> Void)?

    // MARK: NativeBrowser (Core worker threads)

    func attach(events: BrowserEvents) throws {
        lock.withLock { self.events = events }
    }

    func ensure(browser: BrowserId, initialUrl: String) throws {
        try admit()
        DispatchQueue.main.async { self.create(browser, initialUrl) }
    }

    func dispatch(command: BrowserCommand) throws {
        try admit()
        DispatchQueue.main.async { self.perform(command) }
    }

    func currentUrl(browser: BrowserId) throws -> String? {
        lock.withLock { urls[browser] }
    }

    func dispose(browser: BrowserId) throws {
        DispatchQueue.main.async {
            self.discard(browser)
            try? self.listener()?.closed(browser: browser)
        }
    }

    func shutdown() throws {
        lock.withLock {
            closedForGood = true
            urls.removeAll()
        }
        DispatchQueue.main.async {
            for view in self.views.values { view.webView.removeFromSuperview() }
            self.views.removeAll()
        }
    }

    private func admit() throws {
        if lock.withLock({ closedForGood }) { throw SdkError.Closed }
    }

    private func listener() -> BrowserEvents? {
        lock.withLock { events }
    }

    // MARK: Presentation (main thread)

    /// The resident view for `site`, if Core created one.
    @MainActor func webView(for site: SchoolSite) -> WKWebView? {
        views[site]?.webView
    }

    /// Returns a view to the off-screen parking host so WebKit keeps running it.
    @MainActor func park(_ webView: WKWebView) {
        guard let window = Self.keyWindow() else { return }
        if parking.superview !== window { window.insertSubview(parking, at: 0) }
        webView.frame = parking.bounds
        parking.addSubview(webView)
    }

    @MainActor private static func keyWindow() -> UIWindow? {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first
    }

    /// The text scales offered for school pages, in percent.
    static let textScales = [100, 115, 130]
    /// Where the chosen scale is kept on this device (設定 › ウェブ).
    static let textScaleKey = "web.textScale"

    /// Text scale for school pages, in percent; starts above 100 for readability.
    @MainActor private(set) var textScale: Int = {
        let stored = UserDefaults.standard.integer(forKey: textScaleKey)
        return textScales.contains(stored) ? stored : 115
    }()

    private static let noScriptedFocus = "HTMLElement.prototype.focus = function () {};"

    private static func textScaleScript(_ percent: Int) -> String {
        "document.documentElement.style.webkitTextSizeAdjust = '\(percent)%';"
    }

    /// Applies and keeps a text scale on every resident page.
    @MainActor func setTextScale(_ percent: Int) {
        textScale = percent
        UserDefaults.standard.set(percent, forKey: Self.textScaleKey)
        for view in views.values {
            view.webView.evaluateJavaScript(Self.textScaleScript(textScale))
        }
    }

    /// Cycles 100 → 115 → 130 % on every resident page.
    @MainActor func cycleTextScale() {
        setTextScale(textScale >= 130 ? 100 : textScale + 15)
    }

    @MainActor private func create(_ id: BrowserId, _ initialUrl: String) {
        if views[id.site]?.id == id { return }
        if let previous = views.removeValue(forKey: id.site) {
            previous.webView.removeFromSuperview()
        }
        let configuration = WKWebViewConfiguration()
        // One persistent store: visible pages and hidden pulls share a session.
        configuration.websiteDataStore = .default()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = true
        for script in (try? schoolBrowserInitialization()) ?? [] {
            configuration.userContentController.addUserScript(
                WKUserScript(source: script, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        }
        // A script the app evaluates counts as a user action on iOS, so the
        // login script focusing the field it fills raised the keyboard, even
        // for a hidden pull. Programmatic focus is dropped; a tap still focuses.
        configuration.userContentController.addUserScript(
            WKUserScript(source: Self.noScriptedFocus, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        // Larger text without zooming the layout, so fixed-width pages still fit.
        configuration.userContentController.addUserScript(
            WKUserScript(source: Self.textScaleScript(textScale), injectionTime: .atDocumentEnd, forMainFrameOnly: false))
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.allowsBackForwardNavigationGestures = true
        let resident = ResidentView(id: id, webView: webView, host: self)
        webView.navigationDelegate = resident
        webView.uiDelegate = resident
        resident.observeTitle()
        views[id.site] = resident
        park(webView)
        guard let url = URL(string: initialUrl) else {
            discard(id)
            try? listener()?.closed(browser: id)
            return
        }
        lock.withLock { urls[id] = initialUrl }
        webView.load(URLRequest(url: url))
    }

    @MainActor private func perform(_ command: BrowserCommand) {
        let id = command.browser
        guard let resident = views[id.site], resident.id == id else {
            // A command for a view that no longer exists closes that generation.
            try? listener()?.closed(browser: id)
            return
        }
        switch command.action {
        case let .navigate(raw):
            guard let url = URL(string: raw) else {
                fail(resident, cause: command.actionId, detail: "Invalid URL")
                return
            }
            resident.webView.load(URLRequest(url: url))
        case .reload:
            resident.webView.reload()
        case let .evaluate(script):
            resident.webView.evaluateJavaScript(script) { [weak self] _, error in
                // Scripts report through title and navigation relays; only a
                // script that could not run fails the page.
                guard let self, let error = error as? WKError, error.code != .javaScriptResultTypeIsUnsupported
                else { return }
                self.fail(resident, cause: command.actionId, detail: error.localizedDescription)
            }
        }
    }

    @MainActor private func fail(_ resident: ResidentView, cause: UInt64?, detail: String) {
        try? listener()?.loadFailed(page: resident.nextPage(), cause: cause, url: nil, detail: detail)
    }

    @MainActor private func discard(_ id: BrowserId) {
        guard let resident = views[id.site], resident.id == id else { return }
        resident.webView.removeFromSuperview()
        views.removeValue(forKey: id.site)
        lock.withLock { _ = urls.removeValue(forKey: id) }
    }

    // MARK: Callbacks from a resident view

    @MainActor fileprivate func loaded(_ resident: ResidentView, url: URL?) {
        guard let url else { return }
        lock.withLock { urls[resident.id] = url.absoluteString }
        // WebKit exposes no request id here; never label a late load with an action.
        try? listener()?.pageLoaded(page: resident.nextPage(), cause: nil, url: url.absoluteString)
    }

    @MainActor fileprivate func failed(_ resident: ResidentView, url: URL?, error: Error) {
        let error = error as NSError
        if error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled { return }
        // A navigation cancelled by Core's own decision is not a failure.
        if error.domain == WKError.errorDomain && error.code == 102 { return }
        try? listener()?.loadFailed(
            page: resident.nextPage(), cause: nil, url: url?.absoluteString, detail: error.localizedDescription)
    }

    @MainActor fileprivate func titled(_ resident: ResidentView, _ title: String) {
        try? listener()?.titleChanged(page: resident.page, title: title)
        onTitleChange?(resident.id.site, title)
    }

    @MainActor fileprivate func decide(_ resident: ResidentView, url: URL, mainFrame: Bool) -> Bool {
        guard let listener = listener(),
            let decision = try? listener.navigation(page: resident.page, url: url.absoluteString, mainFrame: mainFrame)
        else { return false }
        if decision != .allow { Self.log.debug("Core cancelled \(url.host() ?? "", privacy: .public) in \(String(describing: resident.id.site), privacy: .public)") }
        return decision == .allow
    }

    @MainActor fileprivate func popup(_ resident: ResidentView, url: URL) {
        guard let decision = try? listener()?.newWindow(page: resident.page, url: url.absoluteString) else { return }
        Self.log.debug("Popup to \(url.host() ?? "", privacy: .public): \(String(describing: decision), privacy: .public)")
        switch decision {
        case let .sameView(target):
            if let target = URL(string: target) { resident.webView.load(URLRequest(url: target)) }
        case .openSchool:
            // Core already emitted `school-web-open`; the router presents it.
            break
        case let .openExternal(target):
            if let target = URL(string: target) { onOpenExternal?(target) }
        case .ignore:
            break
        }
    }
}

/// One site's WKWebView and its load sequence.
@MainActor
private final class ResidentView: NSObject, WKNavigationDelegate, WKUIDelegate {
    let id: BrowserId
    let webView: WKWebView
    private(set) var sequence: UInt64 = 0
    private weak var host: SchoolBrowserHost?
    private var titleObservation: NSKeyValueObservation?

    init(id: BrowserId, webView: WKWebView, host: SchoolBrowserHost) {
        self.id = id
        self.webView = webView
        self.host = host
    }

    var page: PageId { PageId(browser: id, sequence: sequence) }

    func nextPage() -> PageId {
        sequence += 1
        return page
    }

    func observeTitle() {
        titleObservation = webView.observe(\.title, options: [.new]) { [weak self] webView, _ in
            let title = webView.title ?? ""
            Task { @MainActor in
                guard let self, !title.isEmpty else { return }
                self.host?.titled(self, title)
            }
        }
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = action.request.url, let host else { return decisionHandler(.cancel) }
        if ["about", "blob", "data"].contains(url.scheme ?? "") { return decisionHandler(.allow) }
        let mainFrame = action.targetFrame?.isMainFrame ?? true
        decisionHandler(host.decide(self, url: url, mainFrame: mainFrame) ? .allow : .cancel)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        host?.loaded(self, url: webView.url)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        host?.failed(self, url: webView.url, error: error)
    }

    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
    ) {
        host?.failed(self, url: webView.url, error: error)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        webView.reload()
    }

    func webView(
        _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction, windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if let url = action.request.url { host?.popup(self, url: url) }
        return nil
    }
}
