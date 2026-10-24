import IsctCore
import QuickLook
import SwiftUI

/// An assignment pushed from 今日 or 課題: what the course page shows when it
/// opens one in place, as a page of its own.
struct AssignmentRoute: Hashable {
    let id: String
}

struct AssignmentScreen: View {
    let assignmentId: String
    @Environment(AppModel.self) private var model
    @State private var assignment: AssignmentDetail?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if let assignment {
                    NavigationLink(value: CourseRoute(id: assignment.courseId)) {
                        Text(model.records.courses[assignment.courseId]?.text("title") ?? "")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Theme.accent)
                    }
                    .buttonStyle(PressableStyle())
                    Text(assignment.title)
                        .font(.title.bold())
                        .foregroundStyle(Theme.ink)
                    AssignmentBody(assignment: assignment)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
        }
        .background(Theme.bg)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .task(id: "\(assignmentId)-\(model.contentRevision)") {
            let loaded = await model.assignmentDetail(assignmentId)
            if !Task.isCancelled, let loaded { assignment = loaded }
        }
    }
}

/// One assignment inside a lesson: state first, the teacher's text on tap.
struct AssignmentCard: View {
    let assignment: AssignmentDetail
    @Binding var expanded: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                withAnimation(.snappy) { expanded.toggle() }
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(assignment.title)
                        .font(.body.weight(.semibold))
                        .foregroundStyle(Theme.ink)
                        .multilineTextAlignment(.leading)
                    Spacer(minLength: 8)
                    AssignmentState(assignment: assignment)
                    Image(systemName: "chevron.down")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Theme.muted)
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if expanded {
                AssignmentBody(assignment: assignment, showsState: false)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
        .sensoryFeedback(.selection, trigger: expanded)
    }
}

/// Deadline, the LMS text, its files and the way to the submission page.
struct AssignmentBody: View {
    let assignment: AssignmentDetail
    var showsState = true
    @Environment(Navigator.self) private var navigator

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if showsState {
                HStack(spacing: 8) {
                    AssignmentState(assignment: assignment)
                    if let due = assignment.dueAt {
                        Text(t("Due {{date}}", ["date": Format.due(due)])).font(.subheadline).foregroundStyle(Theme.muted)
                    }
                }
            } else if let due = assignment.dueAt {
                Text(t("Due {{date}}", ["date": Format.due(due)])).font(.subheadline).foregroundStyle(Theme.muted)
            }
            if !assignment.bodyHtml.isEmpty {
                HTMLText(html: assignment.bodyHtml)
            } else if !assignment.details.isEmpty {
                Text(assignment.details).font(.body).foregroundStyle(Theme.ink)
            }
            if !assignment.attachments.isEmpty {
                Card {
                    ForEach(Array(assignment.attachments.enumerated()), id: \.element.url) { index, file in
                        if index > 0 { RowDivider() }
                        MaterialButton(title: file.name, kind: "file", source: file.url, files: assignment.files)
                    }
                }
            }
            if let url = assignment.sourceUrl {
                Button {
                    navigator.push(WebRouter.PushedPage(site: .scienceLms, target: url, title: assignment.title))
                } label: {
                    Label(t("Submit in LMS"), systemImage: "arrow.up.forward.square")
                        .font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                        .foregroundStyle(.white)
                        .background(Theme.accent, in: Capsule())
                }
                .buttonStyle(PressableStyle())
            }
        }
    }
}

/// 提出済 / the grade, or the countdown, in the deadline chip's colours.
private struct AssignmentState: View {
    let assignment: AssignmentDetail

    var body: some View {
        let done = ["submitted", "graded"].contains(assignment.submission ?? "")
        let past = Format.date(assignment.dueAt).map { $0 < Date() } ?? false
        let (ink, fill) = DeadlineChip.colors(done ? "done" : past ? "past" : "soon")
        Text(done ? (assignment.grade ?? t("Submitted")) : Format.countdown(assignment.dueAt, now: Date()))
            .font(.caption.weight(.semibold).monospacedDigit())
            .foregroundStyle(ink)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(fill, in: RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
            .fixedSize()
    }
}

/// A course file or page row with its icon.
struct MaterialButton: View {
    let title: String
    let kind: String
    let source: String?
    let files: CourseKey?
    var courseId: String? = nil

    var body: some View {
        MaterialOpener(title: title, source: source, files: files, courseId: courseId) { opening in
            HStack(spacing: 12) {
                Image(systemName: Self.icon(kind)).foregroundStyle(Theme.muted).frame(width: 18)
                Text(title).foregroundStyle(Theme.ink).lineLimit(2).multilineTextAlignment(.leading)
                Spacer(minLength: 8)
                if opening { ProgressView().controlSize(.small) }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
    }

    static func icon(_ kind: String) -> String {
        switch kind {
        case "pdf": "doc.richtext"
        case "video": "play.rectangle"
        case "file": "paperclip"
        case "folder": "folder"
        case "link": "link"
        default: "doc.text"
        }
    }
}

/// Opens a course file or page wherever it is listed. Core fetches it with
/// the signed-in LMS session (or from the course folder) and Quick Look shows
/// it; what is not a file opens in the in-app LMS page. A download that only
/// failed this time (offline, timed out) asks instead: silently showing the
/// LMS page made Share hand out a link where it had handed out the file.
struct MaterialOpener<Label: View>: View {
    let title: String
    let source: String?
    let files: CourseKey?
    /// When `files` is not known yet, the course whose folder Core names.
    var courseId: String? = nil
    @ViewBuilder var label: (_ opening: Bool) -> Label
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var opening = false
    @State private var preview: URL?
    @State private var notice: String?
    @State private var retry = false
    @State private var opened = 0

    var body: some View {
        Button {
            Task { await open() }
        } label: {
            label(opening).contentShape(Rectangle())
        }
        .buttonStyle(PressableStyle())
        .disabled(opening)
        .quickLookPreview($preview)
        .sensoryFeedback(.success, trigger: opened)
        .alert(notice ?? "", isPresented: Binding(get: { notice != nil }, set: { if !$0 { notice = nil } })) {
            Button("OK", role: .cancel) {}
        }
        .alert(t("The file could not be downloaded. Try again, or open it in the LMS."), isPresented: $retry) {
            Button(t("Try again")) { Task { await open() } }
            Button(t("Open in LMS")) { if let source { push(source) } }
            Button(t("Cancel"), role: .cancel) {}
        }
    }

    private func open() async {
        guard let source else { return }
        opening = true
        defer { opening = false }
        var folder = files
        if folder == nil, let courseId {
            // Core names a folder for every course; none means the read failed.
            guard let detail = await model.courseDetail(courseId) else { return retry = true }
            folder = detail.files
        }
        guard let files = folder else { return push(source) }
        switch await model.openMaterial(files, source: source) {
        case let .ready(file)?:
            preview = URL(fileURLWithPath: file.path)
            opened += 1
        case let .tooLarge(bytes)?:
            notice = t("This file is too large ({{size}}). Open it in the LMS.", ["size": ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)])
        case .failed(.login)?:
            notice = t("Sign in to the LMS again. Check your school account in Settings.")
        case nil, .failed(.network)?, .failed(.timeout)?, .failed(.cancelled)?:
            retry = true
        case let .external(url)?:
            push(url)
        default:
            // Folders, pages and anything Core could not fetch: the LMS page itself.
            push(source)
        }
    }

    private func push(_ url: String) {
        navigator.push(WebRouter.PushedPage(site: .scienceLms, target: url, title: title))
    }
}
