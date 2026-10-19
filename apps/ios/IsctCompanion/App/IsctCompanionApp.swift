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
    case today, timetable, lectures, assignments, web, settings

    var title: String {
        switch self {
        case .today: "今日"
        case .timetable: "時間割"
        case .lectures: "授業"
        case .assignments: "課題"
        case .web: "ウェブ"
        case .settings: "設定"
        }
    }

    var symbol: String {
        switch self {
        case .today: "calendar"
        case .timetable: "square.grid.3x3"
        case .lectures: "books.vertical"
        case .assignments: "checklist"
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
    static var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    static var tabs: [AppTab] {
        isPad ? [.today, .timetable, .lectures, .assignments, .web] : [.today, .timetable, .assignments, .web]
    }

    static let sidebar: [AppTab] = [.today, .timetable, .lectures, .assignments, .web, .settings]
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
    @State private var navigator = Navigator()
    @Namespace private var zoom

    var body: some View {
        switch model.status {
        case .opening:
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity).background(Theme.bg)
        case let .failed(message):
            ContentUnavailableView("起動できませんでした", systemImage: "exclamationmark.triangle", description: Text(message))
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
        TabView(selection: selection) {
            ForEach(Layout.tabs, id: \.self) { tab in
                page(tab)
                    .tabItem { Label(tab.title, systemImage: tab.symbol) }
                    .tag(tab)
            }
        }
        // Turning an iPad upright from 設定 in the sidebar lands on a page the
        // tab bar has.
        .onAppear { if !Layout.tabs.contains(navigator.tab) { navigator.tab = .today } }
    }

    /// iPad, wide: the desktop's sidebar and one page beside it.
    private var split: some View {
        NavigationSplitView {
            List(Layout.sidebar, id: \.self, selection: Binding(get: { navigator.tab }, set: { if let tab = $0 { navigator.tab = tab } })) { tab in
                Label(tab.title, systemImage: tab.symbol)
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
        case .assignments: stack(.assignments) { AssignmentsScreen() }
        case .web: WebTabScreen()
        case .settings: stack(.settings) { SettingsScreen() }
        }
    }

    /// Tapping the current tab returns it to its root.
    private var selection: Binding<AppTab> {
        Binding(
            get: { navigator.tab },
            set: { next in
                if next == navigator.tab { navigator.paths[next] = NavigationPath() }
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
        .navigationDestination(for: SettingsRoute.self) { SettingsDestination(route: $0) }
    }
}
