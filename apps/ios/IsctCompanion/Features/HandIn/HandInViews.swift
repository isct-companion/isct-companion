import IsctCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// What waits to be handed in, as a stack of cards in the bottom-right
/// corner, like the screenshot thumbnail: the finger moves it at once. Drop it
/// on an assignment row or upload card (which floats while the stack is over
/// it) to put the files there and open that assignment; throw it to the trash
/// that appears top right to discard them; tap it for the upload panel. The
/// layer covers the page but only the stack takes touches.
struct InboxTray: View {
    /// Space kept free under the stack: the tab bar or the status bar.
    let bottom: CGFloat
    @Environment(InboxModel.self) private var inbox
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var drag: CGSize = .zero
    @State private var dragging = false
    @State private var overTrash = false
    @State private var discarded = 0

    private static let card = CGSize(width: 68, height: 88)

    var body: some View {
        GeometryReader { proxy in
            let frame = proxy.frame(in: .global)
            ZStack(alignment: .bottomTrailing) {
                Color.clear.allowsHitTesting(false)
                if dragging {
                    trash
                        .allowsHitTesting(false)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                        .padding(.top, 24)
                        .padding(.trailing, 24)
                        .transition(.scale.combined(with: .opacity))
                }
                if !inbox.items.isEmpty {
                    stack
                        .scaleEffect(dragging ? 0.9 : 1)
                        .offset(drag)
                        .padding(.trailing, 16)
                        .padding(.bottom, bottom)
                        .onTapGesture { inbox.presenting = true }
                        .gesture(gesture(in: frame))
                        .transition(.scale.combined(with: .opacity))
                        .accessibilityAddTraits(.isButton)
                        .accessibilityLabel(t("Shared files"))
                }
            }
            .animation(.snappy, value: dragging)
            .animation(.snappy, value: inbox.items.count)
        }
        .sensoryFeedback(.impact(weight: .medium), trigger: discarded)
        .sensoryFeedback(.selection, trigger: inbox.hovered)
    }

    /// The waiting files in order, each a card a little behind the next.
    private var stack: some View {
        let shown = Array(inbox.items.suffix(4))
        return ZStack(alignment: .bottomTrailing) {
            ForEach(Array(shown.enumerated()), id: \.element.id) { index, item in
                let depth = CGFloat(shown.count - 1 - index)
                if let url = item.url {
                    Thumbnail(url: url, corner: 10)
                        .frame(width: Self.card.width, height: Self.card.height)
                        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Theme.surface, lineWidth: 3))
                        .shadow(color: .black.opacity(0.22), radius: 6, y: 3)
                        .offset(x: -depth * 7, y: -depth * 7)
                }
            }
        }
        .padding([.top, .leading], CGFloat(max(shown.count - 1, 0)) * 7)
        .contentShape(Rectangle())
    }

    private var trash: some View {
        Image(systemName: overTrash ? "trash.fill" : "trash")
            .font(.title2.weight(.semibold))
            .foregroundStyle(overTrash ? Theme.accentInk : Theme.danger)
            .frame(width: 60, height: 60)
            .background(overTrash ? Theme.danger : Theme.surface, in: Circle())
            .shadow(color: .black.opacity(0.18), radius: 8, y: 3)
            .scaleEffect(overTrash ? 1.2 : 1)
            .symbolEffect(.bounce, value: discarded)
            .animation(.snappy, value: overTrash)
            .accessibilityHidden(true)
    }

    private func gesture(in frame: CGRect) -> some Gesture {
        DragGesture(minimumDistance: 4, coordinateSpace: .global)
            .onChanged { value in
                dragging = true
                drag = value.translation
                inbox.hover(at: value.location)
                // The trash sits in the window's top-right corner.
                let corner = CGRect(x: frame.maxX - 140, y: frame.minY, width: 140, height: 140)
                overTrash = inbox.hovered == nil && corner.contains(value.location)
            }
            .onEnded { _ in
                let target = inbox.hoveredTarget
                let trashed = overTrash
                inbox.hover(at: nil)
                overTrash = false
                if let target {
                    Task { await give(to: target) }
                } else if trashed {
                    discarded += 1
                    inbox.discardAll()
                }
                withAnimation(.snappy) {
                    drag = .zero
                    dragging = false
                }
            }
    }

    private func give(to target: InboxModel.Target) async {
        guard await inbox.give(inbox.items, to: target.assignment, model) else { return }
        if target.opens {
            navigator.tab = .today
            navigator.open(AssignmentRoute(id: target.assignment))
        }
    }
}

extension View {
    /// Makes the view a place to drop the corner stack for `assignment`; it
    /// floats while the stack is over it. `opens`: the drop opens the
    /// assignment (a row), or the page is already showing it (its card).
    func handInTarget(_ assignment: String, opens: Bool = true) -> some View {
        modifier(HandInTargetModifier(assignment: assignment, opens: opens))
    }
}

private struct HandInTargetModifier: ViewModifier {
    let assignment: String
    let opens: Bool
    @Environment(InboxModel.self) private var inbox
    @State private var token = UUID()

    func body(content: Content) -> some View {
        let over = inbox.hovered == token
        content
            .background {
                if over {
                    RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous)
                        .fill(Theme.accentSoft)
                }
            }
            .overlay {
                if over {
                    RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous)
                        .stroke(Theme.accent, lineWidth: 2)
                }
            }
            .scaleEffect(over ? 1.03 : 1)
            .shadow(color: .black.opacity(over ? 0.18 : 0), radius: 12, y: 4)
            .zIndex(over ? 1 : 0)
            .animation(.snappy, value: over)
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
                inbox.targets[token] = InboxModel.Target(assignment: assignment, frame: frame, opens: opens)
            }
            .onDisappear { inbox.targets[token] = nil }
    }
}

/// The upload panel: what waits in the inbox, then the quarter's unsubmitted
/// assignments as 今日 groups them. Picking one puts the files in its upload
/// card and opens its page, where 「LMS にアップロード」 is the next step.
struct InboxSheet: View {
    @Environment(InboxModel.self) private var inbox
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss
    @State private var giving = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 12) {
                            ForEach(inbox.items) { item in
                                VStack(spacing: 6) {
                                    if let url = item.url { Thumbnail(url: url).frame(width: 72, height: 92) }
                                    Text(item.name).font(.caption2).lineLimit(1).frame(width: 80)
                                }
                                .contextMenu {
                                    Button(t("Delete"), systemImage: "trash", role: .destructive) { inbox.remove(item) }
                                }
                            }
                        }
                        .padding(.vertical, 4)
                    }
                }
                if let groups = model.board?.today {
                    section(t("Pinned"), groups.pinned)
                    section(t("This week"), groups.dueThisWeek)
                    section(t("Next week and later"), groups.later)
                    section(t("No deadline"), groups.undated)
                    section(t("Past"), groups.pastRecent)
                }
            }
            .disabled(giving)
            .navigationTitle(t("Hand in to"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("Close")) { dismiss() } }
            }
            .onChange(of: inbox.items.isEmpty) { _, empty in if empty { dismiss() } }
        }
        .presentationDetents([.large])
    }

    @ViewBuilder
    private func section(_ title: String, _ items: [DeadlineItem]) -> some View {
        if !items.isEmpty {
            Section(title) {
                ForEach(items, id: \.assignmentId) { item in
                    Button { give(to: item.assignmentId) } label: { DeadlineRow(item: item, now: Date()) }
                        .buttonStyle(.plain)
                }
            }
        }
    }

    private func give(to assignment: String) {
        giving = true
        Task {
            defer { giving = false }
            if await inbox.give(inbox.items, to: assignment, model) {
                dismiss()
                navigator.tab = .today
                navigator.open(AssignmentRoute(id: assignment))
            }
        }
    }
}

/// The assignment page's one place to hand in (slotledger `approach/share-entry-points`):
/// a card to drop files on (from the inbox stack or another app) or to tap to
/// choose files or photos. What it receives joins the assignment's `submit/`,
/// listed in the card; Upload fills the LMS draft area with it and shows that
/// page, where the person reviews and saves.
struct HandInCard: View {
    let assignment: AssignmentDetail
    @Environment(AppModel.self) private var model
    @Environment(InboxModel.self) private var inbox
    @Environment(\.push) private var push
    @State private var files: [LocalFile] = []
    @State private var choosing = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var targeted = false
    @State private var working = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(files, id: \.path) { file in
                HStack(spacing: 12) {
                    Thumbnail(url: URL(fileURLWithPath: file.path)).frame(width: 32, height: 40)
                    Text(file.name).foregroundStyle(Theme.ink).lineLimit(2)
                    Spacer(minLength: 8)
                    Button(t("Delete"), systemImage: "minus.circle") { remove(file) }
                        .labelStyle(.iconOnly)
                        .foregroundStyle(Theme.danger)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                RowDivider(inset: 16)
            }
            Button {
                choosing = true
            } label: {
                VStack(spacing: 6) {
                    Image(systemName: "arrow.up.doc").font(.title2).foregroundStyle(Theme.accent)
                    Text(t("Drop files here, or tap to choose")).font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
                    Text(t("Up to 32 files, 20 MB in all")).font(.footnote).foregroundStyle(Theme.muted)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 18)
                .contentShape(Rectangle())
            }
            .buttonStyle(PressableStyle())
            HStack {
                PhotosPicker(selection: $photos, maxSelectionCount: 32, matching: .images) {
                    Label(t("Photos"), systemImage: "photo.on.rectangle").font(.subheadline.weight(.semibold))
                }
                Spacer()
                if !files.isEmpty {
                    Button {
                        Task { await handIn() }
                    } label: {
                        Label(t("Upload to LMS"), systemImage: "arrow.up.forward.square")
                            .font(.subheadline.weight(.semibold))
                            .padding(.horizontal, 14)
                            .padding(.vertical, 8)
                            .foregroundStyle(Theme.accentInk)
                            .background(Theme.accent, in: Capsule())
                    }
                    .buttonStyle(PressableStyle())
                    .disabled(working)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 12)
        }
        .background(targeted ? Theme.accentSoft : Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous)
                .stroke(targeted ? Theme.accent : Theme.line, style: StrokeStyle(lineWidth: targeted ? 2 : 1, dash: [6, 4]))
                .allowsHitTesting(false)
        }
        .handInDrop(assignment.assignmentId, targeted: $targeted) { files = $0 }
        .handInTarget(assignment.assignmentId, opens: false)
        .fileImporter(isPresented: $choosing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case let .success(urls) = result else { return }
            Task { await add(urls) }
        }
        .onChange(of: photos) { _, picked in
            guard !picked.isEmpty else { return }
            photos = []
            Task { await add(picked) }
        }
        .task(id: "\(assignment.assignmentId)-\(inbox.items.count)") {
            files = await model.submitFiles(assignment.assignmentId)
        }
        .animation(.snappy, value: files.map(\.path))
        .alert(error ?? "", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        }
    }

    /// Files chosen in the Files picker, read while their access lasts.
    private func add(_ urls: [URL]) async {
        let opened = urls.filter { $0.startAccessingSecurityScopedResource() }
        defer { opened.forEach { $0.stopAccessingSecurityScopedResource() } }
        await add(urls.map { SharedFile(path: $0.path, name: $0.lastPathComponent) })
    }

    /// Photos, written out as files first.
    private func add(_ items: [PhotosPickerItem]) async {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        var shared: [SharedFile] = []
        for (index, item) in items.enumerated() {
            guard let data = try? await item.loadTransferable(type: Data.self) else { continue }
            let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
            let url = folder.appendingPathComponent("photo-\(index + 1).\(ext)")
            guard (try? data.write(to: url)) != nil else { continue }
            shared.append(SharedFile(path: url.path, name: url.lastPathComponent))
        }
        await add(shared)
    }

    private func add(_ shared: [SharedFile]) async {
        guard !shared.isEmpty else { return }
        do { files = try await model.addSubmitFiles(assignment.assignmentId, shared) }
        catch { self.error = AppModel.describe(error) }
    }

    private func remove(_ file: LocalFile) {
        Task {
            do { files = try await model.removeSubmitFile(assignment.assignmentId, name: file.name) }
            catch { self.error = AppModel.describe(error) }
        }
    }

    private func handIn() async {
        working = true
        defer { working = false }
        do {
            let url = try await model.handIn(assignment.assignmentId)
            push(WebRouter.PushedPage(site: .scienceLms, target: url, title: assignment.title))
        } catch {
            self.error = AppModel.describe(error)
        }
    }
}

private extension View {
    /// The card's drop: files from another app join the assignment's
    /// `submit/`; `listed` receives what is there afterwards. The inbox stack
    /// lands through `InboxModel.targets` instead.
    func handInDrop(_ assignment: String, targeted: Binding<Bool>, listed: @escaping ([LocalFile]) -> Void) -> some View {
        modifier(HandInDrop(assignment: assignment, targeted: targeted, listed: listed))
    }
}

private struct HandInDrop: ViewModifier {
    let assignment: String
    @Binding var targeted: Bool
    let listed: ([LocalFile]) -> Void
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content
            .onDrop(of: [.data], isTargeted: $targeted) { providers in
                Task { await receive(providers) }
                return true
            }
    }

    private func receive(_ providers: [NSItemProvider]) async {
        // Files from another app: copied out of each provider while it lasts.
        var files: [SharedFile] = []
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        for provider in providers {
            if let file = await Self.file(provider, into: folder) { files.append(file) }
        }
        guard !files.isEmpty, let added = try? await model.addSubmitFiles(assignment, files) else { return }
        listed(added)
    }

    private static func file(_ provider: NSItemProvider, into folder: URL) async -> SharedFile? {
        guard let type = provider.registeredTypeIdentifiers.compactMap(UTType.init)
            .first(where: { $0.conforms(to: .data) && !$0.conforms(to: .plainText) })
        else { return nil }
        let suggested = provider.suggestedName
        return await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                guard let url else { return continuation.resume(returning: nil) }
                let name = suggested.map { name in
                    url.pathExtension.isEmpty || name.hasSuffix(".\(url.pathExtension)") ? name : "\(name).\(url.pathExtension)"
                } ?? url.lastPathComponent
                let copy = folder.appendingPathComponent(UUID().uuidString).appendingPathExtension(url.pathExtension)
                guard (try? FileManager.default.copyItem(at: url, to: copy)) != nil else {
                    return continuation.resume(returning: nil)
                }
                continuation.resume(returning: SharedFile(path: copy.path, name: name))
            }
        }
    }
}
