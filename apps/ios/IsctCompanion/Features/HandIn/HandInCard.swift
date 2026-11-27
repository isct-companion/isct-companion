import IsctCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// The assignment page starts a new file selection for its LMS draft.
struct HandInCard: View {
    let assignment: AssignmentDetail
    @Environment(InboxModel.self) private var inbox
    @State private var choosing = false
    @State private var photos: [PhotosPickerItem] = []

    private var choice: SharedInbox.Choice {
        SharedInbox.Choice(id: assignment.assignmentId, title: assignment.title, course: nil, due: "")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                choosing = true
            } label: {
                VStack(spacing: Theme.space3) {
                    Image(systemName: "arrow.up.doc").font(.title2).foregroundStyle(Theme.accent)
                    Text(t("handIn.tapToChoose")).font(.subheadline.weight(.semibold)).foregroundStyle(Theme.ink)
                    Text(t("handIn.upTo32Files20")).font(.footnote).foregroundStyle(Theme.muted)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, Theme.space8)
                .contentShape(Rectangle())
            }
            .buttonStyle(PressableStyle())
            PhotosPicker(selection: $photos, maxSelectionCount: 32, matching: .images) {
                Label(t("handIn.photos"), systemImage: "photo.on.rectangle")
                    .font(.subheadline.weight(.semibold))
            }
            .padding(.horizontal, Theme.space8)
            .padding(.bottom, Theme.space6)
        }
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface))
        .handInTarget(assignment: assignment.assignmentId, title: assignment.title, availability: assignment.handInAvailability)
        .fileImporter(isPresented: $choosing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            switch result {
            case let .success(urls):
                if !urls.isEmpty { Task { await inbox.receive(urls, choice: choice) } }
            case let .failure(error):
                inbox.error = AppModel.describe(error)
            }
        }
        .onChange(of: photos) { _, picked in
            guard !picked.isEmpty else { return }
            photos = []
            Task { await copyPhotos(picked) }
        }
    }

    private func copyPhotos(_ items: [PhotosPickerItem]) async {
        await inbox.select(choice) { folder in
            var files: [SharedInbox.File] = []
            for (index, item) in items.enumerated() {
                guard let data = try await item.loadTransferable(type: Data.self) else {
                    throw CocoaError(.fileReadUnknown)
                }
                let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
                let file = SharedInbox.newFile(name: "photo-\(index + 1).\(ext)")
                try data.write(to: file.url(in: folder))
                files.append(file)
            }
            return files
        }
    }
}
