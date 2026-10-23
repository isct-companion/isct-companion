import AVFoundation
import IsctCore
import PhotosUI
import SwiftUI
import Vision
import VisionKit

// Ways to bring a school second factor onto the phone. Parsing (authenticator
// links and transfers) and recognition (Matrix OCR) are Core's; the shell only
// turns the camera or a photo into text or image bytes.

/// One row of equal tiles, icon over a short label, one per source. A bordered
/// HStack sized each button to its label, so they wrapped unevenly.
private struct SourceTiles<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: 8) { content }
            .buttonStyle(SourceTile())
            .fixedSize(horizontal: false, vertical: true)
    }
}

private struct SourceTile: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .labelStyle(TileLabel())
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.vertical, 10)
            .foregroundStyle(enabled ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
            .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(Rectangle())
            .opacity(configuration.isPressed ? 0.6 : 1)
    }
}

private struct TileLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View { Tile(configuration: configuration) }

    private struct Tile: View {
        let configuration: LabelStyleConfiguration
        // Symbols differ in height; a fixed slot keeps the labels on one line.
        @ScaledMetric(relativeTo: .title3) private var icon: CGFloat = 26

        var body: some View {
            VStack(spacing: 4) {
                configuration.icon.font(.title3).frame(height: icon)
                configuration.title.font(.caption).lineLimit(1).minimumScaleFactor(0.7)
            }
            .padding(.horizontal, 4)
        }
    }
}

// MARK: OTP

/// The OTP setup, as the desktop's chooser: once saved, one line that says
/// codes fill themselves in; otherwise a choice between an authenticator's
/// export QR (camera or a screenshot on this phone) and a pasted setup.
/// Registering with the school is the desktop's job, so it is pointed to.
struct OtpImportField: View {
    let saved: Bool
    @Binding var setup: String
    @State private var changing = false
    @State private var path = Path.export
    @State private var scanning = false
    @State private var photo: PhotosPickerItem?
    @State private var choices: [OtpImportAccount] = []
    @State private var label: String?
    @State private var error: String?

    private enum Path: Hashable { case export, paste }

    var body: some View {
        if saved && !changing && setup.isEmpty {
            HStack {
                Label(t("Codes are filled in for you at sign-in."), systemImage: "checkmark.circle.fill")
                    .labelStyle(.titleAndIcon)
                    .foregroundStyle(Theme.ok, Theme.ink)
                Spacer()
                Button(t("Change")) { changing = true }
            }
        } else {
            chooser
        }
    }

    /// The camera, photo and account pickers, attached to one row only: in a
    /// Form every view is its own row, and a sheet on each would compete.
    private func presenters<V: View>(_ row: V) -> some View {
        row
            .sheet(isPresented: $scanning) {
                QRScanner { payload in
                    scanning = false
                    read(payload)
                }
                .ignoresSafeArea()
            }
            .onChange(of: photo) { _, item in
                guard let item else { return }
                photo = nil
                Task { await readPhoto(item) }
            }
            .confirmationDialog(t("Which account should be used?"), isPresented: .constant(!choices.isEmpty), titleVisibility: .visible) {
                ForEach(Array(choices.enumerated()), id: \.offset) { _, account in
                    Button(Self.name(account)) { choose(account) }
                }
                Button(t("Cancel"), role: .cancel) { choices = [] }
            }
    }

    @ViewBuilder
    private var chooser: some View {
        presenters(
            Picker(t("Source"), selection: $path) {
                Text(t("From an authenticator app")).tag(Path.export)
                Text(t("Setup link or secret")).tag(Path.paste)
            }
            .pickerStyle(.segmented)
        )
        switch path {
        case .export:
            SourceTiles {
                if DataScannerViewController.isSupported {
                    Button(t("Scan QR"), systemImage: "qrcode.viewfinder") { scanning = true }
                }
                PhotosPicker(selection: $photo, matching: .images) {
                    Label(t("From photo"), systemImage: "photo")
                }
            }
            Text(t("Google Authenticator: Transfer accounts → Export accounts, select only the school account."))
                .font(.footnote)
                .foregroundStyle(Theme.muted)
        case .paste:
            TextField(t("otpauth://… or secret key"), text: $setup)
                .textContentType(nil)
                .keyboardType(.asciiCapable)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .privacySensitive()
                .onChange(of: setup) { _, _ in label = nil }
            SourceTiles {
                Button(t("Paste"), systemImage: "doc.on.clipboard") {
                    if let text = UIPasteboard.general.string { read(text) } else { error = t("The clipboard is empty.") }
                }
            }
            Text(t("Not the six-digit code that changes every 30 seconds."))
                .font(.footnote)
                .foregroundStyle(Theme.muted)
        }
        if let label {
            Text(t("Read “{{label}}”. Save to use it.", ["label": label]))
                .font(.footnote)
                .foregroundStyle(Theme.ok)
        }
        if let error {
            Text(error).font(.footnote).foregroundStyle(Theme.danger)
        }
        if !saved {
            Text(t("Not registered yet? The desktop app registers with the school for you."))
                .font(.footnote)
                .foregroundStyle(Theme.muted)
        } else if setup.isEmpty {
            Button(t("Keep the saved setup")) { changing = false }
        }
    }

    private func read(_ payload: String) {
        error = nil
        let text = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        // A bare secret is typed or pasted as is; Core validates it on save.
        guard text.contains("://") else {
            setup = text
            return
        }
        do {
            let accounts = try importOtp(payload: text)
            if accounts.count == 1 { choose(accounts[0]) } else { choices = accounts }
        } catch OtpImportError.MultiplePages {
            error = t("Export only your school account, then load it again.")
        } catch OtpImportError.Unsupported {
            error = t("This account uses a code format that is not supported.")
        } catch {
            self.error = t("This is not a QR code from a supported authenticator app.")
        }
    }

    private func readPhoto(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self),
            let image = UIImage(data: data)?.cgImage
        else {
            error = t("Could not load the image.")
            return
        }
        guard let payload = await Self.qrPayload(in: image) else {
            error = t("No QR code found. Choose an image that shows the whole code.")
            return
        }
        read(payload)
    }

    private func choose(_ account: OtpImportAccount) {
        setup = account.setup
        choices = []
        label = Self.name(account)
    }

    private static func name(_ account: OtpImportAccount) -> String {
        [account.issuer, account.label].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The first QR code Vision finds in a still image.
    nonisolated static func qrPayload(in image: CGImage) async -> String? {
        await Task.detached {
            let request = VNDetectBarcodesRequest()
            request.symbologies = [.qr]
            try? VNImageRequestHandler(cgImage: image).perform([request])
            return request.results?.compactMap(\.payloadStringValue).first
        }.value
    }
}

/// VisionKit's live QR scanner; reports the first code it recognizes.
private struct QRScanner: UIViewControllerRepresentable {
    let found: (String) -> Void

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced,
            isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        try? scanner.startScanning()
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        private var done = false
        init(found: @escaping (String) -> Void) { self.found = found }

        func dataScanner(
            _ scanner: DataScannerViewController, didAdd items: [RecognizedItem], allItems: [RecognizedItem]
        ) {
            guard !done else { return }
            for case let .barcode(code) in items {
                if let payload = code.payloadStringValue {
                    done = true
                    scanner.stopScanning()
                    found(payload)
                    return
                }
            }
        }
    }
}

// MARK: Matrix

/// The Matrix card: photograph it (or pick a photo), Core reads the 70 cells,
/// and the person checks the grid against the card before saving. Pasting all
/// 70 letters (copied from the desktop app) fills the grid too.
struct MatrixImportSection: View {
    let saved: Bool
    @Binding var matrix: String
    @Environment(AppModel.self) private var model
    @State private var cells = Array(repeating: "", count: 70)
    @State private var confidence: [Double?] = Array(repeating: nil, count: 70)
    @State private var editing = false
    @State private var camera: LiveScan?
    @State private var photo: PhotosPickerItem?
    @State private var reading = false
    @State private var note: String?
    @State private var error: String?

    var body: some View {
        Section {
            if !editing {
                Text(saved ? t("Saved") : t("Not set"))
                    .foregroundStyle(saved ? Theme.ok : Theme.muted)
            }
            SourceTiles {
                if model.canScanMatrix {
                    if AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back) != nil {
                        Button(t("Scan with camera"), systemImage: "camera.viewfinder") { startCamera() }
                    }
                    PhotosPicker(selection: $photo, matching: .images) {
                        Label(t("From photo"), systemImage: "photo")
                    }
                }
                Button(t("Paste"), systemImage: "doc.on.clipboard") { paste(UIPasteboard.general.string ?? "") }
                if !editing {
                    Button(t("Enter manually"), systemImage: "keyboard") { editing = true }
                }
            }
            .disabled(reading)
            // Presentation lives on one row: modifiers on a Section repeat on
            // every row, and several covers presenting at once tear down the
            // settings sheet.
            .onChange(of: photo) { _, item in
                guard let item else { return }
                photo = nil
                Task {
                    guard let data = try? await item.loadTransferable(type: Data.self) else {
                        error = t("Could not load the image.")
                        return
                    }
                    await scan(data)
                }
            }
            .onChange(of: cells) { _, value in
                // Core checks for all 70 letters when the account is saved.
                matrix = value.joined()
            }
            .fullScreenCover(item: $camera) { live in
                MatrixCameraScreen(session: live.session) { result in
                    camera = nil
                    if let result { apply(result) }
                }
            }
            if reading {
                ProgressView(value: Double(model.ocrProgress), total: 70) {
                    Text(t("Reading {{done}}/70", ["done": model.ocrProgress])).font(.footnote)
                }
            }
            if editing {
                MatrixGrid(cells: $cells, confidence: $confidence)
                legend
            }
            if let note { Text(note).font(.footnote).foregroundStyle(Theme.muted) }
            if let error { Text(error).font(.footnote).foregroundStyle(Theme.danger) }
        } header: {
            Text(t("Matrix card"))
        } footer: {
            Text(model.canScanMatrix
                ? t("Lay the card flat and photograph the whole grid.")
                : t("Reading is not available on this device. Paste the 70 letters copied from the desktop app, or type them."))
        }
    }

    private var legend: some View {
        HStack(spacing: 12) {
            swatch(Theme.okSoft, t("Certain"))
            swatch(Theme.warnSoft, t("Check"))
            swatch(Theme.dangerSoft, t("Unreadable"))
        }
        .font(.caption)
        .foregroundStyle(Theme.muted)
    }

    private func swatch(_ color: Color, _ text: String) -> some View {
        HStack(spacing: 4) {
            RoundedRectangle(cornerRadius: 3).fill(color).frame(width: 12, height: 12)
            Text(text)
        }
    }

    private func scan(_ data: Data) async {
        // OCR reads PNG/JPEG/WebP; HEIC photos are converted here.
        guard let jpeg = UIImage(data: data)?.jpegData(compressionQuality: 0.95) else {
            error = t("Could not load the image.")
            return
        }
        await scan(jpeg: jpeg)
    }

    private func startCamera() {
        error = nil
        note = nil
        do { camera = LiveScan(session: try model.beginMatrixCamera()) } catch {
            self.error = AppModel.describe(error)
        }
    }

    /// The final state of a live scan fills the same draft as a photo.
    private func apply(_ result: [MatrixCell]) {
        guard result.count == 70 else { return }
        cells = result.map(\.text)
        confidence = result.map(\.confidence)
        editing = true
        note = t("Check the coloured cells against your card.")
    }

    private func scan(jpeg: Data) async {
        reading = true
        error = nil
        note = nil
        defer { reading = false }
        do {
            let result = try await model.scanMatrix(jpeg)
            guard result.count == 70 else {
                error = t("Could not find the grid. Retake the photo with the whole grid in view.")
                return
            }
            cells = result.map(\.text)
            confidence = result.map(\.confidence)
            editing = true
            note = t("Check the coloured cells against your card.")
        } catch {
            self.error = AppModel.describe(error)
        }
    }

    private func paste(_ text: String) {
        let letters = text.uppercased().filter { !$0.isWhitespace }
        guard letters.count == 70, letters.allSatisfy({ $0.isASCII && $0.isLetter }) else {
            error = t("Paste exactly 70 letters in row order (A1…J1, A2…J2, …). Spaces and line breaks are ignored.")
            return
        }
        error = nil
        cells = letters.map(String.init)
        confidence = Array(repeating: nil, count: 70)
        editing = true
    }
}

/// 7 rows × 10 columns (A–J), one letter each, tinted by Core's review level.
private struct MatrixGrid: View {
    @Binding var cells: [String]
    @Binding var confidence: [Double?]
    @FocusState private var focus: Int?

    var body: some View {
        Grid(horizontalSpacing: 3, verticalSpacing: 3) {
            GridRow {
                Text("")
                ForEach(0..<10, id: \.self) { column in
                    Text(String(UnicodeScalar(65 + column)!)).font(.caption2.weight(.semibold)).foregroundStyle(Theme.muted)
                }
            }
            ForEach(0..<7, id: \.self) { row in
                GridRow {
                    Text("\(row + 1)").font(.caption2.weight(.semibold)).foregroundStyle(Theme.muted)
                    ForEach(0..<10, id: \.self) { column in cell(row * 10 + column) }
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func cell(_ index: Int) -> some View {
        TextField("", text: Binding(
            get: { cells[index] },
            set: { value in
                let letter = value.uppercased().filter { $0.isASCII && $0.isLetter }.suffix(1)
                cells[index] = String(letter)
                // A corrected cell is the person's own entry now.
                confidence[index] = nil
                if !letter.isEmpty, index < 69 { focus = index + 1 }
            }))
            .focused($focus, equals: index)
            .multilineTextAlignment(.center)
            .font(.callout.monospaced().weight(.semibold))
            .textInputAutocapitalization(.characters)
            .autocorrectionDisabled()
            .keyboardType(.asciiCapable)
            .privacySensitive()
            .frame(minWidth: 24, minHeight: 30)
            .background(fill(index), in: RoundedRectangle(cornerRadius: 5))
    }

    private func fill(_ index: Int) -> Color {
        guard let score = confidence[index] else { return Theme.surface2 }
        switch matrixConfidence(text: cells[index], confidence: score) {
        case .high: return Theme.okSoft
        case .medium: return Theme.warnSoft
        case .low: return Theme.dangerSoft
        }
    }
}

/// One live camera session, presented while it runs.
private struct LiveScan: Identifiable {
    let id = UUID()
    let session: MatrixCameraSession
}
