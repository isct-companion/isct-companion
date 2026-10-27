import IsctCore
import SwiftUI
import TipKit
import WebKit

/// Hosts a site's resident WKWebView while on screen and parks it afterwards,
/// so its page, scroll position and session survive tab and route changes.
/// The ウェブ tab and pushed pages share one view per site: whichever
/// container is in the window holds it, and a container that leaves the
/// window (a covered tab, a popped page) hands it back to the parking window,
/// where WebKit keeps it running for hidden pulls.
private struct ResidentWebView: UIViewRepresentable {
    let site: SchoolSite
    let host: SchoolBrowserHost
    /// Changes when Core created a view, so the container attaches it.
    let revision: Int
    /// Pull to refresh, as in Safari; Core reloads the page.
    var refresh: (() -> Void)?

    func makeUIView(context: Context) -> Container {
        let container = Container(host: host)
        container.backgroundColor = .systemBackground
        return container
    }

    func updateUIView(_ container: Container, context: Context) {
        container.site = site
        container.refresh = refresh
        container.claim()
    }

    static func dismantleUIView(_ container: Container, coordinator: ()) {
        container.release()
    }

    final class Container: UIView {
        let host: SchoolBrowserHost
        var site: SchoolSite?
        var refresh: (() -> Void)?
        /// One per container: the resident view carries it while held here.
        private lazy var refreshControl: UIRefreshControl = {
            let control = UIRefreshControl()
            control.addAction(UIAction { [weak self, weak control] _ in
                self?.refresh?()
                // The page shows its own progress; the spinner only answers the pull.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { control?.endRefreshing() }
            }, for: .valueChanged)
            return control
        }()

        init(host: SchoolBrowserHost) {
            self.host = host
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { nil }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            if window == nil { release() } else { claim() }
        }

        /// Shows this container's site, parking any other site it held.
        func claim() {
            guard window != nil, let site else { return }
            for case let held as WKWebView in subviews where held !== host.webView(for: site) {
                host.park(held)
            }
            guard let webView = host.webView(for: site), webView.superview !== self else { return }
            webView.frame = bounds
            webView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            if refresh != nil { webView.scrollView.refreshControl = refreshControl }
            addSubview(webView)
        }

        /// Returns whatever view it holds to the parking window.
        func release() {
            for case let held as WKWebView in subviews {
                if held.scrollView.refreshControl === refreshControl { held.scrollView.refreshControl = nil }
                host.park(held)
            }
        }
    }
}

/// ウェブ tab: one resident view per site under a standard navigation bar.
/// The title is the site and its menu switches sites (long-pressing the tab
/// does too); back and forward sit leading, page actions in the trailing
/// menu, and a pull reloads. Tapping the tab again returns the site home.
struct WebTabScreen: View {
    @Environment(WebRouter.self) private var web
    @State private var visibilityOwner = UUID()

    var body: some View {
        // Read so a navigation (which retitles the page) refreshes the actions.
        _ = web.titles[web.tabSite]
        return ZStack {
            ResidentWebView(site: web.tabSite, host: web.browser, revision: web.attachRevision) {
                web.reload(web.tabSite)
            }
            if web.loading == web.tabSite && web.browser.webView(for: web.tabSite) == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if let error = web.error {
                ProblemNote(text: error).padding(.top, 12).frame(maxHeight: .infinity, alignment: .top)
            }
            // The one thing drawn over the page, once: how to reach the
            // other sites.
            TipView(WebSitesTip())
                .padding(12)
                .frame(maxHeight: .infinity, alignment: .bottom)
        }
        .safeAreaInset(edge: .top, spacing: 0) { PageBars(site: web.tabSite) }
        .background(Theme.bg)
        .navigationTitle(WebRouter.name(web.tabSite))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarTitleMenu {
            ForEach(WebRouter.tabSites, id: \.self) { site in
                Button {
                    web.choose(site)
                } label: {
                    if site == web.tabSite {
                        Label(WebRouter.name(site), systemImage: "checkmark")
                    } else {
                        Text(WebRouter.name(site))
                    }
                }
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .topBarLeading) {
                Button { web.back(web.tabSite) } label: { Image(systemName: "chevron.backward") }
                    .accessibilityLabel(t("Previous page"))
                Button { web.forward(web.tabSite) } label: { Image(systemName: "chevron.forward") }
                    .accessibilityLabel(t("Next page"))
            }
            ToolbarItem(placement: .topBarTrailing) {
                PageActions(site: web.tabSite)
            }
        }
        .onAppear {
            web.showTab(web.tabSite)
            web.appear(web.tabSite, owner: visibilityOwner)
        }
        .onDisappear { web.disappear(owner: visibilityOwner) }
    }
}

/// The page menu both web screens share: reload, share, Safari and text size.
private struct PageActions: View {
    let site: SchoolSite
    @Environment(WebRouter.self) private var web
    @AppStorage(SchoolBrowserHost.textScaleKey) private var textScale = 115

    var body: some View {
        Menu {
            Button(t("Reload"), systemImage: "arrow.clockwise") { web.reload(site) }
            if let url = web.browser.frontWebView(for: site)?.url {
                ShareLink(item: url) { Label(t("Share"), systemImage: "square.and.arrow.up") }
                Link(destination: url) { Label(t("Open in browser"), systemImage: "safari") }
            }
            Picker(selection: $textScale) {
                ForEach(SchoolBrowserHost.textScales, id: \.self) { Text(verbatim: "\($0)%").tag($0) }
            } label: {
                Label(t("Text size"), systemImage: "textformat.size")
            }
            .pickerStyle(.menu)
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .onChange(of: textScale) { _, percent in web.browser.setTextScale(percent) }
    }
}

/// What the site picker offers: the ウェブ tab's sites, then page actions.
enum SitePick: Hashable {
    case site(SchoolSite)
    case reload
    case openInBrowser
}

extension WebRouter {
    /// Runs a picker choice on the ウェブ tab's page.
    func perform(_ pick: SitePick) {
        switch pick {
        case let .site(site): choose(site)
        case .reload: reload(tabSite)
        case .openInBrowser:
            if let url = browser.frontWebView(for: tabSite)?.url { UIApplication.shared.open(url) }
        }
    }
}

/// The site picker's rows: one per site in `WebRouter.tabSites` (the shown one
/// checked), a divider, then 再読み込み and ブラウザで開く. The same buttons fill
/// the tab's long-press panel and the workstation rail row's context menu, where
/// the menu draws them itself and ignores the panel's styling.
struct SitePickerRows: View {
    /// The row under the finger during a press-and-slide.
    var highlighted: SitePick?
    /// Receives each row's frame in window coordinates, for hit-testing the finger.
    var report: ((SitePick, CGRect) -> Void)?
    let pick: (SitePick) -> Void
    @Environment(WebRouter.self) private var web

    var body: some View {
        ForEach(WebRouter.tabSites, id: \.self) { site in
            row(.site(site), WebRouter.name(site), icon: site == web.tabSite ? "checkmark" : nil)
        }
        Divider()
        row(.reload, t("Reload"), icon: "arrow.clockwise")
        if web.browser.frontWebView(for: web.tabSite)?.url != nil {
            row(.openInBrowser, t("Open in browser"), icon: "safari")
        }
    }

    private func row(_ choice: SitePick, _ title: String, icon: String?) -> some View {
        TabPressRow(choice: choice, title: title, icon: icon, highlighted: highlighted, report: report, pick: pick)
    }
}

/// The ウェブ tab item's long press (`TabItemPress`).
typealias SitePickerPress = TabItemPress<SitePick>

/// A school page pushed from a course or assignment, with the system back
/// button and title; back, forward, share and Safari sit in the bottom
/// toolbar as in Safari, and a pull reloads.
struct WebPageScreen: View {
    let page: WebRouter.PushedPage
    @Environment(WebRouter.self) private var web
    @State private var visibilityOwner = UUID()

    var body: some View {
        _ = web.titles[page.site]
        return ZStack {
            ResidentWebView(site: page.site, host: web.browser, revision: web.attachRevision) {
                web.reload(page.site)
            }
            if let error = web.error {
                ProblemNote(text: error)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) { PageBars(site: page.site) }
        .background(Theme.bg)
        .navigationTitle(page.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                PageActions(site: page.site)
            }
            ToolbarItemGroup(placement: .bottomBar) {
                Button { web.back(page.site) } label: { Image(systemName: "chevron.backward") }
                    .accessibilityLabel(t("Previous page"))
                Spacer()
                Button { web.forward(page.site) } label: { Image(systemName: "chevron.forward") }
                    .accessibilityLabel(t("Next page"))
                Spacer()
                if let url = web.browser.frontWebView(for: page.site)?.url {
                    ShareLink(item: url) { Image(systemName: "square.and.arrow.up") }
                    Spacer()
                    Link(destination: url) { Image(systemName: "safari") }
                        .accessibilityLabel(t("Open in browser"))
                }
            }
        }
        .onAppear {
            web.open(page.site, target: page.target)
            web.appear(page.site, owner: visibilityOwner)
        }
        .onDisappear { web.disappear(owner: visibilityOwner) }
    }
}

/// What sits between the navigation bar and the page, outside the school
/// page: the close bar of a popup the page opened over itself, and Core's
/// notice about the page the person sees (it did not load, or a reload was
/// opened again without resending a form).
private struct PageBars: View {
    let site: SchoolSite
    @Environment(WebRouter.self) private var web

    /// What the notice says when a page did not load, by Core's `LoadFailure`.
    private static let failureText: [String: String] = [
        "offline": "This page could not be opened: there is no network connection.",
        "host-not-found":
            "This page could not be opened: its server was not found. Campus-only services need the campus network or VPN.",
        "unreachable":
            "This page could not be opened: its server did not answer. Campus-only services need the campus network or VPN.",
        "insecure": "This page could not be opened: the secure connection failed.",
        "cleartext": "This page uses an unencrypted connection, so it opens only in your browser.",
    ]

    var body: some View {
        VStack(spacing: 0) {
            if (web.popups[site] ?? 0) > 0 {
                HStack {
                    Label(t("Popup window"), systemImage: "macwindow.on.rectangle")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button(t("Close window")) { web.closePopup(site) }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                }
                .padding(.horizontal)
                .padding(.vertical, 8)
                Divider()
            }
            if let notice = web.notices[site] {
                note(notice)
                Divider()
            }
        }
        .background(.bar)
        .animation(.default, value: web.popups[site] ?? 0)
        .animation(.default, value: web.notices[site])
    }

    @ViewBuilder
    private func note(_ notice: WebRouter.Notice) -> some View {
        switch notice.kind {
        case let .loadFailed(reason):
            VStack(alignment: .leading, spacing: 8) {
                Label(t(Self.failureText[reason] ?? "This page could not be opened."), systemImage: "exclamationmark.triangle")
                    .font(.subheadline)
                    .symbolRenderingMode(.multicolor)
                HStack {
                    if let url = notice.url {
                        if reason != "cleartext" {
                            Button(t("Retry")) { web.retry(site) }
                        }
                        Button(t("Open in browser")) { UIApplication.shared.open(url) }
                    }
                    Spacer()
                    Button { web.dismissNotice(site) } label: { Image(systemName: "xmark") }
                        .accessibilityLabel(t("Close"))
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal)
            .padding(.vertical, 8)
        case .reopened:
            Label(t("Reopened this page without sending the form again."), systemImage: "arrow.clockwise")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal)
                .padding(.vertical, 8)
        }
    }
}

private struct ProblemNote: View {
    let text: String
    var body: some View {
        Text(text)
            .font(.subheadline)
            .padding(12)
            .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: 12))
            .padding()
    }
}
