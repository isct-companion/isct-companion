import IsctCore
import SwiftUI
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

/// ウェブ tab: one resident view per site, nearly full screen. The site
/// switcher is a floating button the person can drag to either edge, so it
/// never sits on what they are reading.
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
            SiteSwitcher()
        }
        .background(Theme.bg)
        .onAppear {
            web.showTab(web.tabSite)
            web.setVisible(web.tabSite)
        }
        .onDisappear { web.setVisible(nil) }
    }
}

/// The floating site button: tap for sites and page actions, drag to move;
/// on release it settles against the nearer side.
private struct SiteSwitcher: View {
    @Environment(WebRouter.self) private var web
    @State private var open = false
    @State private var drag: CGSize = .zero
    /// Starts on the left: school pages keep their own controls on the right.
    @AppStorage("web.switcher.trailing") private var trailing = false
    @AppStorage("web.switcher.height") private var height = 0.78

    private static let size: CGFloat = 56
    private static let margin: CGFloat = 14

    var body: some View {
        GeometryReader { proxy in
            let bounds = proxy.size
            let x = trailing ? bounds.width - Self.margin - Self.size / 2 : Self.margin + Self.size / 2
            let y = min(max(height * bounds.height, Self.size), bounds.height - Self.size)
            ZStack {
                if open {
                    Color.black.opacity(0.08)
                        .ignoresSafeArea()
                        .onTapGesture { withAnimation(.snappy) { open = false } }
                    menu
                        .position(x: trailing ? bounds.width - 120 : 120, y: y > bounds.height / 2 ? y - 170 : y + 170)
                        .transition(.scale(scale: 0.85, anchor: trailing ? .bottomTrailing : .bottomLeading).combined(with: .opacity))
                }
                button
                    .position(x: x + drag.width, y: y + drag.height)
                    .gesture(
                        DragGesture(minimumDistance: 6)
                            .onChanged { drag = $0.translation }
                            .onEnded { value in
                                let endX = x + value.translation.width
                                let endY = y + value.translation.height
                                withAnimation(.spring(response: 0.35, dampingFraction: 0.78)) {
                                    trailing = endX > bounds.width / 2
                                    height = min(max(endY / bounds.height, 0.12), 0.9)
                                    drag = .zero
                                }
                            })
            }
        }
        .sensoryFeedback(.selection, trigger: trailing)
        .sensoryFeedback(.impact(weight: .light), trigger: open)
    }

    private var button: some View {
        Button {
            withAnimation(.snappy) { open.toggle() }
        } label: {
            Text(Self.short(web.tabSite))
                .font(.caption.weight(.bold))
                .foregroundStyle(Theme.accent)
                .frame(width: Self.size, height: Self.size)
                .floatingGlass(in: Circle())
                .overlay { Circle().strokeBorder(Theme.line, lineWidth: 0.5) }
        }
        .buttonStyle(PressableStyle())
        .accessibilityLabel("サイトを切り替え · \(WebRouter.name(web.tabSite))")
    }

    private var menu: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(WebRouter.tabSites, id: \.self) { site in
                menuRow(WebRouter.name(site), icon: site == web.tabSite ? "checkmark" : "globe", selected: site == web.tabSite) {
                    web.choose(site)
                }
            }
            Divider().padding(.vertical, 4)
            HStack(spacing: 0) {
                iconButton("chevron.left", "戻る") { web.back(web.tabSite) }
                iconButton("chevron.right", "進む") { web.forward(web.tabSite) }
                iconButton("arrow.clockwise", "再読み込み") { web.reload(web.tabSite) }
                iconButton("house", "このサイトの最初へ") { web.goHome(web.tabSite) }
                if let url = web.browser.webView(for: web.tabSite)?.url {
                    Link(destination: url) {
                        Image(systemName: "safari").frame(maxWidth: .infinity, minHeight: 40)
                    }
                    .accessibilityLabel("ブラウザで開く")
                }
            }
            .font(.body.weight(.semibold))
            .foregroundStyle(Theme.accent)
        }
        .padding(8)
        .frame(width: 220)
        .floatingGlass(in: RoundedRectangle(cornerRadius: 22, style: .continuous))
    }

    private func menuRow(_ title: String, icon: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button {
            action()
            withAnimation(.snappy) { open = false }
        } label: {
            HStack {
                Text(title).font(.body.weight(selected ? .semibold : .regular)).foregroundStyle(Theme.ink)
                Spacer()
                if selected { Image(systemName: icon).foregroundStyle(Theme.accent) }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressableStyle())
    }

    private func iconButton(_ symbol: String, _ label: String, action: @escaping () -> Void) -> some View {
        Button {
            action()
            withAnimation(.snappy) { open = false }
        } label: {
            Image(systemName: symbol).frame(maxWidth: .infinity, minHeight: 40)
        }
        .accessibilityLabel(label)
    }

    /// The button's own label: short enough for a circle.
    private static func short(_ site: SchoolSite) -> String {
        switch site {
        case .scienceLms: "LMS"
        case .kyomu: "教務"
        case .sciencePortal: "Portal"
        case .tokyoMail: "Mail"
        case .library: "図書館"
        default: String(WebRouter.name(site).prefix(4))
        }
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
            circle("chevron.left", label: "戻る") { dismiss() }
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
                Text("ぁあ")
                    .font(.subheadline.weight(.semibold))
                    .frame(width: 40, height: 40)
                    .background(.regularMaterial, in: Circle())
            }
            .accessibilityLabel("文字の大きさ")
        }
        .foregroundStyle(Theme.ink)
        .padding(.horizontal, 12)
        .padding(.top, 4)
    }

    private var bottomBar: some View {
        HStack {
            Button { web.back(page.site) } label: { Image(systemName: "chevron.left") }
                .accessibilityLabel("前のページ")
            Spacer()
            Button { web.forward(page.site) } label: { Image(systemName: "chevron.right") }
                .accessibilityLabel("次のページ")
            Spacer()
            if let url = web.browser.webView(for: page.site)?.url {
                ShareLink(item: url) { Image(systemName: "square.and.arrow.up") }
                Spacer()
                Link(destination: url) { Image(systemName: "safari") }
                    .accessibilityLabel("ブラウザで開く")
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
