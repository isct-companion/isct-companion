import IsctCore
import SwiftUI

// The wide container (slotledger `approach/mobile-layout-by-width`): desktop's
// rail, workbench and status bar around the same page views the tab bar
// shows.

/// Desktop's page rail: the pages, then 設定 at its foot. ⌘1–4 and ⌘, switch
/// pages from a keyboard, as on desktop.
struct PageRail<Menu: View>: View {
    @Binding var selection: AppTab
    /// A row's context menu: the site picker on ウェブ, the calendar export on
    /// 時間割.
    @ViewBuilder var menu: (AppTab) -> Menu

    var body: some View {
        VStack(spacing: 4) {
            ForEach(Array(Layout.rail.enumerated()), id: \.element) { index, tab in
                item(tab)
                    .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
            }
            Spacer(minLength: 8)
            item(.settings).keyboardShortcut(",", modifiers: .command)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 14)
        .frame(width: 72)
        .background(Theme.surface.opacity(0.7).ignoresSafeArea())
    }

    private func item(_ tab: AppTab) -> some View {
        let selected = selection == tab
        return Button {
            selection = tab
        } label: {
            VStack(spacing: 4) {
                Image(systemName: tab.symbol)
                    .font(.title3)
                    .frame(height: 24)
                Text(tab.title)
                    .font(.caption2.weight(.semibold))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .foregroundStyle(selected ? Theme.accent : Theme.muted)
            .background(selected ? Theme.accentSoft : .clear, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .buttonStyle(.plain)
        .hoverEffect(.highlight)
        .contextMenu { menu(tab) }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Desktop's TodayWorkbench: 今日 alone as the two-column dashboard, or
/// narrowed to one column with what it opened beside it. Opening the same row
/// again, or Close, returns to the dashboard.
struct TodayWorkbench: View {
    @Environment(Navigator.self) private var navigator

    var body: some View {
        let open = navigator.bench != nil || navigator.paths[.today]?.isEmpty == false
        HStack(spacing: 0) {
            NavigationStack { TodayScreen(columns: !open) }
                .frame(maxWidth: open ? 360 : .infinity)
            if open {
                Divider().ignoresSafeArea(edges: .top)
                NavigationStack(path: Binding(
                    get: { navigator.paths[.today] ?? NavigationPath() },
                    set: { navigator.paths[.today] = $0 }
                )) {
                    BenchRoot().appDestinations()
                }
                .transition(.move(edge: .trailing).combined(with: .opacity))
            }
        }
        .frame(maxWidth: .infinity)
        .background(Theme.bg)
        .animation(.smooth(duration: Theme.motionSlow), value: open)
    }
}

/// The root of 今日's pane: the page `appDestinations` would push for the
/// opened route, with Close where back would be.
private struct BenchRoot: View {
    @Environment(Navigator.self) private var navigator

    var body: some View {
        page
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(t("Close"), systemImage: "xmark") { navigator.closeBench() }
                }
            }
    }

    @ViewBuilder
    private var page: some View {
        switch navigator.bench {
        case let route as CourseRoute: CourseDetailScreen(courseId: route.id).id(route.id)
        case let route as AssignmentRoute: AssignmentScreen(assignmentId: route.id).id(route.id)
        case let page as WebRouter.PushedPage: WebPageScreen(page: page)
        case let route as SettingsRoute: SettingsDestination(route: route)
        case is AssignmentRecordRoute: RecordScreen()
        default:
            // Pages pushed before the window widened, without what they were
            // pushed from.
            ContentUnavailableView(t("Choose a course"), systemImage: "books.vertical")
        }
    }
}

/// Desktop's status bar under the page: each school source's sign-in and last
/// update, and the sync ring. Tapping it opens the connection sheet.
struct SchoolStatusBar: View {
    @Environment(AppModel.self) private var model
    @State private var showsSheet = false

    var body: some View {
        HStack(spacing: 20) {
            Button {
                showsSheet = true
            } label: {
                HStack(spacing: 20) {
                    ForEach(sources, id: \.source) { source in
                        HStack(spacing: 8) {
                            Circle().fill(color(source)).frame(width: 8, height: 8)
                            VStack(alignment: .leading, spacing: 0) {
                                Text(SyncText.source(source.source))
                                    .font(.caption.weight(.bold))
                                    .foregroundStyle(Theme.ink)
                                Text(source.lastUpdated.map { t("Updated {{date}}", ["date": Format.due($0)]) } ?? t("Never updated"))
                                    .font(.caption2.monospacedDigit())
                                    .foregroundStyle(Theme.muted)
                            }
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(t("School connection"))
            SyncRingButton()
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 48)
        .background(Theme.surface.ignoresSafeArea(edges: .bottom))
        .overlay(alignment: .top) { Rectangle().fill(Theme.line).frame(height: 0.5) }
        .sheet(isPresented: $showsSheet) { SyncStatusSheet() }
    }

    /// The school sites, which have a sign-in; reference data and the catalog
    /// stay in the sheet.
    private var sources: [SourceView] {
        model.syncSnapshot?.sources.filter { $0.session != nil } ?? []
    }

    private func color(_ source: SourceView) -> Color {
        if source.failure != nil { return Theme.danger }
        if source.maintenance != nil { return Theme.warn }
        return source.session == "signed-out" ? Theme.muted : Theme.ok
    }
}
