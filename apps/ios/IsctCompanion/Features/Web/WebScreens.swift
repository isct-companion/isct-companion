import IsctCore
import SwiftUI
import WebKit

/// Hosts a site's resident WKWebView while on screen and parks it afterwards,
/// so its page, scroll position and session survive tab and route changes.
private struct ResidentWebView: UIViewRepresentable {
    let site: SchoolSite
    let host: SchoolBrowserHost
    /// Changes when Core created a view, so the container attaches it.
    let revision: Int

    func makeUIView(context: Context) -> UIView {
        let container = UIView()
        container.backgroundColor = .systemBackground
        return container
    }

    func updateUIView(_ container: UIView, context: Context) {
        guard let webView = host.webView(for: site) else { return }
        if webView.superview !== container {
            container.subviews.forEach { $0.removeFromSuperview() }
            webView.frame = container.bounds
            webView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            container.addSubview(webView)
        }
    }

    static func dismantleUIView(_ container: UIView, coordinator: ()) {
        container.subviews.compactMap { $0 as? WKWebView }.forEach { webView in
            Task { @MainActor in AppParking.park?(webView) }
        }
    }
}

/// Lets a dismantled container return its view to the browser host.
@MainActor
enum AppParking {
    static var park: ((WKWebView) -> Void)?
}

/// ウェブ tab: one resident view per site, a site switcher on top.
struct WebTabScreen: View {
    @Environment(WebRouter.self) private var web

    var body: some View {
        ZStack(alignment: .top) {
            ResidentWebView(site: web.tabSite, host: web.browser, revision: web.attachRevision)
                .ignoresSafeArea(edges: .bottom)
                .padding(.top, 60)
            if web.loading == web.tabSite && web.browser.webView(for: web.tabSite) == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if let error = web.error {
                ProblemNote(text: error).padding(.top, 70)
            }
            siteBar
        }
        .background(Theme.bg)
        .onAppear {
            AppParking.park = { [browser = web.browser] in browser.park($0) }
            web.showTab(web.tabSite)
            web.setVisible(web.tabSite)
        }
        .onDisappear { web.setVisible(nil) }
    }

    private var siteBar: some View {
        HStack(spacing: 8) {
            Button {
                web.back(web.tabSite)
            } label: {
                Image(systemName: "chevron.left")
                    .font(.body.weight(.semibold))
                    .frame(width: 40, height: 40)
                    .background(.regularMaterial, in: Circle())
            }
            .accessibilityLabel("戻る")
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(WebRouter.tabSites, id: \.self) { site in
                        Button {
                            web.showTab(site)
                            web.setVisible(site)
                        } label: {
                            Text(WebRouter.name(site))
                                .font(.subheadline.weight(.semibold))
                                .padding(.horizontal, 12)
                                .padding(.vertical, 8)
                                .foregroundStyle(site == web.tabSite ? .white : Theme.ink)
                                .background(site == web.tabSite ? Theme.accent : .clear, in: Capsule())
                        }
                    }
                }
                .padding(4)
            }
            .background(.regularMaterial, in: Capsule())
            Menu {
                Button("再読み込み", systemImage: "arrow.clockwise") { web.reload(web.tabSite) }
                Button("進む", systemImage: "chevron.right") { web.forward(web.tabSite) }
                if let url = web.browser.webView(for: web.tabSite)?.url {
                    Link(destination: url) { Label("ブラウザで開く", systemImage: "safari") }
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.body.weight(.semibold))
                    .frame(width: 40, height: 40)
                    .background(.regularMaterial, in: Circle())
            }
        }
        .foregroundStyle(Theme.accent)
        .padding(.horizontal, 12)
        .padding(.top, 8)
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
            AppParking.park = { [browser = web.browser] in browser.park($0) }
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
