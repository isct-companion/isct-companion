import IsctCore
import SwiftUI

/// A downloaded course file Core shows as Markdown (`Viewer.markdown`):
/// pushed as a page, since Quick Look would show its source.
struct CourseMarkdownRoute: Hashable {
    let path: String
    let title: String
}

struct CourseMarkdownScreen: View {
    let route: CourseMarkdownRoute

    var body: some View {
        let file = URL(fileURLWithPath: route.path)
        MarkdownFileView(path: route.path) {
            QuickLookView(url: file)
        }
        .ignoresSafeArea(edges: .bottom)
        .background(Theme.bg)
        .navigationTitle(route.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                ShareLink(item: file) { Label(t("common.share"), systemImage: "square.and.arrow.up") }
            }
        }
    }
}

/// A downloaded ZIP Core lists as its files (`Material.archive`), shown as a
/// folder. Each file opens like any course item: Core extracts it on demand
/// and keeps it as a downloaded file of its own.
struct CourseArchiveRoute: Hashable {
    let courseId: String
    let source: String
    let title: String
}

struct CourseArchiveScreen: View {
    let route: CourseArchiveRoute
    @Environment(AppModel.self) private var model
    @State private var files: [ArchiveFile]?
    @State private var failed = false

    var body: some View {
        Group {
            if let files {
                List(files, id: \.url) { file in
                    MaterialOpener(title: file.name, source: file.url, courseId: route.courseId) { opening in
                        HStack(spacing: 12) {
                            Image(systemName: "doc").foregroundStyle(Theme.muted).frame(width: 18)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(file.name.split(separator: "/").last.map(String.init) ?? file.name)
                                    .foregroundStyle(Theme.ink).lineLimit(2)
                                // A file inside a folder of the archive names that folder.
                                if file.name.contains("/") {
                                    Text(file.name.split(separator: "/").dropLast().joined(separator: "/"))
                                        .font(.footnote).foregroundStyle(Theme.muted).lineLimit(1)
                                }
                            }
                            Spacer(minLength: 8)
                            if opening {
                                ProgressView().controlSize(.small)
                            } else {
                                Text(ByteCountFormatter.string(fromByteCount: Int64(file.bytes), countStyle: .file))
                                    .font(.footnote).foregroundStyle(Theme.muted)
                            }
                        }
                    }
                }
                .listStyle(.plain)
            } else if failed {
                ContentUnavailableView {
                    Label(route.title, systemImage: "doc.zipper")
                } actions: {
                    Button(t("common.retry")) { Task { await load() } }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Theme.bg)
        .navigationTitle(route.title)
        .navigationBarTitleDisplayMode(.inline)
        .task { if files == nil { await load() } }
    }

    /// The archive is on disk by now; opening it again reads its listing.
    private func load() async {
        failed = false
        if case let .archive(_, listed, _)? = await model.openMaterial(courseId: route.courseId, source: route.source) {
            files = listed
        } else {
            failed = true
        }
    }
}
