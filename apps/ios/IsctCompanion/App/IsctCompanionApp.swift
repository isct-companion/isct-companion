import IsctCore
import QuickLook
import SwiftUI

@main
struct IsctCompanionApp: App {
    @State private var model = AppModel()
    @State private var inbox = InboxModel()
    @Environment(\.scenePhase) private var scenePhase

    init() {
        Guide.configure()
        NotificationScheduler.presentInForeground()
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .environment(model.web)
                .environment(inbox)
                .task { await model.start() }
        }
        .onChange(of: scenePhase) { _, phase in model.scenePhase(phase) }
        .backgroundTask(.appRefresh(AppModel.backgroundRefresh)) { [model] in
            await model.backgroundRefresh()
        }
    }
}

/// The desktop's pages (`routes.ts`), as tabs, rail rows or both.
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

/// Which container the pages sit in (slotledger `approach/mobile-layout-by-width`).
/// The window's size picks it, on iPhone and iPad alike: the tab bar when
/// stacked, the desktop's workstation otherwise.
enum Layout {
    /// The desktop rule (`form-factor.ts`): one column up to 600 pt wide or in
    /// portrait up to 1100 pt, the workstation otherwise.
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

    /// The workstation's rail, as desktop's; 設定 sits apart at its foot.
    static let rail: [AppTab] = [.today, .timetable, .lectures, .web]
}

/// 授業's two lists, as desktop's LectureDesk switches them.
enum LectureMode: Hashable {
    case courses, assignments
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
    /// The assignment open beside 授業's 課題 list.
    var lectureAssignment: String?
    /// The quarter 授業 shows, this quarter until one is picked.
    var lectureTerm: String?
    /// Whether 授業 lists the quarter's courses or its assignments.
    var lectureMode = LectureMode.courses
    /// The quarter 時間割 shows, for its tab's calendar export.
    var timetableTerm: String?
    /// The workstation is up: 今日 opens what its column points at beside
    /// itself, as desktop's TodayWorkbench, instead of pushing it.
    private(set) var workbench = false
    /// What 今日 opened beside itself, the root of its pane's stack.
    var bench: (any Hashable)?

    /// Pushes onto the current page, for actions that cannot be a
    /// NavigationLink (menu items, buttons).
    func push<Route: Hashable>(_ route: Route) {
        paths[tab, default: NavigationPath()].append(route)
    }

    /// Opens from 今日's column: beside it on the workstation, pushed
    /// otherwise. Opening what is already open closes it, as on desktop.
    func open(_ route: any Hashable) {
        guard workbench, tab == .today else { return push(route) }
        var route = route
        // One course, whichever card it grew out of.
        if var course = route as? CourseRoute {
            course.zoom = nil
            route = course
        }
        let shown = bench.map { AnyHashable($0) == AnyHashable(route) } ?? false
        bench = shown && paths[.today]?.isEmpty != false ? nil : route
        paths[.today] = NavigationPath()
    }

    /// Closes 今日's pane.
    func closeBench() {
        bench = nil
        paths[.today] = NavigationPath()
    }

    /// Follows the container. Leaving the workstation, what was open beside
    /// 今日 becomes its push; pages pushed over it there do not follow.
    func setWorkbench(_ wide: Bool) {
        guard wide != workbench else { return }
        workbench = wide
        if !wide, let bench {
            var path = NavigationPath()
            path.append(bench)
            paths[.today] = path
            self.bench = nil
        }
    }
}

/// How a view opens a route. Pages push; 今日's column opens beside itself
/// on the workstation (`Navigator.open`).
struct PushAction {
    var action: (any Hashable) -> Void = { _ in }

    func callAsFunction(_ route: some Hashable) { action(route) }
}

extension EnvironmentValues {
    @Entry var push = PushAction()
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @Environment(InboxModel.self) private var inbox
    @Environment(\.scenePhase) private var scenePhase
    @State private var navigator = Navigator()
    @State private var sitePress = SitePickerPress()
    @State private var exportPress = TabItemPress<CalendarExportPick>()
    @State private var calendarFile: QuarterCalendarFile?
    @State private var exportError: String?
    @Namespace private var zoom

    var body: some View {
        // System controls (date pickers, number formats) follow the app's
        // language rather than the phone's.
        content.environment(\.locale, Localizer.shared.locale)
            .environment(\.isctTheme, model.records.settings.text("theme") ?? "warm")
            .quickLookPreview(Binding(get: { web.downloads.first }, set: { value in
                if value == nil { web.dismissDownload() }
            }))
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
                let stacked = Layout.isStacked(proxy.size)
                Group {
                    if stacked {
                        tabs
                    } else {
                        workstation
                    }
                }
                .onChange(of: stacked, initial: true) { _, stacked in navigator.setWorkbench(!stacked) }
                // What was shared into the app waits in the corner, above the
                // tab bar or the status bar.
                .overlay { InboxTray(bottom: stacked ? 64 : 60) }
                .task(id: scenePhase) {
                    if scenePhase == .active { await inbox.refresh(model) }
                }
                // "Open in IsctCompanion" from another app joins the corner
                // stack; the share sheet's "open the app" link only brings it
                // forward, where the refresh above reads the inbox.
                .onOpenURL { url in if url.isFileURL { inbox.receive(url) } }
                .sheet(isPresented: Binding(get: { inbox.presenting }, set: { inbox.presenting = $0 })) {
                    InboxSheet()
                }
                // A share-sheet pick was handed in: show that assignment.
                .onChange(of: inbox.handedTo) { _, assignment in
                    guard let assignment else { return }
                    navigator.tab = .today
                    navigator.open(AssignmentRoute(id: assignment))
                    inbox.handedTo = nil
                }
                .alert(inbox.error ?? "", isPresented: Binding(get: { inbox.error != nil }, set: { if !$0 { inbox.error = nil } })) {
                    Button("OK", role: .cancel) {}
                }
                .environment(navigator)
                .environment(\.push, PushAction { navigator.push($0) })
                .environment(\.courseZoom, zoom)
                .welcomeSheet {
                    navigator.tab = .today
                    navigator.open(SettingsRoute.account(.science))
                }
                .sheet(item: $calendarFile) { file in
                    ShareSheet(url: file.url).presentationDetents([.medium, .large])
                }
                .alert(exportError ?? "", isPresented: Binding(get: { exportError != nil }, set: { if !$0 { exportError = nil } })) {
                    Button("OK", role: .cancel) {}
                }
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
            // Long-pressing 時間割 offers the shown quarter as a calendar file.
            .background {
                TabBarLongPress(index: Layout.tabs.firstIndex(of: .timetable) ?? 0) { event in
                    guard exportTerm != nil else { return }
                    if let choice = exportPress.handle(event) { export(choice) }
                }
                .frame(width: 0, height: 0)
            }
            TabPressPanel(press: sitePress, pick: pickSite) { highlighted, report, pick in
                SitePickerRows(highlighted: highlighted, report: report, pick: pick)
            }
            TabPressPanel(press: exportPress, pick: export) { highlighted, report, pick in
                if let term = exportTerm {
                    CalendarExportRows(term: term, highlighted: highlighted, report: report, pick: pick)
                }
            }
        }
        // A page the tab bar lacks falls back to 今日.
        .onAppear { if !Layout.tabs.contains(navigator.tab) { navigator.tab = .today } }
    }

    /// A site picked from the ウェブ tab's panel or the rail row's menu
    /// shows on the ウェブ page; the site already shown there stays on its
    /// page instead of going home.
    private func pickSite(_ choice: SitePick) {
        if case let .site(site) = choice, navigator.tab != .web {
            navigator.tab = .web
            if site == web.tabSite { return }
        }
        web.perform(choice)
    }

    /// The quarter the timetable shows, or this quarter before it is opened.
    private var exportTerm: String? {
        navigator.timetableTerm ?? model.today?.term
    }

    private func export(_ pick: CalendarExportPick) {
        guard let term = exportTerm else { return }
        Task {
            do {
                calendarFile = try await QuarterCalendarFile.write(model: model, term: term, pick: pick)
            } catch {
                exportError = t("Export failed: {{reason}}", ["reason": AppModel.describe(error)])
            }
        }
    }

    /// Wide: the desktop's workstation (slotledger
    /// `approach/mobile-layout-by-width`): the page rail, one page beside it and the school status bar
    /// under the page.
    private var workstation: some View {
        HStack(spacing: 0) {
            PageRail(selection: selection) { tab in
                switch tab {
                case .web: SitePickerRows { pickSite($0) }
                case .timetable:
                    if let term = exportTerm { CalendarExportRows(term: term) { export($0) } }
                default: EmptyView()
                }
            }
            Divider().ignoresSafeArea()
            VStack(spacing: 0) {
                switch navigator.tab {
                case .today: TodayWorkbench()
                case .lectures: LecturesSplit()
                default: page(navigator.tab)
                }
                SchoolStatusBar()
            }
        }
        .background(Theme.bg)
    }

    /// A page in its own stack; the same views fill tabs and the workstation.
    @ViewBuilder
    private func page(_ tab: AppTab) -> some View {
        switch tab {
        case .today: stack(.today) { TodayScreen() }
        case .timetable: stack(.timetable) { TimetableScreen() }
        case .lectures: stack(.lectures) { LecturesScreen() }
        case .web: stack(.web) { WebTabScreen() }
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
                    if next == .today { navigator.bench = nil }
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
    /// Everything a page can push, registered once per stack. `BenchRoot`
    /// shows the same pages as the root of 今日's pane.
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
