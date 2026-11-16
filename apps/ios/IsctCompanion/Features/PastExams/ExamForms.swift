import IsctCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import VisionKit

private let occasions = ["final", "midterm", "makeup", "quiz", "check", "report", "exercise", "textbook", "mock"]
/// Held several times a term, so the number matters (Core's `NUMBERED`).
private let numbered: Set<String> = ["quiz", "check", "report"]
private let roles = ["question", "answer", "both", "notes", "other"]
private let photoExtensions: Set<String> = ["jpg", "jpeg", "png", "webp"]
/// Requests and uploads name a recent year; older ones are rare.
private let yearsBack = 8

/// What an upload belongs to: a course, or the request it answers.
struct ExamUploadTarget {
    let subjectCode: String
    let subjectName: String
    let teachers: [ExamTeacher]
    let mine: [String]
    var academicYear: UInt32? = nil
    var occasion: String? = nil
    var role: String? = nil
    var requestId: String? = nil
}

/// Shares a past exam. Pages come from the camera's document scanner,
/// Photos or Files; Core checks the site's rules again and turns photos
/// into one PDF.
struct ExamUploadSheet: View {
    @Environment(ExamShareStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let target: ExamUploadTarget
    let done: () async -> Void

    private struct Picked: Identifiable {
        let id = UUID()
        let url: URL
        var name: String { url.lastPathComponent }
        var isPhoto: Bool { photoExtensions.contains(url.pathExtension.lowercased()) }
        var size: String {
            let bytes = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            return ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
        }
    }

    @State private var files: [Picked] = []
    @State private var role: String
    @State private var year: UInt32?
    @State private var occasion: String
    @State private var number = 1
    @State private var teachers: Set<String>
    @State private var provenance = "original"
    @State private var answerSource = "unknown"
    @State private var note = ""
    @State private var sending = false
    @State private var scanning = false
    @State private var importing = false
    @State private var photos: [PhotosPickerItem] = []

    init(target: ExamUploadTarget, done: @escaping () async -> Void) {
        self.target = target
        self.done = done
        _role = State(initialValue: target.role ?? "question")
        _year = State(initialValue: target.academicYear ?? UInt32(Calendar.current.component(.year, from: .now)))
        _occasion = State(initialValue: target.occasion ?? "")
        _teachers = State(initialValue: Set(target.mine))
    }

    private var sitting: Bool { role != "notes" && role != "other" }
    private var question: Bool { role == "question" || role == "both" }
    private var answer: Bool { role == "answer" || role == "both" }
    private var mixed: Bool { files.count > 1 && !files.allSatisfy(\.isPhoto) }
    private var ready: Bool { !files.isEmpty && !mixed && (!sitting || !occasion.isEmpty) && !sending }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ForEach(files) { file in
                        HStack {
                            Image(systemName: file.isPhoto ? "photo" : "doc").foregroundStyle(Theme.muted)
                            Text(file.name).lineLimit(1).truncationMode(.middle)
                            Spacer()
                            Text(file.size).font(.footnote).foregroundStyle(Theme.muted)
                        }
                    }
                    .onDelete { files.remove(atOffsets: $0) }
                    Menu {
                        if VNDocumentCameraViewController.isSupported {
                            Button(t("examShare.scan"), systemImage: "doc.viewfinder") { scanning = true }
                        }
                        PhotosPicker(selection: $photos, matching: .images) {
                            Label(t("examShare.photos"), systemImage: "photo.on.rectangle")
                        }
                        Button(t("examShare.files"), systemImage: "folder") { importing = true }
                    } label: {
                        Label(t("examShare.pickFiles"), systemImage: "plus")
                    }
                } header: {
                    Text(t("examShare.uploadFiles"))
                } footer: {
                    Text(mixed ? t("examShare.onlyPhotosMerge") : t("examShare.uploadFilesHint"))
                        .foregroundStyle(mixed ? Theme.danger : Theme.muted)
                }
                Section {
                    Picker(t("examShare.kind"), selection: $role) {
                        ForEach(roles, id: \.self) { Text(t("examShare.role.\($0)")).tag($0) }
                    }
                    if sitting {
                        Picker(t("examShare.year"), selection: $year) {
                            let now = Calendar.current.component(.year, from: .now)
                            ForEach(0..<yearsBack, id: \.self) { back in
                                Text(verbatim: String(now - back)).tag(Optional(UInt32(now - back)))
                            }
                            Text(t("examShare.unknownYear")).tag(nil as UInt32?)
                        }
                        Picker(t("examShare.occasion"), selection: $occasion) {
                            Text(t("examShare.pickOccasion")).tag("")
                            ForEach(occasions + ["unknown"], id: \.self) { Text(t("examShare.occasion.\($0)")).tag($0) }
                        }
                        if numbered.contains(occasion) {
                            Stepper(value: $number, in: 1...15) {
                                LabeledContent(t("examShare.occasionNumber"), value: "\(number)")
                            }
                        }
                    }
                }
                if !target.teachers.isEmpty {
                    Section(t("examShare.teachers")) {
                        ForEach(target.teachers, id: \.id) { teacher in
                            Button {
                                if !teachers.insert(teacher.id).inserted { teachers.remove(teacher.id) }
                            } label: {
                                HStack {
                                    Text(teacher.name).foregroundStyle(Theme.ink)
                                    Spacer()
                                    if teachers.contains(teacher.id) {
                                        Image(systemName: "checkmark").foregroundStyle(Theme.accent)
                                    }
                                }
                            }
                        }
                    }
                }
                if question || answer {
                    Section {
                        if question {
                            Picker(t("examShare.provenance"), selection: $provenance) {
                                ForEach(["original", "reconstructed"], id: \.self) {
                                    Text(t("examShare.provenance.\($0)")).tag($0)
                                }
                            }
                        }
                        if answer {
                            Picker(t("examShare.answerSource"), selection: $answerSource) {
                                ForEach(["official", "student", "unknown"], id: \.self) {
                                    Text(t("examShare.answerSource.\($0)")).tag($0)
                                }
                            }
                        }
                    }
                }
                Section {
                    TextField(t("examShare.note"), text: $note)
                        .onChange(of: note) { if note.count > 40 { note = String(note.prefix(40)) } }
                } footer: {
                    Text(t("examShare.uploadPublic"))
                }
                if let problem = store.problem {
                    Section { Text(problem).foregroundStyle(Theme.danger) }
                }
            }
            .themedList()
            .navigationTitle(t("examShare.uploadTitle", ["subject": target.subjectName]))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if sending {
                        ProgressView()
                    } else {
                        Button(t("examShare.upload")) { Task { await upload() } }.disabled(!ready)
                    }
                }
            }
            .interactiveDismissDisabled(sending)
            .fullScreenCover(isPresented: $scanning) {
                DocumentScanner { pages in
                    scanning = false
                    for (index, page) in pages.enumerated() {
                        if let data = page.jpegData(compressionQuality: 0.9) {
                            add(data, name: "scan-\(files.count + index + 1).jpg")
                        }
                    }
                }
                .ignoresSafeArea()
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.pdf, .jpeg, .png, .webP, .plainText],
                          allowsMultipleSelection: true) { result in
                for url in (try? result.get()) ?? [] {
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    if let data = try? Data(contentsOf: url) { add(data, name: url.lastPathComponent) }
                }
            }
            .onChange(of: photos) {
                let items = photos
                photos = []
                Task {
                    for item in items {
                        // Photos may be HEIC, which the site does not take: as JPEG.
                        guard let data = try? await item.loadTransferable(type: Data.self),
                              let jpeg = UIImage(data: data)?.jpegData(compressionQuality: 0.9) else { continue }
                        add(jpeg, name: "photo-\(files.count + 1).jpg")
                    }
                }
            }
        }
    }

    /// A private copy Core reads; picked files may vanish once the picker closes.
    private func add(_ data: Data, name: String) {
        let folder = FileManager.default.temporaryDirectory
            .appending(path: "exam-upload", directoryHint: .isDirectory)
            .appending(path: UUID().uuidString, directoryHint: .isDirectory)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let url = folder.appending(path: name)
            try data.write(to: url)
            files.append(Picked(url: url))
        } catch {
            store.problem = AppModel.describe(error)
        }
    }

    private func upload() async {
        sending = true
        defer { sending = false }
        let draft = ExamUploadDraft(
            files: files.map(\.url.path),
            subjectCodes: [target.subjectCode],
            teacherIds: target.teachers.map(\.id).filter(teachers.contains),
            academicYear: sitting ? year : nil,
            occasion: sitting ? occasion : nil,
            occasionNumber: numbered.contains(occasion) ? UInt32(number) : 0,
            variant: "",
            role: role,
            provenance: question ? provenance : nil,
            answerSource: answer ? answerSource : nil,
            note: note,
            requestId: target.requestId)
        guard case .uploaded? = await store.call(.upload(draft: draft)) else { return }
        store.notice = t("examShare.uploaded")
        dismiss()
        await done()
    }
}

/// VisionKit's scanner: finds the page edges, straightens and evens out
/// each photo, so a paper exam reads like a scan.
private struct DocumentScanner: UIViewControllerRepresentable {
    let finish: ([UIImage]) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(finish: finish) }

    func makeUIViewController(context: Context) -> VNDocumentCameraViewController {
        let controller = VNDocumentCameraViewController()
        controller.delegate = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: VNDocumentCameraViewController, context: Context) {}

    final class Coordinator: NSObject, VNDocumentCameraViewControllerDelegate {
        let finish: ([UIImage]) -> Void
        init(finish: @escaping ([UIImage]) -> Void) { self.finish = finish }

        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
            finish((0..<scan.pageCount).map(scan.imageOfPage))
        }

        func documentCameraViewControllerDidCancel(_ controller: VNDocumentCameraViewController) { finish([]) }

        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFailWithError error: Error) {
            finish([])
        }
    }
}

/// Asks for documents of one subject; every field but the subject is optional.
struct ExamRequestForm: View {
    @Environment(ExamShareStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let subject: ExamSubjectPage
    let submit: (ExamRequestDraft) async -> Void
    @State private var classId: UInt32?
    @State private var year: UInt32?
    @State private var occasion = ""
    @State private var role = ""
    @State private var description = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if subject.rows.count > 1 {
                        Picker(t("examShare.class"), selection: $classId) {
                            Text(t("examShare.allClasses")).tag(nil as UInt32?)
                            ForEach(subject.rows, id: \.classIds) { row in
                                if let id = row.classIds.first {
                                    Text((row.labels + row.teachers.map(\.name) + [row.taughtIn])
                                        .filter { !$0.isEmpty }.joined(separator: " · ")).tag(Optional(id))
                                }
                            }
                        }
                    }
                    Picker(t("examShare.year"), selection: $year) {
                        Text(t("examShare.anyYear")).tag(nil as UInt32?)
                        let now = Calendar.current.component(.year, from: .now)
                        ForEach(0..<yearsBack, id: \.self) { back in
                            Text(verbatim: String(now - back)).tag(Optional(UInt32(now - back)))
                        }
                    }
                    Picker(t("examShare.occasion"), selection: $occasion) {
                        Text(t("examShare.allOccasions")).tag("")
                        ForEach(occasions, id: \.self) { Text(t("examShare.occasion.\($0)")).tag($0) }
                    }
                    Picker(t("examShare.fileType"), selection: $role) {
                        Text(t("examShare.anyRole")).tag("")
                        Text(t("examShare.role.question")).tag("question")
                        Text(t("examShare.role.answer")).tag("answer")
                    }
                }
                Section(t("examShare.description")) {
                    TextField(t("examShare.description"), text: $description, axis: .vertical).lineLimit(3...6)
                }
                if let problem = store.problem {
                    Section { Text(problem).foregroundStyle(Theme.danger) }
                }
            }
            .themedList()
            .navigationTitle(t("examShare.newRequest"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("examShare.sendRequest")) {
                        let draft = ExamRequestDraft(
                            subjectCode: subject.subject.code, classId: classId, academicYear: year,
                            occasion: occasion.isEmpty ? nil : occasion,
                            wantedRole: role.isEmpty ? nil : role,
                            description: description.trimmingCharacters(in: .whitespacesAndNewlines))
                        Task { await submit(draft) }
                    }
                    .disabled(store.busy || description.utf16.count > 1000)
                }
            }
        }
    }
}

struct ExamReportForm: View {
    @Environment(ExamShareStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let submit: (ExamReportReason, String) async -> Void
    @State private var reason: ExamReportReason = .copyright
    @State private var detail = ""

    var body: some View {
        NavigationStack {
            Form {
                Picker(t("examShare.reportReason"), selection: $reason) {
                    Text(t("examShare.reason.copyright")).tag(ExamReportReason.copyright)
                    Text(t("examShare.reason.inappropriate")).tag(ExamReportReason.inappropriate)
                    Text(t("examShare.reason.wrongAnswer")).tag(ExamReportReason.wrongAnswer)
                    Text(t("examShare.reason.brokenFile")).tag(ExamReportReason.brokenFile)
                    Text(t("examShare.reason.other")).tag(ExamReportReason.other)
                }
                .pickerStyle(.inline)
                Section(t("examShare.description")) {
                    TextField(t("examShare.description"), text: $detail, axis: .vertical).lineLimit(3...6)
                }
                if let problem = store.problem {
                    Section { Text(problem).foregroundStyle(Theme.danger) }
                }
            }
            .themedList()
            .navigationTitle(t("examShare.report"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("examShare.sendReport")) {
                        Task { await submit(reason, detail.trimmingCharacters(in: .whitespacesAndNewlines)) }
                    }
                    .disabled(store.busy || detail.utf16.count > 900)
                }
            }
        }
    }
}
