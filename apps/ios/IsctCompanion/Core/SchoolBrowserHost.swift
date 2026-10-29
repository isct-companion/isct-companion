import Foundation
import IsctCore
import OSLog
import UIKit
import WebKit

/// `NativeBrowser` over one resident WKWebView per school site and the popups
/// its pages open (docs/core-school-browser.md). Core decides every
/// navigation, window, sign-in and extraction; this adapter reports what
/// WebKit knows, carries out what Core decides, creates, shows and hides the
/// views, and keeps no rules of its own. Port methods arrive on Core workers
/// and only enqueue main-thread work.
///
/// - Navigation: `decidePolicyFor` reports URL, method, frame and navigation
///   type (`.formResubmitted` with whether it travels through history) and
///   applies Allow / External / Deny / SkipEntry.
/// - Popups: `createWebViewWith` returns a web view built from WebKit's own
///   configuration, so a posted form and `window.opener` survive. It covers
///   its site's view inside the web screen until the page closes it
///   (`webViewDidClose`) or the person does, as a sibling of the view in
///   whatever holds it, never a subview of a web view: WebKit answers a drag
///   (iPad drag and drop) at the outermost web view under it. The host moves
///   and sizes the popups with their view (`arrangePopups`).
/// - History: whether each view's and popup's own history can go back or
///   forward goes to Core when a page commits or finishes, when the list
///   changes in place, and when a popup is created. Core decides what Back
///   and Forward do, closing a popup at the start of its history included.
/// - Page dialogs and `window.print()` appear only for a page on screen; a
///   hidden view gets Cancel and prints nothing.
/// - Load failures, cancellations included, go to Core, which decides what
///   the web screen shows (`school-web-page`).
final class SchoolBrowserHost: NSObject, NativeBrowser, @unchecked Sendable {
    fileprivate static let log = Logger(subsystem: "app.isctcompanion", category: "school-web")
    private let lock = NSLock()
    private var events: BrowserEvents?
    /// Last committed URL per view and popup, readable from Core workers.
    private var urls: [BrowserId: String] = [:]
    private var closedForGood = false

    // Main-thread state.
    @MainActor private var views: [SchoolSite: SchoolPage] = [:]
    /// Open popups in opening order, so every popup follows its opener.
    @MainActor private var popups: [SchoolPage] = []
    /// Disposed views and popups whose files are still being written; each
    /// leaves once its last transfer ends.
    @MainActor private var finishing: Set<SchoolPage> = []
    @MainActor private lazy var parking: UIView = {
        let view = UIView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        view.isUserInteractionEnabled = false
        view.alpha = 0.01
        return view
    }()
    @MainActor var onOpenExternal: ((URL) -> Void)?
    @MainActor var onDownload: ((URL) -> Void)?
    @MainActor var onDownloadError: ((Error) -> Void)?

    // MARK: NativeBrowser (Core worker threads)

    func capabilities() -> HostCapabilities {
        HostCapabilities(formResubmission: true, inlinePopups: true)
    }

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
            for page in self.popups.reversed() + Array(self.views.values) + Array(self.finishing) {
                page.cancelDownloads()
                page.tearDown()
            }
            self.popups.removeAll()
            self.views.removeAll()
            self.finishing.removeAll()
        }
    }

    private func admit() throws {
        if lock.withLock({ closedForGood }) { throw SdkError.Closed }
    }

    private func listener() -> BrowserEvents? {
        lock.withLock { events }
    }

    // MARK: Presentation (main thread)

    /// The resident view for `site`, if Core created one. Its popups are
    /// siblings the host keeps over it wherever it is held (`arrangePopups`).
    @MainActor func webView(for site: SchoolSite) -> WKWebView? {
        views[site]?.webView
    }

    /// Puts `site`'s popups over its view again: in the view's superview,
    /// above the view in opening order, with its frame and resizing. A
    /// container that takes or parks the view calls this right after.
    @MainActor func arrangePopups(_ site: SchoolSite) {
        guard let view = views[site]?.webView, let container = view.superview else { return }
        var below: UIView = view
        for popup in popups where popup.id.site == site {
            popup.webView.frame = view.frame
            popup.webView.autoresizingMask = view.autoresizingMask
            container.insertSubview(popup.webView, aboveSubview: below)
            below = popup.webView
        }
    }

    /// The page the person sees for `site`: its newest popup, else its view.
    @MainActor private func frontWebView(for site: SchoolSite) -> WKWebView? {
        (popups.last { $0.id.site == site } ?? views[site])?.webView
    }

    /// Closes the newest popup over `site`'s view, and any it opened, as the
    /// web screen's close bar asks.
    @MainActor func closeFrontPopup(_ site: SchoolSite) {
        guard let front = popups.last(where: { $0.id.site == site }) else { return }
        closePopup(front.id)
    }

    /// Loads `url` in the page the person sees for `site` (a notice's Retry).
    /// The navigation goes through Core's decision like any other.
    @MainActor func retry(_ site: SchoolSite, url: URL) {
        frontWebView(for: site)?.load(URLRequest(url: url))
    }

    /// Returns a view to the off-screen parking host so WebKit keeps running
    /// it, its popups with it. A popup itself only moves with its view.
    @MainActor func park(_ webView: WKWebView) {
        guard !popups.contains(where: { $0.webView === webView }), let window = Self.keyWindow() else { return }
        if parking.superview !== window { window.insertSubview(parking, at: 0) }
        webView.frame = parking.bounds
        parking.addSubview(webView)
        if let site = views.first(where: { $0.value.webView === webView })?.key { arrangePopups(site) }
    }

    @MainActor private static func keyWindow() -> UIWindow? {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first
    }

    /// The controller to present a page's dialog or print panel from, when
    /// the page is on screen: in a window, not parked, the app in front.
    @MainActor fileprivate func presenter(for webView: WKWebView) -> UIViewController? {
        guard UIApplication.shared.applicationState == .active, let window = webView.window,
            !webView.isDescendant(of: parking), var top = window.rootViewController
        else { return nil }
        while let presented = top.presentedViewController, !presented.isBeingDismissed { top = presented }
        return top.isBeingDismissed ? nil : top
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

    /// Applies and keeps a text scale on every resident page and popup.
    @MainActor func setTextScale(_ percent: Int) {
        textScale = percent
        UserDefaults.standard.set(percent, forKey: Self.textScaleKey)
        for page in Array(views.values) + popups {
            page.webView.evaluateJavaScript(Self.textScaleScript(textScale))
        }
    }

    @MainActor private func create(_ id: BrowserId, _ initialUrl: String) {
        if views[id.site]?.id == id { return }
        if let previous = views[id.site] { discard(previous.id) }
        let configuration = WKWebViewConfiguration()
        // One persistent store: visible pages and hidden pulls share a session.
        configuration.websiteDataStore = .default()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = true
        // A script the app evaluates counts as a user action on iOS, so the
        // login script focusing the field it fills raised the keyboard, even
        // for a hidden pull. Programmatic focus is dropped; a tap still focuses.
        configuration.userContentController.addUserScript(
            WKUserScript(source: Self.noScriptedFocus, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        // Larger text without zooming the layout, so fixed-width pages still fit.
        configuration.userContentController.addUserScript(
            WKUserScript(source: Self.textScaleScript(textScale), injectionTime: .atDocumentEnd, forMainFrameOnly: false))
        let page = SchoolPage(id: id, parent: nil, webView: WKWebView(frame: .zero, configuration: configuration), host: self)
        views[id.site] = page
        park(page.webView)
        guard let url = URL(string: initialUrl) else {
            discard(id)
            try? listener()?.closed(browser: id)
            return
        }
        lock.withLock { urls[id] = initialUrl }
        page.webView.load(URLRequest(url: url))
    }

    /// The open view or popup `id` names.
    @MainActor private func page(_ id: BrowserId) -> SchoolPage? {
        if let view = views[id.site], view.id == id { return view }
        return popups.first { $0.id == id }
    }

    @MainActor private func perform(_ command: BrowserCommand) {
        guard let page = page(command.browser) else {
            // A command for a page that no longer exists closes that generation.
            try? listener()?.closed(browser: command.browser)
            return
        }
        let webView = page.webView
        switch command.action {
        case let .navigate(raw):
            guard let url = URL(string: raw) else {
                fail(page, cause: command.actionId, detail: "Invalid URL")
                return
            }
            webView.load(URLRequest(url: url))
        case .reload:
            // A page that would resend a form asks again as `.formResubmitted`.
            webView.reload()
        case .goBack:
            webView.goBack()
        case .goForward:
            webView.goForward()
        case let .evaluate(script):
            webView.evaluateJavaScript(script) { [weak self] _, error in
                // Scripts report through title and navigation relays; only a
                // script that could not run fails the page.
                guard let self, let error = error as? WKError, error.code != .javaScriptResultTypeIsUnsupported
                else { return }
                self.fail(page, cause: command.actionId, detail: error.localizedDescription)
            }
        case let .stop(script):
            // Core abandoned this view's attempt and leases it again only
            // once this confirms; the page reported at that point is the last
            // one the attempt may claim.
            webView.stopLoading()
            webView.evaluateJavaScript(script) { [weak self] _, _ in
                try? self?.listener()?.stopped(page: page.page, actionId: command.actionId)
            }
        }
    }

    @MainActor private func fail(_ page: SchoolPage, cause: UInt64?, detail: String) {
        try? listener()?.loadFailed(page: page.nextPage(), cause: cause, url: nil, reason: .other, detail: detail)
    }

    /// Takes a view, with its popups, or a popup, with the ones it opened,
    /// off the screen and out of the host.
    @MainActor private func discard(_ id: BrowserId) {
        if let view = views[id.site], view.id == id {
            for popup in popups.reversed() where popup.id.site == id.site { forget(popup) }
            popups.removeAll { $0.id.site == id.site }
            forget(view)
            views.removeValue(forKey: id.site)
        } else {
            var closing: Set<BrowserId> = [id]
            for popup in popups where popup.parent.map(closing.contains) == true { closing.insert(popup.id) }
            for popup in popups.reversed() where closing.contains(popup.id) { forget(popup) }
            popups.removeAll { closing.contains($0.id) }
        }
    }

    @MainActor private func forget(_ page: SchoolPage) {
        page.tearDown()
        if page.downloading { finishing.insert(page) }
        lock.withLock { _ = urls.removeValue(forKey: page.id) }
    }

    /// A popup its page (`window.close()`) or the person closed. Core then
    /// forgets the popups it opened and asks for them to be disposed.
    @MainActor private func closePopup(_ id: BrowserId) {
        guard popups.contains(where: { $0.id == id }) else { return }
        discard(id)
        try? listener()?.closed(browser: id)
    }

    // MARK: Callbacks from a page

    @MainActor fileprivate func loaded(_ page: SchoolPage, url: URL?) {
        guard let url else { return }
        lock.withLock { urls[page.id] = url.absoluteString }
        // WebKit exposes no request id here; never label a late load with an action.
        try? listener()?.pageLoaded(page: page.nextPage(), cause: nil, url: url.absoluteString)
    }

    @MainActor fileprivate func failed(_ page: SchoolPage, error: Error) {
        let error = error as NSError
        let url = (error.userInfo[NSURLErrorFailingURLStringErrorKey] as? String)
            ?? (error.userInfo[NSURLErrorFailingURLErrorKey] as? URL)?.absoluteString
        // WebKit shows nothing for a failed load and stays on the old page;
        // Core decides whether the web screen says so.
        try? listener()?.loadFailed(
            page: page.nextPage(), cause: nil, url: url, reason: Self.failure(error),
            detail: "\(error.domain) \(error.code): \(error.localizedDescription)")
    }

    /// A coarse reason for a failed load. Cancellations are a newer load, a
    /// policy decision, or a response that became a download.
    private static func failure(_ error: NSError) -> LoadFailure {
        switch (error.domain, error.code) {
        case (NSURLErrorDomain, NSURLErrorCancelled): .cancelled
        // Frame load interrupted by policy change; plug-in handled load.
        case (WKError.errorDomain, 102), (WKError.errorDomain, 204): .cancelled
        case (NSURLErrorDomain, NSURLErrorNotConnectedToInternet), (NSURLErrorDomain, NSURLErrorDataNotAllowed),
            (NSURLErrorDomain, NSURLErrorInternationalRoamingOff):
            .offline
        case (NSURLErrorDomain, NSURLErrorCannotFindHost), (NSURLErrorDomain, NSURLErrorDNSLookupFailed): .hostNotFound
        case (NSURLErrorDomain, NSURLErrorTimedOut), (NSURLErrorDomain, NSURLErrorCannotConnectToHost),
            (NSURLErrorDomain, NSURLErrorNetworkConnectionLost):
            .unreachable
        // App Transport Security refused plain HTTP.
        case (NSURLErrorDomain, NSURLErrorAppTransportSecurityRequiresSecureConnection): .cleartext
        case (NSURLErrorDomain, NSURLErrorClientCertificateRequired...NSURLErrorSecureConnectionFailed): .insecure
        default: .other
        }
    }

    /// A navigation of `page` became a download, saved to a file. Core
    /// disposes a popup that had nothing else to show; its transfer goes on.
    @MainActor fileprivate func becameDownload(_ page: SchoolPage, hadPage: Bool) {
        try? listener()?.becameDownload(page: page.page, hadPage: hadPage, disposition: .saved)
    }

    /// A transfer of `page` ended; a disposed page goes once it has none left.
    @MainActor fileprivate func transferEnded(_ page: SchoolPage) {
        if !page.downloading { finishing.remove(page) }
    }

    @MainActor fileprivate func titled(_ page: SchoolPage, _ title: String) {
        try? listener()?.titleChanged(page: page.page, title: title)
    }

    /// Where `page`'s own history can go from the page it shows, for Core's
    /// Back and Forward.
    @MainActor fileprivate func historyChanged(_ page: SchoolPage) {
        try? listener()?.historyChanged(
            browser: page.id, canGoBack: page.webView.canGoBack, canGoForward: page.webView.canGoForward)
    }

    /// The facts of a navigation action. A new window's action has no target
    /// frame yet; it loads in a main frame.
    @MainActor private func facts(_ page: SchoolPage, _ action: WKNavigationAction) -> NavigationRequest? {
        guard let url = action.request.url?.absoluteString else { return nil }
        let mainFrame = action.targetFrame?.isMainFrame ?? true
        let kind: NavigationKind
        switch action.navigationType {
        case .linkActivated: kind = .link
        case .formSubmitted: kind = .formSubmit
        case .backForward: kind = .backForward
        case .reload: kind = .reload
        case .formResubmitted:
            // WebKit uses this for a reload and for history alike. It has
            // already moved its list to the entry it is going to when it
            // asks, so an entry other than the committed page's is history.
            let fromHistory = mainFrame && page.webView.backForwardList.currentItem !== page.committed
            Self.log.info(
                "Form resubmission in \(String(describing: page.id.site), privacy: .public), from history: \(fromHistory, privacy: .public)"
            )
            kind = .formResubmit(fromHistory: fromHistory)
        case .other: kind = .other
        @unknown default: kind = .unknown
        }
        // WebKit says whether the person acted only through private API,
        // which an App Store build does not call; Core counts links and forms
        // as the person's.
        return NavigationRequest(
            page: page.page, url: url, method: action.request.httpMethod, mainFrame: mainFrame, kind: kind,
            userGesture: false)
    }

    @MainActor fileprivate func decide(_ page: SchoolPage, _ action: WKNavigationAction) -> WKNavigationActionPolicy {
        guard let request = facts(page, action), let listener = listener(),
            let decision = try? listener.navigation(request: request)
        else { return .cancel }
        switch decision {
        case .allow:
            return action.shouldPerformDownload ? .download : .allow
        case let .external(target):
            if let target = URL(string: target) { onOpenExternal?(target) }
            return .cancel
        case .deny:
            return .cancel
        case .skipEntry:
            // The entry WebKit was going to; once the cancel has put the list
            // back, step past it.
            if let target = page.webView.backForwardList.currentItem {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak page] in page?.skip(past: target) }
            }
            return .cancel
        }
    }

    /// The popup WebKit asks for, if Core gives it one, built from WebKit's
    /// configuration and laid over its opener.
    @MainActor fileprivate func window(
        _ opener: SchoolPage, configuration: WKWebViewConfiguration, action: WKNavigationAction,
        features: WKWindowFeatures
    ) -> WKWebView? {
        guard let facts = facts(opener, action), let listener = listener() else { return nil }
        let request = NewWindowRequest(
            opener: opener.page, url: facts.url, method: facts.method, kind: facts.kind,
            userGesture: facts.userGesture, name: nil, width: features.width?.doubleValue,
            height: features.height?.doubleValue)
        guard let decision = try? listener.newWindow(request: request) else { return nil }
        switch decision {
        case let .popup(browser):
            let popup = SchoolPage(
                id: browser, parent: opener.id,
                webView: WKWebView(frame: opener.webView.frame, configuration: configuration), host: self)
            popups.append(popup)
            // Over the view and the site's earlier popups, as their sibling.
            arrangePopups(browser.site)
            lock.withLock { urls[browser] = facts.url }
            // No history yet: Core's Back closes it until it has some.
            historyChanged(popup)
            return popup.webView
        case let .external(target):
            if let target = URL(string: target) { onOpenExternal?(target) }
            return nil
        case .deny:
            return nil
        }
    }

    @MainActor fileprivate func closedByPage(_ page: SchoolPage) {
        // A view is not one a script opened, so only popups close this way.
        if page.parent != nil { closePopup(page.id) }
    }

    /// `window.print()`: the system print panel for a page on screen.
    @MainActor fileprivate func printPage(_ page: SchoolPage) {
        guard let presenter = presenter(for: page.webView) else { return }
        let controller = UIPrintInteractionController.shared
        let info = UIPrintInfo.printInfo()
        info.outputType = .general
        info.jobName = page.webView.title ?? ""
        controller.printInfo = info
        controller.printFormatter = page.webView.viewPrintFormatter()
        if presenter.traitCollection.userInterfaceIdiom == .pad {
            let view = page.webView
            controller.present(from: CGRect(x: view.bounds.midX, y: view.bounds.minY, width: 1, height: 1), in: view, animated: true)
        } else {
            controller.present(animated: true)
        }
    }
}

/// A page dialog's answer, given to WebKit exactly once: a string is OK (the
/// entered text for `prompt()`), nil is Cancel. WebKit raises if a handler
/// is dropped unanswered, so a dialog that ends without an answer (never
/// shown, or torn down with its page) answers Cancel when it goes.
private final class Reply {
    private var send: ((String?) -> Void)?

    init(_ send: @escaping (String?) -> Void) { self.send = send }

    func answer(_ value: String?) {
        send?(value)
        send = nil
    }

    deinit {
        guard let send else { return }
        DispatchQueue.main.async { send(nil) }
    }
}

/// One school view or popup: its WKWebView, load sequence and delegates.
@MainActor
private final class SchoolPage: NSObject, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    let id: BrowserId
    /// The view or popup that opened this popup; nil for a site's view.
    let parent: BrowserId?
    let webView: WKWebView
    private(set) var sequence: UInt64 = 0
    /// The history entry of the last committed page. A form resubmission
    /// whose entry differs travels through history instead of reloading.
    private(set) var committed: WKBackForwardListItem?
    private weak var host: SchoolBrowserHost?
    private var titleObservation: NSKeyValueObservation?
    /// A same-document navigation (`pushState`) changes the history with no
    /// commit or finish.
    private var historyObservations: [NSKeyValueObservation] = []
    /// WebKit holds its delegate weakly. The page owns each native transfer
    /// until completion or disposal, including its temporary file.
    private var downloads: [ObjectIdentifier: (download: WKDownload, file: URL?)] = [:]

    init(id: BrowserId, parent: BrowserId?, webView: WKWebView, host: SchoolBrowserHost) {
        self.id = id
        self.parent = parent
        self.webView = webView
        self.host = host
        super.init()
        webView.allowsBackForwardNavigationGestures = true
        webView.navigationDelegate = self
        webView.uiDelegate = self
        titleObservation = webView.observe(\.title, options: [.new]) { [weak self] webView, _ in
            let title = webView.title ?? ""
            Task { @MainActor in
                guard let self, !title.isEmpty else { return }
                self.host?.titled(self, title)
            }
        }
        historyObservations = [\WKWebView.canGoBack, \WKWebView.canGoForward].map { path in
            webView.observe(path, options: [.new]) { [weak self] _, _ in
                Task { @MainActor in
                    guard let self else { return }
                    self.host?.historyChanged(self)
                }
            }
        }
    }

    var page: PageId { PageId(browser: id, sequence: sequence) }

    func nextPage() -> PageId {
        sequence += 1
        return page
    }

    var downloading: Bool { !downloads.isEmpty }

    /// Stops the page and takes it off the screen. Its transfers go on until
    /// their files are written.
    func tearDown() {
        webView.stopLoading()
        webView.removeFromSuperview()
    }

    /// Moves one entry beyond `target` in the direction it lies from the
    /// current entry, so the form it would resend is never sent.
    func skip(past target: WKBackForwardListItem) {
        let list = webView.backForwardList
        let offset: Int? =
            if let index = list.backList.firstIndex(where: { $0 === target }) {
                index - list.backList.count - 1
            } else if let index = list.forwardList.firstIndex(where: { $0 === target }) {
                index + 2
            } else {
                nil
            }
        guard let offset else {
            SchoolBrowserHost.log.info("Skipped resubmission: the history had not moved; staying on the page")
            return
        }
        if let item = list.item(at: offset) { webView.go(to: item) }
    }

    // MARK: Navigation

    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        guard let host else { return decisionHandler(.cancel) }
        decisionHandler(host.decide(self, action))
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor response: WKNavigationResponse,
        decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void
    ) {
        let disposition = (response.response as? HTTPURLResponse)?
            .value(forHTTPHeaderField: "Content-Disposition")?
            .split(separator: ";").first?.trimmingCharacters(in: .whitespaces).lowercased()
        // Displayable Moodle attachments otherwise commit an empty page.
        decisionHandler(disposition == "attachment" || !response.canShowMIMEType ? .download : .allow)
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        committed = webView.backForwardList.currentItem
        host?.historyChanged(self)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        host?.historyChanged(self)
        host?.loaded(self, url: webView.url)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        host?.failed(self, error: error)
    }

    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
    ) {
        host?.failed(self, error: error)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        webView.reload()
    }

    // MARK: Windows, dialogs, printing

    func webView(
        _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction, windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        host?.window(self, configuration: configuration, action: action, features: windowFeatures)
    }

    func webViewDidClose(_ webView: WKWebView) {
        host?.closedByPage(self)
    }

    func webView(
        _ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor () -> Void
    ) {
        present(message, frame: frame, cancel: false, field: nil, reply: Reply { _ in completionHandler() })
    }

    func webView(
        _ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor (Bool) -> Void
    ) {
        present(message, frame: frame, cancel: true, field: nil, reply: Reply { completionHandler($0 != nil) })
    }

    func webView(
        _ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?,
        initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor (String?) -> Void
    ) {
        present(prompt, frame: frame, cancel: true, field: defaultText ?? "", reply: Reply { completionHandler($0) })
    }

    /// Shows a page dialog, titled with the initiating frame's origin as in
    /// Safari so an embedded page cannot pass its text off as the school's.
    /// A page that is not on screen raises no UI: dropping `reply` answers
    /// Cancel, as WebKit does without a delegate.
    private func present(_ message: String, frame: WKFrameInfo, cancel: Bool, field: String?, reply: Reply) {
        guard let presenter = host?.presenter(for: webView) else { return }
        let origin = frame.securityOrigin
        let title: String? =
            origin.host.isEmpty ? nil : origin.port == 0
            ? "\(origin.protocol)://\(origin.host)" : "\(origin.protocol)://\(origin.host):\(origin.port)"
        let alert = UIAlertController(title: title ?? message, message: title == nil ? nil : message, preferredStyle: .alert)
        if let field { alert.addTextField { $0.text = field } }
        alert.addAction(UIAlertAction(title: t("common.ok"), style: .default) { [weak alert] _ in
            reply.answer(alert?.textFields?.first?.text ?? "")
        })
        if cancel {
            alert.addAction(UIAlertAction(title: t("common.cancel"), style: .cancel) { _ in reply.answer(nil) })
        }
        presenter.present(alert, animated: true)
    }

    /// `window.print()` reaches WebKit's private
    /// `_webView:printFrame:pdfFirstPageSize:completionHandler:`; without it
    /// nothing happens. The page goes on at once and the panel follows, so
    /// the print formatter never waits on a page held in `print()`.
    @objc(_webView:printFrame:pdfFirstPageSize:completionHandler:)
    func printFrame(_ webView: WKWebView, frame: AnyObject?, pdfFirstPageSize: CGSize, completionHandler: @escaping () -> Void) {
        completionHandler()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.host?.printPage(self)
        }
    }

    // MARK: Downloads

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        receive(download)
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        receive(download)
    }

    /// Reported for this page, the one whose navigation it was, never
    /// `download.webView`: for a popup with nothing committed WebKit names
    /// the opener there.
    private func receive(_ download: WKDownload) {
        downloads[ObjectIdentifier(download)] = (download, nil)
        download.delegate = self
        host?.becameDownload(self, hadPage: committed != nil)
    }

    func download(
        _ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String,
        completionHandler: @escaping @MainActor (URL?) -> Void
    ) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("isct-web-\(UUID().uuidString)", isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let name = (suggestedFilename as NSString).lastPathComponent
            let file = directory.appendingPathComponent(name.isEmpty || name == "." || name == ".." ? "download" : name)
            downloads[ObjectIdentifier(download)] = (download, file)
            completionHandler(file)
        } catch {
            try? FileManager.default.removeItem(at: directory)
            completionHandler(nil)
            host?.onDownloadError?(error)
        }
    }

    func downloadDidFinish(_ download: WKDownload) {
        let file = downloads.removeValue(forKey: ObjectIdentifier(download))?.file
        host?.transferEnded(self)
        if let file { host?.onDownload?(file) }
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        if let file = downloads.removeValue(forKey: ObjectIdentifier(download))?.file {
            try? FileManager.default.removeItem(at: file.deletingLastPathComponent())
        }
        host?.transferEnded(self)
        host?.onDownloadError?(error)
    }

    func cancelDownloads() {
        for item in downloads.values {
            item.download.delegate = nil
            item.download.cancel { _ in
                if let file = item.file { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
            }
        }
        downloads.removeAll()
    }
}
