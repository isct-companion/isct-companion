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
                    if assignment.sourceUrl != nil { HandInCard(assignment: assignment) }
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
            guard !Task.isCancelled, let loaded else { return }
            // The text is ready before the page shows it, outside any view update.
            HTMLCache.warm([loaded.bodyHtml])
            assignment = loaded
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
                withAnimation(.smooth(duration: Theme.motionBase)) { expanded.toggle() }
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
    }
}

/// Deadline, the LMS text, its files and the way to the submission page.
struct AssignmentBody: View {
    let assignment: AssignmentDetail
    var showsState = true
    @Environment(\.push) private var push

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if showsState {
                HStack(spacing: 8) {
                    AssignmentState(assignment: assignment)
                    if let due = assignment.dueAt {
                        Text(t("course.dueDate", ["date": Format.due(due)])).font(.subheadline).foregroundStyle(Theme.muted)
                    }
                }
            } else if let due = assignment.dueAt {
                Text(t("course.dueDate", ["date": Format.due(due)])).font(.subheadline).foregroundStyle(Theme.muted)
            }
            if !assignment.bodyHtml.isEmpty {
                HTMLText(html: assignment.bodyHtml, courseId: assignment.courseId)
            } else if !assignment.details.isEmpty {
                Text(assignment.details).font(.body).foregroundStyle(Theme.ink)
            }
            if !assignment.attachments.isEmpty {
                Card {
                    ForEach(Array(assignment.attachments.enumerated()), id: \.element.url) { index, file in
                        if index > 0 { RowDivider() }
                        MaterialButton(title: file.name, kind: .file, source: file.url, courseId: assignment.courseId)
                    }
                }
            }
            if let url = assignment.sourceUrl {
                Button {
                    push(WebRouter.PushedPage(site: .scienceLms, target: url, title: assignment.title))
                } label: {
                    Label(t("common.submitInLMS"), systemImage: "arrow.up.forward.square")
                        .font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                }
                .buttonStyle(CapsuleButtonStyle())
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
        Text(done ? (assignment.grade ?? t("common.submitted")) : Format.countdown(assignment.dueAt, now: Date()))
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
    let kind: LessonItemKind
    let source: String?
    let courseId: String

    var body: some View {
        MaterialOpener(title: title, source: source, courseId: courseId) { opening in
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

    static func icon(_ kind: LessonItemKind) -> String {
        switch kind {
        case .file: "doc"
        case .folder: "folder"
        case .link, .other: "link"
        case .forum: "bubble.left.and.bubble.right"
        case .page, .label: "doc.text"
        }
    }
}

/// A course item or link opened wherever it is listed or linked.
struct MaterialOpener<Label: View>: View {
    let title: String
    let source: String?
    let courseId: String
    @ViewBuilder var label: (_ opening: Bool) -> Label
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push
    @Environment(\.openURL) private var openURL
    @State private var opening = MaterialOpening()

    var body: some View {
        Button {
            guard let source else { return }
            Task { await opening.open(courseId, source, title: title, at: MaterialPlace(model: model, push: push, openURL: openURL)) }
        } label: {
            label(opening.busy).contentShape(Rectangle())
        }
        .buttonStyle(PressableStyle())
        .disabled(opening.busy)
        .materialOpening(opening)
    }
}

/// What opening needs from the screen it happens on.
struct MaterialPlace {
    let model: AppModel
    let push: PushAction
    let openURL: OpenURLAction

    func show(_ page: SchoolWebTarget, title: String) {
        push(WebRouter.PushedPage(site: page.site, target: page.url, title: title))
    }
}

/// Opens an item or link as Core decides (`open_material`): a file Core
/// fetches with the signed-in LMS session (or from the course folder) shows
/// in Quick Look, a school page opens in its in-app view, any other page in
/// the system browser. When the file is not shown, the alert offers the LMS
/// page Core names for it. A download that only failed this time asks
/// instead of showing the LMS page: silently doing so made Share hand out a
/// link where it had handed out the file.
@MainActor @Observable
final class MaterialOpening {
    var busy = false
    var preview: URL?
    var notice: String?
    var retrying = false
    /// The LMS page Core named for the item last opened.
    var fallback: SchoolWebTarget?
    var title = ""
    private var last: (course: String, source: String)?

    func open(_ course: String, _ source: String, title: String, at place: MaterialPlace) async {
        last = (course, source)
        self.title = title
        fallback = nil
        busy = true
        defer { busy = false }
        switch await place.model.openMaterial(courseId: course, source: source) {
        // A ZIP Core lists as its files still opens whole in Quick Look here.
        case let .ready(file, _)?, let .archive(file, _, _)?:
            preview = URL(fileURLWithPath: file.path)
        case let .page(site, url)?:
            place.push(WebRouter.PushedPage(site: site, target: url, title: title))
        case let .external(url)?:
            if let url = URL(string: url) { place.openURL(url) }
        case let .tooLarge(bytes, page)?:
            fallback = page
            notice = t("lmsFile.thisFileIsTooLarge", ["size": ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)])
        case let .failed(.login, page)?:
            fallback = page
            notice = t("lmsFile.signInToTheLMS")
        case .failed(.network, let page)?, .failed(.timeout, let page)?, .failed(.cancelled, let page)?:
            fallback = page
            retrying = true
        case let .folder(_, page)?, let .missing(page)?:
            // A folder's files are listed on its LMS page.
            place.show(page, title: title)
        case let .failed(_, page)?:
            // What Core could not read (it logged why) still opens on its LMS page.
            if let page { place.show(page, title: title) } else { notice = t("lmsFile.thisCantBeShownHere") }
        case nil:
            notice = t("lmsFile.thisCantBeShownHere")
        }
    }

    func retry(at place: MaterialPlace) async {
        guard let last else { return }
        await open(last.course, last.source, title: title, at: place)
    }
}

extension View {
    /// Shows what `opening` brings up: Quick Look and its alerts.
    func materialOpening(_ opening: MaterialOpening) -> some View {
        modifier(MaterialOpeningPresentation(opening: opening))
    }
}

private struct MaterialOpeningPresentation: ViewModifier {
    @Bindable var opening: MaterialOpening
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push
    @Environment(\.openURL) private var openURL

    func body(content: Content) -> some View {
        let place = MaterialPlace(model: model, push: push, openURL: openURL)
        content
            .quickLookPreview($opening.preview)
            .alert(opening.notice ?? "", isPresented: Binding(get: { opening.notice != nil }, set: { if !$0 { opening.notice = nil } })) {
                if let page = opening.fallback {
                    Button(t("common.openInLMS")) { place.show(page, title: opening.title) }
                }
                Button(t("common.cancel"), role: .cancel) {}
            }
            .alert(t("lmsFile.theFileCouldNotBe"), isPresented: $opening.retrying) {
                Button(t("common.tryAgain")) { Task { await opening.retry(at: place) } }
                if let page = opening.fallback {
                    Button(t("common.openInLMS")) { place.show(page, title: opening.title) }
                }
                Button(t("common.cancel"), role: .cancel) {}
            }
    }
}
