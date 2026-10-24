import QuickLookThumbnailing
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The share sheet's entry: whatever was shared (files, photos, PDFs) goes to
/// the app's inbox, optionally for an assignment picked here. The extension
/// never loads Core; it reads the list of open assignments the app last wrote
/// (`SharedInbox.Choices`) and leaves the hand-in to the app.
final class ShareViewController: UIViewController {
    private let state = ShareState()

    override func viewDidLoad() {
        super.viewDidLoad()
        let choices = SharedInbox.readChoices()
        Localizer.shared.use(choices?.language)
        state.choices = choices?.assignments ?? []
        let host = UIHostingController(rootView: ShareView(state: state, done: { [weak self] save, open in
            self?.finish(save: save, open: open)
        }))
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
        Task { await load() }
    }

    private func load() async {
        state.files = await ExtensionInput.files(extensionContext)
        state.loading = false
    }

    private func finish(save: Bool, open: Bool = false) {
        if save {
            for file in state.files {
                _ = try? SharedInbox.add(file.url, name: file.name, assignmentId: state.assignment)
            }
        }
        if open { ExtensionInput.openApp(from: self) }
        extensionContext?.completeRequest(returningItems: nil)
    }
}

@MainActor
@Observable
final class ShareState {
    var files: [SharedFile] = []
    var loading = true
    var choices: [SharedInbox.Choice] = []
    /// `nil`: decide in the app.
    var assignment: String?
}

private struct ShareView: View {
    @Bindable var state: ShareState
    let done: (_ save: Bool, _ open: Bool) -> Void

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 12) {
                            ForEach(state.files) { file in
                                VStack(spacing: 6) {
                                    Thumbnail(url: file.url).frame(width: 72, height: 92)
                                    Text(file.name).font(.caption2).lineLimit(1).frame(width: 80)
                                }
                            }
                            if state.loading { ProgressView().frame(width: 72, height: 92) }
                        }
                        .padding(.vertical, 4)
                    }
                }
                // Nothing is submitted from here or by itself: the person
                // opens the app, checks the files and uploads them.
                Section {
                    Label(t("IsctCompanion never submits for you. Open the app to check the files and upload them yourself."), systemImage: "hand.raised")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button {
                        done(true, true)
                    } label: {
                        Label(t("Add and open IsctCompanion"), systemImage: "arrow.up.forward.app")
                            .font(.body.weight(.semibold))
                    }
                    .disabled(state.loading || state.files.isEmpty)
                }
                Section(t("Hand in to")) {
                    choiceRow(nil, title: t("Choose in the app"), detail: nil)
                    ForEach(state.choices) { choice in
                        choiceRow(choice.id, title: choice.title,
                            detail: [choice.course, choice.due].compactMap { $0 }.joined(separator: " · "))
                    }
                }
            }
            .navigationTitle("IsctCompanion")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(t("Cancel")) { done(false, false) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("Add")) { done(true, false) }
                        .disabled(state.loading || state.files.isEmpty)
                }
            }
        }
    }

    private func choiceRow(_ id: String?, title: String, detail: String?) -> some View {
        Button {
            state.assignment = id
        } label: {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).foregroundStyle(.primary)
                    if let detail, !detail.isEmpty {
                        Text(detail).font(.footnote).foregroundStyle(.secondary)
                    }
                }
                Spacer()
                if state.assignment == id {
                    Image(systemName: "checkmark").foregroundStyle(.tint)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
