import IsctCore
import SwiftUI

@main
struct IsctCompanionApp: App {
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .environment(model.web)
                .task { await model.start() }
        }
        .onChange(of: scenePhase) { _, phase in model.scenePhase(phase) }
    }
}

/// The desktop's pages (`routes.ts`), as tabs, sidebar rows or both.
enum AppTab: Hashable, CaseIterable {
    case today, timetable, lectures, web, settings

    var title: String {
        switch self {
        case .today: t("Today")
        case .timetable: t("Timetable")
        case .lectures: t("Courses")
        case .web: t("Web")
        case .settings: t("Settings")
        }
    }

    var symbol: String {
        switch self {
        case .today: "calendar"
        case .timetable: "square.grid.3x3"
        case .lectures: "books.vertical"
        case .web: "globe"
        case .settings: "gearshape"
        }
    }
}

/// Which container the pages sit in (docs/ux/mobile.md, Devices and layouts).
enum Layout {
    /// The desktop rule (`form-factor.ts`): one column up to 600 pt wide or in
    /// portrait up to 1100 pt, two columns otherwise.
    static func isStacked(_ size: CGSize) -> Bool {
        size.width <= 600 || (size.height > size.width && size.width <= 1100)
    }

    /// iPhone and iPad differ on purpose: the iPad adds 授業. This is the one
    /// runtime device check AGENTS.md allows (one iOS binary, two idioms).
    /// Assignments live on 今日, and 設定 is the last tab.
    static var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    static var tabs: [AppTab] {
        isPad ? [.today, .timetable, .lectures, .web, .settings] : [.today, .timetable, .web, .settings]
    }

    static let sidebar: [AppTab] = [.today, .timetable, .lectures, .web, .settings]
}

/// A course pushed from any row, block or card. `zoom` names the view it
/// grows out of, when there is one.
struct CourseRoute: Hashable {
    let id: String
    var zoom: String? = nil
}

/// Every assignment of the quarter, pushed from 課題.
struct AssignmentRecordRoute: Hashable {}

/// The one owner of what each page has pushed. Every push goes through a
/// page's path: a view pushed outside it (a destination-closure
/// NavigationLink) is dropped the moment a value is appended, which popped
/// the screen the person was on.
@MainActor
@Observable
final class Navigator {
    var tab: AppTab = .today
    var paths: [AppTab: NavigationPath] = [:]
    /// The course open beside the list in the split 授業.
    var lecture: String?

    /// Pushes onto the current page, for actions that cannot be a
    /// NavigationLink (menu items, buttons).
    func push<Route: Hashable>(_ route: Route) {
        paths[tab, default: NavigationPath()].append(route)
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @State private var navigator = Navigator()
    @State private var sitePress = SitePickerPress()
    @Namespace private var zoom

    var body: some View {
        // System controls (date pickers, number formats) follow the app's
        // language rather than the phone's.
        content.environment(\.locale, Localizer.shared.locale)
    }

    @ViewBuilder
    private var content: some View {
        switch model.status {
        case .opening:
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity).background(Theme.bg)
        case let .failed(message):
            ContentUnavailableView(t("Could not start"), systemImage: "exclamationmark.triangle", description: Text(message))
        case .ready:
            GeometryReader { proxy in
                Group {
                    if Layout.isPad, !Layout.isStacked(proxy.size) {
                        split
                    } else {
                        tabs
                    }
                }
                .environment(navigator)
                .environment(\.courseZoom, zoom)
            }
        }
    }

    private var tabs: some View {
        ZStack {
            TabView(selection: selection) {
                ForEach(Layout.tabs, id: \.self) { tab in
                    page(tab)
                        .tabItem { Label(tab.title, systemImage: tab.symbol) }
                        .tag(tab)
                }
            }
            // Long-pressing the ウェブ item raises the site picker over it. The
            // probe sits beside the TabView, so it works before ウェブ is opened.
            .background {
                TabBarLongPress(index: Layout.tabs.firstIndex(of: .web) ?? 0) { event in
                    if let choice = sitePress.handle(event) { pickSite(choice) }
                }
                .frame(width: 0, height: 0)
            }
            SitePickerPanel(press: sitePress) { pickSite($0) }
        }
        // A page the tab bar lacks falls back to 今日.
        .onAppear { if !Layout.tabs.contains(navigator.tab) { navigator.tab = .today } }
    }

    /// A site picked from the ウェブ tab's panel or the sidebar row's menu
    /// shows on the ウェブ page; the site already shown there stays on its
    /// page instead of going home.
    private func pickSite(_ choice: SitePick) {
        if case let .site(site) = choice, navigator.tab != .web {
            navigator.tab = .web
            if site == web.tabSite { return }
        }
        web.perform(choice)
    }

    /// iPad, wide: the desktop's sidebar and one page beside it.
    private var split: some View {
        NavigationSplitView {
            List(Layout.sidebar, id: \.self, selection: Binding(get: { navigator.tab }, set: { if let tab = $0 { navigator.tab = tab } })) { tab in
                if tab == .web {
                    Label(tab.title, systemImage: tab.symbol)
                        .contextMenu { SitePickerRows { pickSite($0) } }
                } else {
                    Label(tab.title, systemImage: tab.symbol)
                }
            }
            .navigationTitle("IsctCompanion")
            .safeAreaInset(edge: .bottom) {
                HStack {
                    SyncRingButton()
                    Spacer()
                }
                .padding(.horizontal, 8)
            }
        } detail: {
            if navigator.tab == .lectures {
                LecturesSplit()
            } else {
                page(navigator.tab)
            }
        }
    }

    /// A page in its own stack; the same views fill tabs and the sidebar.
    @ViewBuilder
    private func page(_ tab: AppTab) -> some View {
        switch tab {
        case .today: stack(.today) { TodayScreen() }
        case .timetable: stack(.timetable) { TimetableScreen() }
        case .lectures: stack(.lectures) { LecturesScreen() }
        case .web: WebTabScreen()
        case .settings: stack(.settings) { SettingsScreen() }
        }
    }

    /// Tapping the current tab returns it to its root; for ウェブ, to the
    /// shown site's start page.
    private var selection: Binding<AppTab> {
        Binding(
            get: { navigator.tab },
            set: { next in
                if next == navigator.tab {
                    navigator.paths[next] = NavigationPath()
                    if next == .web { web.goHome(web.tabSite) }
                }
                navigator.tab = next
            })
    }

    private func stack<Content: View>(_ tab: AppTab, @ViewBuilder content: () -> Content) -> some View {
        NavigationStack(path: Binding(
            get: { navigator.paths[tab] ?? NavigationPath() }, set: { navigator.paths[tab] = $0 }
        )) {
            content().appDestinations()
        }
    }
}

extension View {
    /// Everything a page can push, registered once per stack.
    func appDestinations() -> some View {
        navigationDestination(for: CourseRoute.self) { route in
            CourseDetailScreen(courseId: route.id).courseZoomDestination(route.zoom)
        }
        .navigationDestination(for: WebRouter.PushedPage.self) { WebPageScreen(page: $0) }
        .navigationDestination(for: AssignmentRecordRoute.self) { _ in RecordScreen() }
        .navigationDestination(for: AssignmentRoute.self) { AssignmentScreen(assignmentId: $0.id) }
        .navigationDestination(for: SettingsRoute.self) { SettingsDestination(route: $0) }
    }
}
