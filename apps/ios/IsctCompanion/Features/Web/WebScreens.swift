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

    func makeUIView(context: Context) -> Container {
        let container = Container(host: host)
        container.backgroundColor = .systemBackground
        return container
    }

    func updateUIView(_ container: Container, context: Context) {
        container.site = site
        container.claim()
    }

    static func dismantleUIView(_ container: Container, coordinator: ()) {
        container.release()
    }

    final class Container: UIView {
        let host: SchoolBrowserHost
        var site: SchoolSite?

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
            addSubview(webView)
        }

        /// Returns whatever view it holds to the parking window.
        func release() {
            for case let held as WKWebView in subviews { host.park(held) }
        }
    }
}

/// ウェブ tab: one resident view per site, nearly full screen. Sites are
/// switched by long-pressing the tab (`SitePickerPanel`); back and forward are
/// WebKit's edge swipes and tapping the tab again returns the site home.
struct WebTabScreen: View {
    @Environment(WebRouter.self) private var web

    var body: some View {
        ZStack {
            // Ends at the tab bar: before iOS 26 the bar turns transparent over a
            // web page and the page's text would run under the tab labels.
            ResidentWebView(site: web.tabSite, host: web.browser, revision: web.attachRevision)
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
        .background(Theme.bg)
        .onAppear {
            web.showTab(web.tabSite)
            web.setVisible(web.tabSite)
        }
        .onDisappear { web.setVisible(nil) }
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
            if let url = browser.webView(for: tabSite)?.url { UIApplication.shared.open(url) }
        }
    }
}

/// The site picker's rows: one per site in `WebRouter.tabSites` (the shown one
/// checked), a divider, then 再読み込み and ブラウザで開く. The same buttons fill
/// the tab's long-press panel and the iPad sidebar row's context menu, where
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
        if web.browser.webView(for: web.tabSite)?.url != nil {
            row(.openInBrowser, t("Open in browser"), icon: "safari")
        }
    }

    private func row(_ choice: SitePick, _ title: String, icon: String?) -> some View {
        Button {
            pick(choice)
        } label: {
            if let icon {
                Label(title, systemImage: icon)
            } else {
                Text(title)
            }
        }
        .buttonStyle(SitePickerRowStyle(highlighted: highlighted == choice))
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
            report?(choice, frame)
        }
    }
}

/// A panel row: title leading, symbol trailing, filled while under the finger
/// or pressed.
private struct SitePickerRowStyle: ButtonStyle {
    let highlighted: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .labelStyle(TrailingIconLabelStyle())
            .font(.body)
            .foregroundStyle(Theme.ink)
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                highlighted || configuration.isPressed ? Theme.line.opacity(0.9) : .clear,
                in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .contentShape(Rectangle())
    }
}

private struct TrailingIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack {
            configuration.title
            Spacer(minLength: 12)
            configuration.icon.foregroundStyle(Theme.accent)
        }
    }
}

/// The long-press state of the ウェブ tab item, fed by `TabBarLongPress` and
/// drawn by `SitePickerPanel` in the root.
@MainActor
@Observable
final class SitePickerPress {
    /// The pressed item's frame (window coordinates) while the panel is up.
    private(set) var anchor: CGRect?
    /// Where the finger is while it stays down; nil once lifted.
    private(set) var finger: CGPoint?
    /// The finger left the item, so lifting away from a row dismisses.
    private var moved = false
    /// Bumped when the panel opens, for the haptic.
    private(set) var opened = 0
    /// The panel rows' frames (window coordinates).
    @ObservationIgnored var rows: [SitePick: CGRect] = [:]

    var isOpen: Bool { anchor != nil }

    /// The row under the finger.
    var highlighted: SitePick? {
        guard let finger else { return nil }
        return rows.first { $0.value.contains(finger) }?.key
    }

    /// Feeds one tab-bar event; returns the row chosen by lifting on it.
    func handle(_ event: TabBarLongPress.Event) -> SitePick? {
        switch event {
        case let .began(item, at):
            rows = [:]
            anchor = item
            finger = at
            moved = false
            opened += 1
        case let .changed(point):
            finger = point
            if let anchor, !anchor.insetBy(dx: -8, dy: -8).contains(point) { moved = true }
        case let .ended(point):
            finger = point
            let chosen = highlighted
            finger = nil
            // Lifting on the item leaves the panel open for a tap.
            if chosen != nil || moved { dismiss() }
            return chosen
        case .cancelled:
            finger = nil
            dismiss()
        }
        return nil
    }

    func dismiss() {
        anchor = nil
        finger = nil
    }
}

/// The glass panel raised above the ウェブ tab item by a long press.
struct SitePickerPanel: View {
    let press: SitePickerPress
    let pick: (SitePick) -> Void

    private static let width: CGFloat = 240

    var body: some View {
        GeometryReader { proxy in
            if let anchor = press.anchor {
                let origin = proxy.frame(in: .global).origin
                let half = Self.width / 2
                let x = min(max(anchor.midX - origin.x, half + 12), proxy.size.width - half - 12)
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(0.08)
                        .contentShape(Rectangle())
                        .onTapGesture { withAnimation(.snappy) { press.dismiss() } }
                    VStack(spacing: 0) {
                        SitePickerRows(highlighted: press.highlighted, report: { press.rows[$0] = $1 }) { choice in
                            withAnimation(.snappy) { press.dismiss() }
                            pick(choice)
                        }
                    }
                    .padding(6)
                    .frame(width: Self.width)
                    .floatingGlass(in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                    .transition(.scale(scale: 0.85, anchor: .bottom).combined(with: .opacity))
                    // Bottom edge 10 pt above the tab item, centred on it.
                    .frame(height: max(0, anchor.minY - origin.y - 10), alignment: .bottom)
                    .offset(x: x - half)
                }
            }
        }
        .ignoresSafeArea()
        .animation(.snappy, value: press.isOpen)
        .sensoryFeedback(.impact(weight: .medium), trigger: press.opened)
        .sensoryFeedback(.selection, trigger: press.highlighted)
    }
}

/// A school page pushed from a course or assignment; back returns to it.
struct WebPageScreen: View {
    let page: WebRouter.PushedPage
    @Environment(WebRouter.self) private var web
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ZStack {
            ResidentWebView(site: page.site, host: web.browser, revision: web.attachRevision)
                .ignoresSafeArea()
            if let error = web.error {
                ProblemNote(text: error)
            }
        }
        .safeAreaInset(edge: .top) { topBar }
        .safeAreaInset(edge: .bottom) { bottomBar }
        .toolbar(.hidden, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
        .onAppear {
            web.open(page.site, target: page.target)
            web.setVisible(page.site)
        }
        .onDisappear { web.setVisible(nil) }
    }

    private var topBar: some View {
        HStack(spacing: 8) {
            circle("chevron.left", label: t("Back")) { dismiss() }
            VStack(spacing: 0) {
                Text(page.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                Label(WebRouter.name(page.site), systemImage: "lock.fill")
                    .font(.caption2)
                    .foregroundStyle(Theme.muted)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 6)
            .background(.regularMaterial, in: Capsule())
            Button {
                web.browser.cycleTextScale()
            } label: {
                Text(t("Aa"))
                    .font(.subheadline.weight(.semibold))
                    .frame(width: 40, height: 40)
                    .background(.regularMaterial, in: Circle())
            }
            .accessibilityLabel(t("Text size"))
        }
        .foregroundStyle(Theme.ink)
        .padding(.horizontal, 12)
        .padding(.top, 4)
    }

    private var bottomBar: some View {
        HStack {
            Button { web.back(page.site) } label: { Image(systemName: "chevron.left") }
                .accessibilityLabel(t("Previous page"))
            Spacer()
            Button { web.forward(page.site) } label: { Image(systemName: "chevron.right") }
                .accessibilityLabel(t("Next page"))
            Spacer()
            if let url = web.browser.webView(for: page.site)?.url {
                ShareLink(item: url) { Image(systemName: "square.and.arrow.up") }
                Spacer()
                Link(destination: url) { Image(systemName: "safari") }
                    .accessibilityLabel(t("Open in browser"))
            }
        }
        .font(.title3)
        .foregroundStyle(Theme.accent)
        .padding(.horizontal, 28)
        .padding(.vertical, 14)
        .background(.regularMaterial, in: Capsule())
        .padding(.horizontal, 16)
        .padding(.bottom, 4)
    }

    private func circle(_ symbol: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.body.weight(.semibold))
                .frame(width: 40, height: 40)
                .background(.regularMaterial, in: Circle())
        }
        .accessibilityLabel(label)
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
