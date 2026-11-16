import Foundation
import ImageIO
import IsctCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

enum ExamUploadEntry: Identifiable {
    case subject(ExamSubject)
    case request(ExamRequest)
    case recovery

    var id: String {
        switch self {
        case let .subject(subject): "subject-\(subject.code)"
        case let .request(request): "request-\(request.id)"
        case .recovery: "recovery"
        }
    }

    /// Where Core starts the form; a recovery has none.
    var start: ExamUploadStart? {
        switch self {
        case let .subject(subject): .subject(code: subject.code)
        case let .request(request): .request(request: request)
        case .recovery: nil
        }
    }

    var title: String {
        switch self {
        case .subject: t("examShare.upload.title")
        case .request: t("examShare.upload.answerRequest")
        case .recovery: t("examShare.upload.recover")
        }
    }
}

/// Renders Core's upload form. Fields without rules are edited in
/// `form.draft`; subjects and the role go back to Core (`EditUpload`), which
/// answers the consistent form. Progress follows the account Core sends.
struct ExamUploadSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    let entry: ExamUploadEntry
    let choices: ExamChoices
    /// An upload or recovery finished; `published` when materials went up.
    let onFinished: (_ published: Bool) async -> Void

    @State private var form: ExamUploadForm?
    @State private var newTeachers: [TeacherEntry] = []
    @State private var year = ""
    @State private var selection: ExamUploadSelection?
    @State private var files: [ExamUploadSelection.File] = []
    @State private var problem: String?
    @State private var choosingFiles = false
    @State private var choosingPhotos = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var choosingSubject = false
    @State private var loading = true
    @State private var editing = false
    @State private var staging = false
    @State private var running = false
    @State private var cancelling = false

    private var upload: ExamUploadStatus? { model.examAccount?.upload }

    var body: some View {
        NavigationStack {
            Form {
                if let problem { Section { Text(problem).foregroundStyle(.red) } }
                if let stageLabel {
                    Section {
                        if let upload, upload.phase == .transferring, upload.total > 0 {
                            ProgressView(stageLabel, value: Double(upload.sent), total: Double(upload.total))
                        } else {
                            ProgressView(stageLabel)
                        }
                    }
                }
                if upload?.phase == .pending {
                    Section {
                        Text(t("examShare.upload.pending"))
                        Button(t("examShare.upload.recover")) { Task { await perform(.recoverUpload) } }
                            .disabled(running)
                    }
                }
                if let form = Binding($form) {
                    fileFields.disabled(formDisabled)
                    classificationFields(form.draft).disabled(formDisabled)
                    subjectFields(form.wrappedValue).disabled(formDisabled)
                    teacherFields(form).disabled(formDisabled)
                }
            }
            .navigationTitle(entry.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    if running {
                        Button(t("common.cancel")) { Task { await cancel() } }
                            .disabled(cancelling || upload?.cancellable != true)
                    } else {
                        Button(t("common.close")) { dismiss() }.disabled(staging)
                    }
                }
                if form != nil {
                    ToolbarItem(placement: .confirmationAction) {
                        Button(t("common.submit")) { Task { await submit() } }.disabled(!canSubmit)
                    }
                }
            }
            .interactiveDismissDisabled(running || staging)
            .fileImporter(isPresented: $choosingFiles, allowedContentTypes: ExamUploadSelection.types, allowsMultipleSelection: true) {
                switch $0 {
                case let .success(urls) where !urls.isEmpty:
                    Task { await stage(urls) }
                case .success:
                    break
                case .failure:
                    problem = t("examShare.upload.failedFiles")
                }
            }
            .photosPicker(isPresented: $choosingPhotos, selection: $photos, matching: .images)
            .onChange(of: photos) { _, picked in
                guard !picked.isEmpty else { return }
                photos = []
                Task { await stage(picked) }
            }
            .sheet(isPresented: $choosingSubject) {
                ExamUploadSubjectPicker(selected: Set(form?.draft.subjectCodes ?? [])) { subject in
                    try await edit(.addSubject(code: subject.code))
                }
            }
            .task { await load() }
        }
    }

    private var formDisabled: Bool {
        running || staging || cancelling || editing || upload?.phase == .pending
    }

    private var canSubmit: Bool {
        guard let draft = form?.draft else { return false }
        let trimmedYear = year.trimmingCharacters(in: .whitespacesAndNewlines)
        return !formDisabled && !files.isEmpty && (trimmedYear.isEmpty || UInt32(trimmedYear) != nil)
            && newTeachers.allSatisfy { !$0.familyName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            && draft.variant.utf16.count <= choices.limits.variant
            && draft.note.utf16.count <= choices.limits.note
    }

    private var stageLabel: String? {
        if loading || staging { return t("examShare.upload.preparing") }
        guard running else { return nil }
        switch upload?.phase {
        case .transferring: return t("examShare.upload.transferring")
        case .registering: return t("examShare.upload.registering")
        case .cleaning: return t("examShare.upload.cleaning")
        default: return t("examShare.upload.preparing")
        }
    }

    private var fileFields: some View {
        Section {
            // Photos come from the library, documents from Files, as in Mail.
            Menu {
                Button(t("examShare.upload.photoLibrary"), systemImage: "photo.on.rectangle") { choosingPhotos = true }
                Button(t("examShare.upload.browseFiles"), systemImage: "folder") { choosingFiles = true }
            } label: {
                Label(t("examShare.upload.chooseFiles"), systemImage: "plus")
            }
            if files.count > 1 { Text(t("examShare.upload.fileOrder")).foregroundStyle(Theme.muted) }
            ForEach(Array(files.enumerated()), id: \.element.url) { index, file in
                HStack {
                    Text(file.name).lineLimit(2)
                    Spacer()
                    if files.count > 1 {
                        fileButton(t("examShare.upload.moveUp"), icon: "arrow.up") { moveFile(file.url, by: -1) }
                            .disabled(index == 0)
                        fileButton(t("examShare.upload.moveDown"), icon: "arrow.down") { moveFile(file.url, by: 1) }
                            .disabled(index == files.count - 1)
                    }
                    fileButton(t("common.remove"), icon: "minus.circle") { files.removeAll { $0.url == file.url } }
                }
            }
        }
    }

    private func fileButton(_ title: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(title, systemImage: icon, action: action)
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
    }

    private func moveFile(_ url: URL, by offset: Int) {
        guard let index = files.firstIndex(where: { $0.url == url }), files.indices.contains(index + offset) else { return }
        files.swapAt(index, index + offset)
    }

    private func classificationFields(_ draft: Binding<ExamUploadDraft>) -> some View {
        let allowed = choices.roles.first { $0.role == draft.wrappedValue.role }
        let occasions = allowed?.occasions ?? []
        let sources = allowed?.answerSources ?? []
        return Section {
            Picker(t("examShare.upload.role"), selection: Binding(
                get: { draft.wrappedValue.role },
                set: { role in Task { await change(.role(role: role)) } }
            )) {
                ForEach(choices.roles, id: \.role) { Text($0.role.label).tag($0.role) }
            }
            TextField(t("examShare.year"), text: $year).keyboardType(.numberPad)
            // A role with one occasion (notes, other) has nothing to choose.
            if occasions.count != 1 {
                Picker(t("examShare.occasion"), selection: draft.occasion) {
                    ForEach(occasions, id: \.self) { Text($0.label).tag($0) }
                }
            }
            if choices.numbered.contains(draft.wrappedValue.occasion) {
                Stepper(value: draft.occasionNumber, in: 0...choices.limits.occasionNumber) {
                    Text("\(t("examShare.upload.occasionNumber")) · \(draft.wrappedValue.occasionNumber)")
                }
            }
            TextField(t("examShare.upload.variant"), text: draft.variant)
            Picker(t("examShare.upload.provenance"), selection: draft.provenance) {
                ForEach(choices.provenances, id: \.self) { Text($0.label).tag($0) }
            }
            if !sources.isEmpty {
                Picker(t("examShare.upload.answerSource"), selection: draft.answerSource) {
                    ForEach(sources, id: \.self) { Text($0.label).tag(Optional($0)) }
                }
            }
            TextField(t("examShare.upload.note"), text: draft.note, axis: .vertical)
        }
    }

    private func subjectFields(_ form: ExamUploadForm) -> some View {
        Section(t("examShare.upload.subjects")) {
            ForEach(form.subjects, id: \.subject.code) { entry in
                let code = entry.subject.code
                HStack {
                    VStack(alignment: .leading) {
                        Text(entry.subject.name)
                        Text(code).font(.caption.monospaced()).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    if entry.removable {
                        fileButton(t("common.remove"), icon: "minus.circle") {
                            Task { await change(.removeSubject(code: code)) }
                        }
                    }
                }
            }
            Button(t("examShare.upload.addSubject"), systemImage: "plus") { choosingSubject = true }
                .disabled(form.subjects.count >= Int(choices.limits.subjects))
        }
    }

    private func teacherFields(_ form: Binding<ExamUploadForm>) -> some View {
        let teacherIds = form.draft.teacherIds
        let limit = Int(choices.limits.teachers)
        return Section(t("examShare.teacher")) {
            ForEach(form.wrappedValue.teachers, id: \.id) { teacher in
                Toggle(teacher.name, isOn: Binding(
                    get: { teacherIds.wrappedValue.contains(teacher.id) },
                    set: { selected in
                        if selected {
                            teacherIds.wrappedValue.append(teacher.id)
                        } else {
                            teacherIds.wrappedValue.removeAll { $0 == teacher.id }
                        }
                    }
                ))
                .disabled(teacherIds.wrappedValue.count >= limit && !teacherIds.wrappedValue.contains(teacher.id))
            }
            ForEach($newTeachers) { $teacher in
                VStack(alignment: .leading, spacing: 8) {
                    TextField(t("examShare.upload.familyName"), text: $teacher.familyName)
                    TextField(t("examShare.upload.givenName"), text: $teacher.givenName)
                    Button(t("common.remove"), role: .destructive) {
                        let id = teacher.id
                        newTeachers.removeAll { $0.id == id }
                    }.buttonStyle(.borderless)
                }
            }
            Button(t("examShare.upload.newTeacher"), systemImage: "plus") { newTeachers.append(TeacherEntry()) }
                .disabled(newTeachers.count >= limit)
        }
    }

    private func load() async {
        defer { loading = false }
        guard let start = entry.start else { return }
        do {
            guard case let .uploadForm(value) = try await model.examShare(.uploadForm(start: start)) else { return }
            form = value
            year = value.draft.academicYear.map(String.init) ?? ""
            newTeachers = value.draft.newTeachers.map { TeacherEntry(familyName: $0.familyName, givenName: $0.givenName) }
        } catch { problem = AppModel.describe(error) }
    }

    /// Sends one rule-bearing edit to Core and shows the form it answers.
    private func edit(_ change: ExamUploadEdit) async throws {
        guard let current = form else { return }
        editing = true
        defer { editing = false }
        if case let .uploadForm(value) = try await model.examShare(.editUpload(form: current, edit: change)) {
            form = value
        }
    }

    private func change(_ edit: ExamUploadEdit) async {
        problem = nil
        do { try await self.edit(edit) }
        catch { problem = AppModel.describe(error) }
    }

    private func stage(_ urls: [URL]) async {
        guard !staging else { return }
        staging = true
        problem = nil
        defer { staging = false }
        do {
            let staged = try await Task.detached(priority: .userInitiated) {
                try ExamUploadSelection.copy(urls)
            }.value
            selection = staged
            files = staged.files
        } catch { problem = t("examShare.upload.failedFiles") }
    }

    /// Photos from the library, written out as files and staged like files
    /// (HEIC becomes JPEG there).
    private func stage(_ items: [PhotosPickerItem]) async {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        var urls: [URL] = []
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            for (index, item) in items.enumerated() {
                guard let data = try await item.loadTransferable(type: Data.self) else { throw CocoaError(.fileReadUnknown) }
                let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
                let url = folder.appendingPathComponent("photo-\(index + 1).\(ext)")
                try data.write(to: url)
                urls.append(url)
            }
        } catch {
            problem = t("examShare.upload.failedFiles")
            return
        }
        await stage(urls)
    }

    private func submit() async {
        guard canSubmit, var draft = form?.draft, let selection else { return }
        draft.academicYear = UInt32(year.trimmingCharacters(in: .whitespacesAndNewlines))
        draft.newTeachers = newTeachers.map {
            ExamNewTeacher(familyName: $0.familyName, givenName: $0.givenName)
        }
        let input = files.map { ExamUploadFile(path: $0.url.path, name: $0.name) }
        // The staging directory is kept alive through the native operation.
        await perform(.upload(files: input, draft: draft))
        withExtendedLifetime(selection) {}
    }

    private func perform(_ action: ExamAction) async {
        guard !running else { return }
        running = true
        problem = nil
        defer {
            running = false
            cancelling = false
        }
        do {
            switch try await model.examShare(action) {
            case .uploaded:
                await onFinished(true)
                dismiss()
            case .done:
                await onFinished(false)
                if entry.start == nil { dismiss() }
            default: break
            }
        } catch SdkError.Cancelled {}
        catch is CancellationError {}
        catch { problem = AppModel.describe(error) }
    }

    private func cancel() async {
        guard !cancelling, upload?.cancellable == true else { return }
        cancelling = true
        do { _ = try await model.examShare(.cancelUpload) }
        catch {
            problem = AppModel.describe(error)
            cancelling = false
        }
    }

    private struct TeacherEntry: Identifiable {
        let id = UUID()
        var familyName = ""
        var givenName = ""
    }
}

private struct ExamUploadSubjectPicker: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let selected: Set<String>
    let pick: (ExamSubject) async throws -> Void
    @State private var query = ""
    @State private var results: [ExamSubject]?
    @State private var problem: String?
    @State private var busy = false

    var body: some View {
        NavigationStack {
            List {
                if let problem { Text(problem).foregroundStyle(.red) }
                if busy { ProgressView() }
                if results?.isEmpty == true { Text(t("examShare.noItems")) }
                ForEach(results ?? [], id: \.code) { subject in
                    Button {
                        Task {
                            await perform {
                                try await pick(subject)
                                dismiss()
                            }
                        }
                    } label: {
                        VStack(alignment: .leading) {
                            Text(subject.name)
                            Text(subject.code).font(.caption.monospaced()).foregroundStyle(Theme.muted)
                        }
                    }
                    .disabled(selected.contains(subject.code))
                }
            }
            .searchable(text: $query, prompt: t("examShare.search"))
            .onSubmit(of: .search) {
                Task { await search() }
            }
            .navigationTitle(t("examShare.upload.addSubject"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.close")) { dismiss() } }
            }
            .disabled(busy)
        }
    }

    private func search() async {
        await perform {
            if case let .subjects(subjects) = try await model.examShare(.search(query: query)) { results = subjects }
        }
    }

    private func perform(_ action: () async throws -> Void) async {
        guard !busy else { return }
        busy = true
        problem = nil
        defer { busy = false }
        do { try await action() }
        catch { problem = AppModel.describe(error) }
    }
}

private struct ExamUploadSelection: Sendable {
    struct File: Sendable {
        let url: URL
        let name: String
    }

    // SwiftUI retains the directory while its staged selection is in the form.
    private final class Directory: Sendable {
        let url: URL
        init(_ url: URL) { self.url = url }
        deinit { try? FileManager.default.removeItem(at: url) }
    }

    static let types: [UTType] = [.pdf, .plainText, .jpeg, .png, .webP, .heic, .heif]

    private let directory: Directory
    let files: [File]

    static func copy(_ urls: [URL]) throws -> Self {
        let directory = Directory(FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true))
        try FileManager.default.createDirectory(at: directory.url, withIntermediateDirectories: false)
        let files = try urls.map { try copy($0, into: directory.url) }
        return Self(directory: directory, files: files)
    }

    private static func copy(_ source: URL, into folder: URL) throws -> File {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        var coordinationError: NSError?
        var outcome: Result<File, Error>?
        NSFileCoordinator().coordinate(readingItemAt: source, options: [], error: &coordinationError) { url in
            outcome = Result {
                let type = try url.resourceValues(forKeys: [.contentTypeKey]).contentType
                    ?? UTType(filenameExtension: source.pathExtension)
                guard let type, types.contains(where: { type.conforms(to: $0) })
                else { throw CocoaError(.fileReadCorruptFile) }
                let needsJPEG = type.conforms(to: .heic) || type.conforms(to: .heif)
                let targetType = needsJPEG ? UTType.jpeg : type
                guard let suffix = targetType.preferredFilenameExtension else { throw CocoaError(.fileReadCorruptFile) }
                let target = folder.appendingPathComponent(UUID().uuidString).appendingPathExtension(suffix)
                if needsJPEG {
                    try transcodeImage(url, into: target)
                } else {
                    try FileManager.default.copyItem(at: url, to: target)
                }
                let name = needsJPEG ? source.deletingPathExtension().lastPathComponent + ".jpg" : source.lastPathComponent
                return File(url: target, name: name)
            }
        }
        if let coordinationError { throw coordinationError }
        guard let outcome else { throw CocoaError(.fileReadUnknown) }
        return try outcome.get()
    }

    private static func transcodeImage(_ source: URL, into target: URL) throws {
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 4000,
        ]
        guard let imageSource = CGImageSourceCreateWithURL(source as CFURL, nil),
              let image = CGImageSourceCreateThumbnailAtIndex(imageSource, 0, options as CFDictionary),
              let destination = CGImageDestinationCreateWithURL(target as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw CocoaError(.fileReadCorruptFile) }
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw CocoaError(.fileWriteUnknown) }
    }
}
