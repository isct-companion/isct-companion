import IsctCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

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
                HStack(spacing: Theme.rowGap) {
                    Thumbnail(url: URL(fileURLWithPath: file.path)).frame(width: 32, height: 40)
                    Text(file.name).foregroundStyle(Theme.ink).lineLimit(2)
                    Spacer(minLength: Theme.space4)
                    Button(t("common.delete"), systemImage: "minus.circle") { remove(file) }
                        .labelStyle(.iconOnly)
                        .foregroundStyle(Theme.danger)
                }
                .padding(.horizontal, Theme.rowX)
                .padding(.vertical, Theme.space4)
                RowDivider(inset: Theme.rowX)
            }
            Button {
                choosing = true
            } label: {
                VStack(spacing: Theme.space3) {
                    Image(systemName: "arrow.up.doc").font(.title2).foregroundStyle(Theme.accent)
                    Text(t("handIn.dropFilesHereOrTap")).font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
                    Text(t("handIn.upTo32Files20")).font(.footnote).foregroundStyle(Theme.muted)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 18)
                .contentShape(Rectangle())
            }
            .buttonStyle(PressableStyle())
            HStack {
                PhotosPicker(selection: $photos, maxSelectionCount: 32, matching: .images) {
                    Label(t("handIn.photos"), systemImage: "photo.on.rectangle").font(.subheadline.weight(.semibold))
                }
                Spacer()
                if !files.isEmpty {
                    Button {
                        Task { await handIn() }
                    } label: {
                        Label(t("handIn.uploadToLMS"), systemImage: "arrow.up.forward.square")
                            .font(.subheadline.weight(.semibold))
                            .padding(.horizontal, Theme.space7)
                            .padding(.vertical, Theme.space4)
                    }
                    .buttonStyle(CapsuleButtonStyle())
                    .disabled(working)
                }
            }
            .padding(.horizontal, Theme.space8)
            .padding(.bottom, Theme.space6)
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
        .animation(.smooth(duration: Theme.motionBase), value: files.map(\.path))
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
