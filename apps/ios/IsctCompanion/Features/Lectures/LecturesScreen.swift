import IsctCore
import SwiftUI

/// 授業 (iPad): the quarter's courses, as the desktop Lectures sidebar lists
/// them (`WorkspaceOverview.termCourseIds`). Stacked, a row pushes the course
/// page; in the split it selects the course shown beside the list.
struct LecturesScreen: View {
    var selection: Binding<String?>? = nil
    @Environment(AppModel.self) private var model
    @State private var overview: WorkspaceOverview?

    var body: some View {
        List(selection: selection) {
            Section {
                ForEach(overview?.termCourseIds ?? [], id: \.self) { id in
                    if selection != nil {
                        row(id).tag(id)
                    } else {
                        NavigationLink(value: CourseRoute(id: id, zoom: "lecture-\(id)")) { row(id) }
                            .courseZoomSource("lecture-\(id)")
                    }
                }
            } header: {
                if let term = overview?.term { Text(Format.quarter(term)) }
            }
        }
        .navigationTitle(t("Courses"))
        .overlay {
            if overview?.termCourseIds.isEmpty == true {
                ContentUnavailableView(t("No courses this term yet"), systemImage: "books.vertical")
            }
        }
        .task(id: "\(model.today?.term ?? "")-\(model.contentRevision)") {
            guard let term = model.today?.term else { return }
            overview = await model.overview(term)
        }
    }

    private func row(_ id: String) -> some View {
        let course = model.records.courses[id] ?? [:]
        return HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(Color(hex: overview?.colors[id] ?? model.colors[id]))
                .frame(width: 6, height: 32)
            VStack(alignment: .leading, spacing: 2) {
                Text(course.text("title") ?? id).font(.body.weight(.medium)).foregroundStyle(Theme.ink).lineLimit(2)
                if let code = course.text("code") {
                    Text(code).font(.footnote.monospaced()).foregroundStyle(Theme.muted)
                }
            }
        }
        .padding(.vertical, 2)
    }
}

/// 授業 beside the sidebar: the course list and the chosen course's page,
/// the desktop LectureDesk's two panes.
struct LecturesSplit: View {
    @Environment(Navigator.self) private var navigator

    var body: some View {
        @Bindable var navigator = navigator
        HStack(spacing: 0) {
            NavigationStack {
                LecturesScreen(selection: $navigator.lecture)
            }
            .frame(width: 320)
            Divider()
            NavigationStack(path: Binding(
                get: { navigator.paths[.lectures] ?? NavigationPath() },
                set: { navigator.paths[.lectures] = $0 }
            )) {
                Group {
                    if let id = navigator.lecture {
                        CourseDetailScreen(courseId: id).id(id)
                    } else {
                        ContentUnavailableView(t("Choose a course"), systemImage: "books.vertical")
                    }
                }
                .appDestinations()
            }
        }
        .onChange(of: navigator.lecture) { navigator.paths[.lectures] = NavigationPath() }
    }
}
