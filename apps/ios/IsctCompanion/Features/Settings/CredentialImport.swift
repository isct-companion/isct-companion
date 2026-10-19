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

/// The OTP setup field with three sources: the camera, a screenshot (the
/// authenticator's export QR on this same phone), or pasted text.
struct OtpImportField: View {
    let saved: Bool
    @Binding var setup: String
    @State private var scanning = false
    @State private var photo: PhotosPickerItem?
    @State private var choices: [OtpImportAccount] = []
    @State private var label: String?
    @State private var error: String?

    var body: some View {
        TextField(saved ? "設定済み（変更する場合のみ）" : "TOTP シークレットまたは otpauth:// リンク", text: $setup)
            .textContentType(nil)
            .keyboardType(.asciiCapable)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .privacySensitive()
            .onChange(of: setup) { _, _ in label = nil }
        SourceTiles {
            if DataScannerViewController.isSupported {
                Button("QR を読み取る", systemImage: "qrcode.viewfinder") { scanning = true }
            }
            PhotosPicker(selection: $photo, matching: .images) {
                Label("写真から", systemImage: "photo")
            }
            Button("貼り付け", systemImage: "doc.on.clipboard") {
                if let text = UIPasteboard.general.string { read(text) } else { error = "クリップボードが空です。" }
            }
        }
        if let label {
            Text("「\(label)」を読み取りました。保存すると使われます。")
                .font(.footnote)
                .foregroundStyle(Theme.ok)
        }
        if let error {
            Text(error).font(.footnote).foregroundStyle(Theme.danger)
        }
        Text("Google Authenticator は「アカウントを移行 → エクスポート」で学校のアカウントだけを選び、表示された QR のスクリーンショットを「写真から」で読み込めます。")
            .font(.footnote)
            .foregroundStyle(Theme.muted)
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
            .confirmationDialog("どのアカウントを使いますか？", isPresented: .constant(!choices.isEmpty), titleVisibility: .visible) {
                ForEach(Array(choices.enumerated()), id: \.offset) { _, account in
                    Button(Self.name(account)) { choose(account) }
                }
                Button("キャンセル", role: .cancel) { choices = [] }
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
            error = "学校のアカウントだけをエクスポートして、もう一度読み込んでください。"
        } catch OtpImportError.Unsupported {
            error = "このアカウントの認証コード形式には対応していません。"
        } catch {
            self.error = "対応している認証アプリの QR コードではありません。"
        }
    }

    private func readPhoto(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self),
            let image = UIImage(data: data)?.cgImage
        else {
            error = "画像を読み込めませんでした。"
            return
        }
        guard let payload = await Self.qrPayload(in: image) else {
            error = "QR コードが見つかりません。QR 全体が写った画像を選んでください。"
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
    @State private var camera = false
    @State private var photo: PhotosPickerItem?
    @State private var reading = false
    @State private var note: String?
    @State private var error: String?

    var body: some View {
        Section {
            if !editing {
                Text(saved ? "設定済み" : "未設定")
                    .foregroundStyle(saved ? Theme.ok : Theme.muted)
            }
            SourceTiles {
                if model.canScanMatrix {
                    if UIImagePickerController.isSourceTypeAvailable(.camera) {
                        Button("撮影", systemImage: "camera") { camera = true }
                    }
                    PhotosPicker(selection: $photo, matching: .images) {
                        Label("写真から", systemImage: "photo")
                    }
                }
                Button("貼り付け", systemImage: "doc.on.clipboard") { paste(UIPasteboard.general.string ?? "") }
                if !editing {
                    Button("入力", systemImage: "keyboard") { editing = true }
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
                        error = "画像を読み込めませんでした。"
                        return
                    }
                    await scan(data)
                }
            }
            .onChange(of: cells) { _, value in
                // Core checks for all 70 letters when the account is saved.
                matrix = value.joined()
            }
            .fullScreenCover(isPresented: $camera) {
                CameraCapture { image in
                    camera = false
                    if let image { Task { await scan(image) } }
                }
                .ignoresSafeArea()
            }
            if reading {
                ProgressView(value: Double(model.ocrProgress), total: 70) {
                    Text("読み取り中 \(model.ocrProgress)/70").font(.footnote)
                }
            }
            if editing {
                MatrixGrid(cells: $cells, confidence: $confidence)
                legend
            }
            if let note { Text(note).font(.footnote).foregroundStyle(Theme.muted) }
            if let error { Text(error).font(.footnote).foregroundStyle(Theme.danger) }
        } header: {
            Text("マトリクス表")
        } footer: {
            Text(model.canScanMatrix
                ? "カードを平らな場所に置き、表全体が写るように撮影してください。"
                : "この端末では読み取りを使えません。デスクトップ版でコピーした 70 文字を貼り付けるか、入力してください。")
        }
    }

    private var legend: some View {
        HStack(spacing: 12) {
            swatch(Theme.okSoft, "確実")
            swatch(Theme.warnSoft, "要確認")
            swatch(Theme.dangerSoft, "読めない")
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
            error = "画像を読み込めませんでした。"
            return
        }
        await scan(jpeg: jpeg)
    }

    private func scan(_ image: UIImage) async {
        guard let jpeg = image.jpegData(compressionQuality: 0.95) else { return }
        await scan(jpeg: jpeg)
    }

    private func scan(jpeg: Data) async {
        reading = true
        error = nil
        note = nil
        defer { reading = false }
        do {
            let result = try await model.scanMatrix(jpeg)
            guard result.count == 70 else {
                error = "表を見つけられませんでした。表全体が写るように撮り直してください。"
                return
            }
            cells = result.map(\.text)
            confidence = result.map(\.confidence)
            editing = true
            note = "色の付いたマスをカードと照らし合わせて確認してください。"
        } catch {
            self.error = AppModel.describe(error)
        }
    }

    private func paste(_ text: String) {
        let letters = text.uppercased().filter { !$0.isWhitespace }
        guard letters.count == 70, letters.allSatisfy({ $0.isASCII && $0.isLetter }) else {
            error = "A1…J1、A2…J2 の順に 70 文字を貼り付けてください（空白と改行は無視されます）。"
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

/// The system camera for one still photo.
private struct CameraCapture: UIViewControllerRepresentable {
    let done: (UIImage?) -> Void

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ picker: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(done: done) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let done: (UIImage?) -> Void
        init(done: @escaping (UIImage?) -> Void) { self.done = done }

        func imagePickerController(
            _ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            done(info[.originalImage] as? UIImage)
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { done(nil) }
    }
}
