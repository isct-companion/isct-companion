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

enum AppTab: Hashable {
    case today, timetable, assignments, web
}

/// A course pushed from any row or block.
struct CourseRoute: Hashable {
    let id: String
}

/// Every assignment of the quarter, pushed from 課題.
struct AssignmentRecordRoute: Hashable {}

/// The one owner of what each tab has pushed. Every push goes through a
/// tab's path: a view pushed outside it (a destination-closure
/// NavigationLink) is dropped the moment a value is appended, which popped
/// the screen the person was on.
@MainActor
@Observable
final class Navigator {
    var tab: AppTab = .today
    var paths: [AppTab: NavigationPath] = [:]

    /// Pushes onto the current tab, for actions that cannot be a
    /// NavigationLink (menu items, buttons).
    func push<Route: Hashable>(_ route: Route) {
        paths[tab, default: NavigationPath()].append(route)
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    @State private var navigator = Navigator()

    var body: some View {
        switch model.status {
        case .opening:
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity).background(Theme.bg)
        case let .failed(message):
            ContentUnavailableView("起動できませんでした", systemImage: "exclamationmark.triangle", description: Text(message))
        case .ready:
            tabs
        }
    }

    private var tabs: some View {
        TabView(selection: selection) {
            stack(.today) { TodayScreen() }
                .tabItem { Label("今日", systemImage: "calendar") }
                .tag(AppTab.today)
            stack(.timetable) { TimetableScreen() }
                .tabItem { Label("時間割", systemImage: "square.grid.3x3") }
                .tag(AppTab.timetable)
            stack(.assignments) { AssignmentsScreen() }
                .tabItem { Label("課題", systemImage: "checklist") }
                .tag(AppTab.assignments)
            WebTabScreen()
                .tabItem { Label("ウェブ", systemImage: "globe") }
                .tag(AppTab.web)
        }
        .environment(navigator)
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
            content()
                .navigationDestination(for: CourseRoute.self) { CourseDetailScreen(courseId: $0.id) }
                .navigationDestination(for: WebRouter.PushedPage.self) { WebPageScreen(page: $0) }
                .navigationDestination(for: AssignmentRecordRoute.self) { _ in RecordScreen() }
        }
    }
}
