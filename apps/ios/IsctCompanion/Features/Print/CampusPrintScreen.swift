import IsctCore
import PDFKit
import SwiftUI

/// The app's own print screen, the same one Android has: the campus printer
/// for a PDF or a picture, from the share sheet's "Print via campus printer"
/// or a reader's print button. The person picks pages from the thumbnails
/// or by number, copies, sides and colour (and, for a picture, its page);
/// Core checks the account and the printer as the screen opens, keeps only
/// the chosen pages, bills the student ID and sends. The send outlives the
/// screen (`CampusPrintRun`).
struct CampusPrintScreen: View {
    let request: CampusPrintRequest
    @Environment(\.dismiss) private var dismiss
    @State private var loaded: Loaded = .loading
    private var run: CampusPrintRun { .shared }

    enum Loaded {
        case loading
        case ready(PrintDocument)
        case unsupported
    }

    var body: some View {
        NavigationStack {
            content
                .background(Theme.bg)
                .navigationTitle(t("campusPrint.window.printToCampusPrinter"))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(t("common.close")) { dismiss() }
                    }
                }
        }
        .onAppear { run.watching = true }
        .onDisappear { run.watching = false }
        .task {
            run.forget()
            let (url, name) = (request.url, request.name)
            let document = await Task.detached { PrintDocument.load(url, name: name) }.value
            loaded = document.map(Loaded.ready) ?? .unsupported
        }
    }

    @ViewBuilder
    private var content: some View {
        switch run.state {
        case let .sending(title):
            SendingView(title: title) { dismiss() }
        case let .ended(title, printed, message):
            EndedView(title: title, printed: printed, message: message, done: { dismiss() }, again: { run.forget() })
        case .idle:
            switch loaded {
            case .loading:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            case .unsupported:
                ContentUnavailableView {
                    Label(request.name, systemImage: "exclamationmark.triangle")
                } description: {
                    Text(t("campusPrint.screen.otherFiles"))
                }
            case let .ready(document):
                PrintSettings(document: document)
            }
        }
    }
}

/// Whether a print would go through now.
private enum PrinterCheck: Equatable {
    case checking
    case ready(studentId: String)
    case problem(String)
}

private struct PrintSettings: View {
    let document: PrintDocument
    @Environment(AppModel.self) private var model
    @State private var copies = 1
    @State private var sides: CampusPrintSides
    @State private var color: CampusPrintColor
    @State private var media: CampusPrintMedia
    // A picture's page lies the way the picture does until the person says
    // otherwise; turning the picture turns the page with it.
    @State private var landscape: Bool
    @State private var turns = 0
    @State private var choosing = false
    @State private var pagesText = ""
    @State private var checkRound = 0
    @State private var check = PrinterCheck.checking
    private let capabilities = try? campusPrinterCapabilities()

    init(document: PrintDocument) {
        self.document = document
        let capabilities = try? campusPrinterCapabilities()
        _sides = State(initialValue: capabilities?.defaultSides ?? .longEdge)
        _color = State(initialValue: capabilities?.defaultColor ?? .mono)
        _media = State(initialValue: capabilities?.media.first ?? .a4)
        if case let .image(image) = document.kind {
            _landscape = State(initialValue: image.size.width > image.size.height)
        } else {
            _landscape = State(initialValue: false)
        }
    }

    private var total: Int { document.pages }
    private var canChoose: Bool { document.isPDF && total > 1 }
    private var chosen: Set<Int>? { choosing && canChoose ? parsePages(pagesText, total: total) : nil }
    private var pagesBad: Bool { choosing && canChoose && chosen == nil }
    private var count: Int {
        if !canChoose { return max(total, 1) }
        if choosing { return chosen?.count ?? 0 }
        return total
    }
    private var sheets: Int { sides == .oneSided ? count : (count + 1) / 2 }
    private var maxCopies: Int { Int(capabilities?.maxCopies ?? 99) }
    private var marginMils: Int32 { capabilities?.marginMils ?? 167 }
    private var layout: ImageLayout { ImageLayout(media: media, landscape: landscape, turns: turns) }

    var body: some View {
        Form {
            Section { status }
            Section {
                Label {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(document.name).font(.headline).lineLimit(1)
                        if canChoose {
                            Text(t("campusPrint.screen.allPages", ["count": total]))
                                .font(.subheadline).foregroundStyle(Theme.muted)
                        }
                    }
                } icon: {
                    Image(systemName: document.isPDF ? "doc.richtext" : "photo")
                        .foregroundStyle(Theme.accent)
                }
                if case let .image(image) = document.kind {
                    ImagePage(image: image, layout: layout, marginMils: marginMils) {
                        turns = (turns + 1) % 4
                        landscape.toggle()
                    }
                } else if case let .pdf(pdf?) = document.kind {
                    pageStrip(pdf)
                }
            }
            Section {
                Stepper(value: $copies, in: 1...max(maxCopies, 1)) {
                    LabeledContent(t("campusPrint.window.copies"), value: "\(copies)")
                }
                if canChoose { pagesRow }
                if !document.isPDF {
                    option(t("campusPrint.screen.orientation"), $landscape, [
                        (false, t("campusPrint.screen.portrait")), (true, t("campusPrint.screen.landscape")),
                    ])
                }
                option(t("campusPrint.window.sides"), $sides, [
                    (.oneSided, t("campusPrint.window.oneSided")),
                    (.longEdge, t("campusPrint.window.longEdge")),
                    (.shortEdge, t("campusPrint.window.shortEdge")),
                ])
                option(t("campusPrint.window.color"), $color, [
                    (.mono, t("campusPrint.window.blackAndWhite")), (.color, t("campusPrint.window.inColor")),
                ])
                // A PDF prints on the paper its pages are made for; only a
                // picture is laid out on the paper chosen here.
                if !document.isPDF {
                    option(t("campusPrint.screen.paper"), $media, (capabilities?.media ?? [.a4, .letter]).map {
                        ($0, $0 == .a4 ? "A4" : "Letter")
                    })
                }
            } footer: {
                Text(t("campusPrint.screen.rules"))
            }
        }
        .scrollContentBackground(.hidden)
        .safeAreaInset(edge: .bottom) { bottomBar }
        .task(id: checkRound) {
            check = .checking
            let state = await model.campusPrinterState()
            // Core answers with a student ID to bill or a reason it would
            // not print; no answer (no session, a failed call) is neither.
            if let problem = state?.problem {
                check = .problem(CampusPrintRun.text(problem))
            } else if let studentId = state?.studentId {
                check = .ready(studentId: studentId)
            } else {
                check = .problem(t("campusPrint.screen.checkFailed"))
            }
        }
    }

    @ViewBuilder
    private var status: some View {
        switch check {
        case .checking:
            HStack(spacing: 8) {
                ProgressView()
                Text(t("campusPrint.screen.checking")).foregroundStyle(Theme.muted)
            }
        case let .ready(studentId):
            Label {
                Text(t("tools.campusPrinter.readyToPrint") + " · " + t("tools.campusPrinter.billedTo", ["id": studentId]))
                    .fontWeight(.semibold)
            } icon: {
                Image(systemName: "checkmark.circle.fill")
            }
            .foregroundStyle(Theme.ok)
        case let .problem(message):
            VStack(alignment: .leading, spacing: 10) {
                Label(message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Theme.warn)
                Button(t("tools.campusPrinter.checkAgain"), systemImage: "arrow.clockwise") { checkRound += 1 }
                    .buttonStyle(.bordered)
            }
            .padding(.vertical, 4)
        }
    }

    private var pagesRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(t("campusPrint.screen.pages"))
            Picker(t("campusPrint.screen.pages"), selection: $choosing) {
                Text(t("campusPrint.screen.allPages", ["count": total])).tag(false)
                Text(t("campusPrint.screen.choosePages")).tag(true)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            if choosing {
                TextField(t("campusPrint.screen.pagesExample"), text: $pagesText)
                    .keyboardType(.numbersAndPunctuation)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityLabel(t("campusPrint.screen.pagesField"))
                if pagesBad {
                    Text(pagesText.trimmingCharacters(in: .whitespaces).isEmpty
                         ? t("campusPrint.screen.pagesEmpty")
                         : t("campusPrint.screen.pagesInvalid", ["count": total]))
                        .font(.footnote)
                        .foregroundStyle(Theme.danger)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func option<Value: Hashable>(_ label: String, _ selection: Binding<Value>, _ choices: [(Value, String)]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(label)
            Picker(label, selection: selection) {
                ForEach(choices.indices, id: \.self) { index in
                    Text(choices[index].1).tag(choices[index].0)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        }
        .padding(.vertical, 4)
    }

    private func pageStrip(_ pdf: PDFDocument) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if canChoose {
                HStack {
                    Text(choosing && chosen != nil
                         ? t("campusPrint.screen.chosen", ["count": chosen?.count ?? 0])
                         : t("campusPrint.screen.tapToChoose"))
                        .font(.footnote)
                        .foregroundStyle(Theme.muted)
                    Spacer()
                    if choosing {
                        Button(t("campusPrint.screen.chooseAll")) { choosing = false }
                            .font(.footnote.weight(.semibold))
                            .buttonStyle(.borderless)
                    }
                }
            }
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(spacing: 10) {
                    ForEach(1...max(total, 1), id: \.self) { number in
                        let marked = choosing && (chosen?.contains(number) ?? false)
                        Button { toggle(number) } label: {
                            PageThumb(pdf: pdf, index: number - 1, number: number, marked: marked)
                                .opacity(!choosing || marked ? 1 : 0.45)
                        }
                        .buttonStyle(.plain)
                        .disabled(!canChoose)
                        .accessibilityLabel(t("campusPrint.screen.page", ["number": number]))
                        .accessibilityAddTraits(marked ? .isSelected : [])
                    }
                }
                .padding(.vertical, 8)
                .padding(.horizontal, 4)
            }
        }
    }

    /// The first tap prints only that page; later taps add or remove one,
    /// and choosing every page is printing all of them.
    private func toggle(_ page: Int) {
        guard choosing else {
            choosing = true
            pagesText = "\(page)"
            return
        }
        var next = parsePages(pagesText, total: total) ?? []
        if next.contains(page) { next.remove(page) } else { next.insert(page) }
        pagesText = formatPages(next)
        if next.count == total { choosing = false }
    }

    /// Larger than Core takes: refused here, before anything is read.
    private var tooLarge: Bool { document.isPDF && document.tooLarge(maxBytes) }
    private var maxBytes: UInt64 { capabilities?.maxDocumentBytes ?? 64 * 1024 * 1024 }

    private var blocked: Bool {
        if pagesBad || tooLarge { return true }
        if case .ready = check { return false }
        return true
    }

    private var bottomBar: some View {
        VStack(spacing: 10) {
            if tooLarge {
                Text(CampusPrintRun.tooLargeText)
                    .font(.footnote)
                    .foregroundStyle(Theme.warn)
                    .multilineTextAlignment(.center)
            } else if !pagesBad, !(document.isPDF && total == 0) {
                Text(t("campusPrint.screen.summary", ["pages": count, "sheets": sheets, "copies": copies]))
                    .font(.footnote)
                    .foregroundStyle(Theme.muted)
            }
            Button(action: send) {
                Label(t("campusPrint.window.print"), systemImage: "printer")
                    .font(.headline)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
            }
            .buttonStyle(.borderedProminent)
            .disabled(blocked)
        }
        .padding(.horizontal, Theme.gutter)
        .padding(.top, 10)
        .padding(.bottom, 6)
        .background(.bar)
    }

    private func send() {
        let pages = choosing ? (chosen ?? []).sorted().map { UInt32($0) } : []
        let job = CampusPrintJob(
            title: document.name, copies: UInt32(copies), sides: sides, color: color, pages: pages)
        let (file, placed, margin, limit) = (document, layout, marginMils, maxBytes)
        CampusPrintRun.shared.start(model, job: job) { try file.bytes(placed, marginMils: margin, limit: limit) }
    }
}

/// One page of a PDF, drawn small.
private struct PageThumb: View {
    let pdf: PDFDocument
    let index: Int
    let number: Int
    let marked: Bool
    @State private var picture: UIImage?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ZStack(alignment: .topTrailing) {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(.white)
                    .overlay {
                        if let picture {
                            Image(uiImage: picture).resizable().scaledToFit().padding(2)
                        }
                    }
                    .overlay {
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(marked ? Theme.accent : Theme.line, lineWidth: marked ? 2 : 1)
                    }
                    .frame(width: 52, height: 72)
                    // The page sits low and left in a frame a badge's half
                    // larger, so the check mark on its corner is drawn whole.
                    .padding(.top, 9)
                    .padding(.trailing, 9)
                if marked {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 18))
                        .foregroundStyle(Theme.accentInk, Theme.accent)
                }
            }
            Text("\(number)")
                .font(.caption2.weight(marked ? .bold : .regular))
                .foregroundStyle(marked ? Theme.accent : Theme.muted)
                .frame(width: 52)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task(id: index) {
            picture = pdf.page(at: index)?.thumbnail(of: CGSize(width: 104, height: 144), for: .mediaBox)
        }
    }
}

/// A picture's page as it will print: the paper the way it lies, the
/// printer's margin, the picture fitted inside; and a button to turn it.
private struct ImagePage: View {
    let image: UIImage
    let layout: ImageLayout
    let marginMils: Int32
    let turn: () -> Void
    @State private var preview: UIImage?

    var body: some View {
        let page = layout.page
        let height: CGFloat = layout.landscape ? 150 : 210
        let width = height * page.width / page.height
        let margin = width * (CGFloat(marginMils) * 72 / 1000) / page.width
        HStack(spacing: 20) {
            Spacer(minLength: 0)
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(.white)
                .overlay {
                    if let preview {
                        Image(uiImage: preview).resizable().scaledToFit().padding(margin)
                    }
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 4, style: .continuous).strokeBorder(Theme.line)
                }
                .frame(width: width, height: height)
                .shadow(color: .black.opacity(0.12), radius: 3, y: 1)
                .animation(.snappy, value: layout)
            Button(action: turn) {
                VStack(spacing: 4) {
                    Image(systemName: "rotate.right")
                        .font(.title3)
                        .frame(width: 44, height: 44)
                        .background(Theme.surface2, in: Circle())
                    Text(t("campusPrint.screen.rotate")).font(.caption).foregroundStyle(Theme.muted)
                }
            }
            .buttonStyle(.plain)
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
        .task(id: layout.turns) {
            let (image, turns) = (image, layout.turns)
            preview = await Task.detached {
                let small = image.preparingThumbnail(of: CGSize(width: 600, height: 600 * image.size.height / max(image.size.width, 1))) ?? image
                return ImageLayout.turned(small, turns)
            }.value
        }
    }
}

private struct SendingView: View {
    let title: String
    let leave: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            Spacer()
            ProgressView().controlSize(.large)
            Text(t("campusPrint.window.sending")).font(.title3.weight(.bold))
            Text(title).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(2).multilineTextAlignment(.center)
            Text(t("campusPrint.screen.leaveNote"))
                .font(.footnote).foregroundStyle(Theme.muted).multilineTextAlignment(.center)
            Spacer()
            Button(action: leave) {
                Text(t("campusPrint.screen.inBackground")).frame(maxWidth: .infinity).padding(.vertical, 6)
            }
            .buttonStyle(.bordered)
        }
        .frame(maxWidth: .infinity)
        .padding(Theme.gutter)
    }
}

private struct EndedView: View {
    let title: String
    let printed: Bool
    let message: String
    let done: () -> Void
    let again: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Spacer()
            Image(systemName: printed ? "checkmark" : "exclamationmark.triangle")
                .font(.system(size: 40, weight: .semibold))
                .foregroundStyle(printed ? Theme.ok : Theme.warn)
                .frame(width: 88, height: 88)
                .background(printed ? Theme.okSoft : Theme.warnSoft, in: Circle())
            Text(message).font(.title3.weight(.bold)).multilineTextAlignment(.center)
            Text(title).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(2).multilineTextAlignment(.center)
            Spacer()
            Button(action: done) {
                Text(t("common.done")).frame(maxWidth: .infinity).padding(.vertical, 6)
            }
            .buttonStyle(.borderedProminent)
            Button(action: again) {
                Text(t("campusPrint.screen.printAgain")).frame(maxWidth: .infinity).padding(.vertical, 6)
            }
            .buttonStyle(.bordered)
        }
        .frame(maxWidth: .infinity)
        .padding(Theme.gutter)
    }
}
