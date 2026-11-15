import Foundation
import ImageIO
import IsctCore
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

    var subject: ExamSubject? {
        switch self {
        case let .subject(subject): subject
        case let .request(request): request.subject
        case .recovery: nil
        }
    }

    var request: ExamRequest? {
        if case let .request(request) = self { return request }
        return nil
    }

    var title: String {
        switch self {
        case .subject: t("examShare.upload.title")
        case .request: t("examShare.upload.answerRequest")
        case .recovery: t("examShare.upload.recover")
        }
    }
}

struct ExamUploadSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    let entry: ExamUploadEntry
    let subjectName: (ExamSubject) -> String
    let onUploaded: () async -> Void

    @State private var draft: ExamUploadDraft?
    @State private var subjectNames: [String: String] = [:]
    @State private var subjectTeachers: [String: [ExamTeacher]] = [:]
    @State private var newTeachers: [TeacherEntry] = []
    @State private var year = ""
    @State private var selection: ExamUploadSelection?
    @State private var files: [ExamUploadSelection.File] = []
    @State private var status: ExamUploadStatus?
    @State private var problem: String?
    @State private var choosingFiles = false
    @State private var choosingSubject = false
    @State private var loading = true
    @State private var staging = false
    @State private var running = false
    @State private var cancelling = false

    private static let roles = ["question", "answer", "both", "notes", "other"]
    private static let occasions = ["final", "midterm", "makeup", "quiz", "check", "report", "exercise", "textbook", "mock", "unknown"]

    var body: some View {
        NavigationStack {
            Form {
                if let problem { Section { Text(problem).foregroundStyle(.red) } }
                if let stageLabel {
                    Section {
                        if let status, status.phase == .transferring, status.total > 0 {
                            ProgressView(stageLabel, value: Double(status.sent), total: Double(status.total))
                        } else {
                            ProgressView(stageLabel)
                        }
                    }
                }
                if status?.phase == .pending {
                    Section {
                        Text(t("examShare.upload.pending"))
                        Button(t("examShare.upload.recover")) { Task { await perform(.recoverUpload) } }
                            .disabled(running)
                    }
                }
                if let draft = Binding($draft) {
                    fileFields.disabled(formDisabled)
                    classificationFields(draft).disabled(formDisabled)
                    subjectFields(draft).disabled(formDisabled)
                    teacherFields(draft).disabled(formDisabled)
                }
            }
            .navigationTitle(entry.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    if running {
                        Button(t("common.cancel")) { Task { await cancel() } }
                            .disabled(cancelling || !canCancel)
                    } else {
                        Button(t("common.close")) { dismiss() }.disabled(staging)
                    }
                }
                if draft != nil {
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
            .sheet(isPresented: $choosingSubject) {
                ExamUploadSubjectPicker(subjectName: subjectName, selected: Set(draft?.subjectCodes ?? [])) { subject in
                    try await addSubject(subject)
                }
            }
            .task { await load() }
            .task(id: running) {
                guard running else { return }
                while !Task.isCancelled {
                    await refreshStatus()
                    do { try await Task.sleep(for: .milliseconds(250)) }
                    catch { return }
                }
            }
        }
    }

    private var formDisabled: Bool {
        running || staging || cancelling || status?.phase == .pending
    }

    private var canSubmit: Bool {
        let trimmedYear = year.trimmingCharacters(in: .whitespacesAndNewlines)
        return !formDisabled && !files.isEmpty && (trimmedYear.isEmpty || UInt32(trimmedYear) != nil)
            && newTeachers.allSatisfy { !$0.familyName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private var canCancel: Bool {
        guard let phase = status?.phase else { return false }
        return phase == .preparing || phase == .transferring
    }

    private var stageLabel: String? {
        if loading || staging { return t("examShare.upload.preparing") }
        guard running else { return nil }
        switch status?.phase {
        case .transferring: return t("examShare.upload.transferring")
        case .registering: return t("examShare.upload.registering")
        case .cleaning: return t("examShare.upload.cleaning")
        default: return t("examShare.upload.preparing")
        }
    }

    private var fileFields: some View {
        Section {
            Button(t("examShare.upload.chooseFiles"), systemImage: "folder") { choosingFiles = true }
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
        let role = draft.wrappedValue.role
        let isExam = role != "notes" && role != "other"
        return Section {
            Picker(t("examShare.upload.role"), selection: draft.role) {
                ForEach(Self.roles, id: \.self) { Text(t("examShare.role.\($0)")).tag($0) }
            }
            TextField(t("examShare.year"), text: $year).keyboardType(.numberPad)
            if isExam {
                Picker(t("examShare.occasion"), selection: draft.occasion) {
                    ForEach(Self.occasions, id: \.self) { Text(t("examShare.occasion.\($0)")).tag($0) }
                }
                if ["quiz", "check", "report"].contains(draft.wrappedValue.occasion) {
                    Stepper(value: draft.occasionNumber, in: 0...30) {
                        Text("\(t("examShare.upload.occasionNumber")) · \(draft.wrappedValue.occasionNumber)")
                    }
                }
            }
            TextField(t("examShare.upload.variant"), text: draft.variant)
            Picker(t("examShare.upload.provenance"), selection: draft.provenance) {
                Text(t("examShare.upload.original")).tag("original")
                Text(t("examShare.upload.reconstructed")).tag("reconstructed")
            }
            if role == "answer" || role == "both" {
                Picker(t("examShare.upload.answerSource"), selection: draft.answerSource) {
                    Text(t("examShare.upload.official")).tag(Optional("official"))
                    Text(t("examShare.upload.student")).tag(Optional("student"))
                    Text(t("examShare.upload.unknown")).tag(Optional("unknown"))
                }
            }
            TextField(t("examShare.upload.note"), text: draft.note, axis: .vertical)
        }
        .onChange(of: role) {
            if isExam && draft.wrappedValue.occasion == "none" { draft.occasion.wrappedValue = "unknown" }
            if (role == "answer" || role == "both") && draft.wrappedValue.answerSource == nil {
                draft.answerSource.wrappedValue = "unknown"
            }
        }
    }

    private func subjectFields(_ draft: Binding<ExamUploadDraft>) -> some View {
        let codes = draft.wrappedValue.subjectCodes
        return Section(t("examShare.upload.subjects")) {
            ForEach(codes, id: \.self) { code in
                HStack {
                    VStack(alignment: .leading) {
                        Text(subjectNames[code] ?? code)
                        Text(code).font(.caption.monospaced()).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    if code != codes.first {
                        fileButton(t("common.remove"), icon: "minus.circle") { removeSubject(code) }
                    }
                }
            }
            Button(t("examShare.upload.addSubject"), systemImage: "plus") { choosingSubject = true }
                .disabled(codes.count >= 10)
        }
    }

    private var availableTeachers: [ExamTeacher] {
        var seen = Set<String>()
        return (draft?.subjectCodes ?? []).flatMap { subjectTeachers[$0] ?? [] }
            .filter { seen.insert($0.id).inserted }
    }

    private func teacherFields(_ draft: Binding<ExamUploadDraft>) -> some View {
        Section(t("examShare.teacher")) {
            ForEach(availableTeachers, id: \.id) { teacher in
                Toggle(teacher.name, isOn: Binding(
                    get: { draft.wrappedValue.teacherIds.contains(teacher.id) },
                    set: { selected in
                        if selected {
                            draft.teacherIds.wrappedValue.append(teacher.id)
                        } else {
                            draft.teacherIds.wrappedValue.removeAll { $0 == teacher.id }
                        }
                    }
                ))
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
        }
    }

    private func load() async {
        defer { loading = false }
        await refreshStatus()
        guard let subject = entry.subject else {
            if status?.phase == .idle { dismiss() }
            return
        }
        do {
            let answer = try await model.examShare(.uploadOptions(subjectCode: subject.code, request: entry.request))
            guard case let .uploadOptions(options) = answer else { return }
            draft = options.draft
            year = options.draft.academicYear.map(String.init) ?? ""
            newTeachers = options.draft.newTeachers.map { TeacherEntry(familyName: $0.familyName, givenName: $0.givenName) }
            if let code = options.draft.subjectCodes.first {
                subjectNames[code] = subjectName(subject)
                subjectTeachers[code] = options.teachers
            }
        } catch { problem = AppModel.describe(error) }
    }

    private func addSubject(_ subject: ExamSubject) async throws {
        let answer = try await model.examShare(.uploadOptions(subjectCode: subject.code, request: nil))
        guard case let .uploadOptions(options) = answer, let code = options.draft.subjectCodes.first,
            var updated = draft, !updated.subjectCodes.contains(code)
        else { return }
        updated.subjectCodes.append(code)
        draft = updated
        subjectNames[code] = subjectName(subject)
        subjectTeachers[code] = options.teachers
    }

    private func removeSubject(_ code: String) {
        guard var updated = draft else { return }
        updated.subjectCodes.removeAll { $0 == code }
        subjectNames.removeValue(forKey: code)
        let removed = Set(subjectTeachers.removeValue(forKey: code)?.map(\.id) ?? [])
        let retained = Set(updated.subjectCodes.flatMap { subjectTeachers[$0]?.map(\.id) ?? [] })
        updated.teacherIds.removeAll { removed.contains($0) && !retained.contains($0) }
        draft = updated
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

    private func submit() async {
        guard canSubmit, var updated = draft, let selection else { return }
        updated.academicYear = UInt32(year.trimmingCharacters(in: .whitespacesAndNewlines))
        updated.newTeachers = newTeachers.map {
            ExamNewTeacher(familyName: $0.familyName, givenName: $0.givenName)
        }
        let input = files.map { ExamUploadFile(path: $0.url.path, name: $0.name) }
        // The staging directory is kept alive through the native operation.
        await perform(.upload(files: input, draft: updated))
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
                await onUploaded()
                dismiss()
            case .done:
                if entry.subject == nil { dismiss() }
            default: break
            }
        } catch SdkError.Cancelled {}
        catch is CancellationError {}
        catch { problem = AppModel.describe(error) }
        await refreshStatus()
    }

    private func cancel() async {
        guard !cancelling, canCancel else { return }
        cancelling = true
        do { _ = try await model.examShare(.cancelUpload) }
        catch {
            problem = AppModel.describe(error)
            cancelling = false
        }
    }

    private func refreshStatus() async {
        guard case let .uploadStatus(value)? = try? await model.examShare(.uploadStatus), !Task.isCancelled else { return }
        status = value
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
    let subjectName: (ExamSubject) -> String
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
                            Text(subjectName(subject))
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
        let files = try urls.map { try copy($0, into: directory.url, imagesOnly: urls.count > 1) }
        return Self(directory: directory, files: files)
    }

    private static func copy(_ source: URL, into folder: URL, imagesOnly: Bool) throws -> File {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        var coordinationError: NSError?
        var outcome: Result<File, Error>?
        NSFileCoordinator().coordinate(readingItemAt: source, options: [], error: &coordinationError) { url in
            outcome = Result {
                let type = try url.resourceValues(forKeys: [.contentTypeKey]).contentType
                    ?? UTType(filenameExtension: source.pathExtension)
                guard let type, types.contains(where: { type.conforms(to: $0) }),
                    !imagesOnly || type.conforms(to: .image)
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
