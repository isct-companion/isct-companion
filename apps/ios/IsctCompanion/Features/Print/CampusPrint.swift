import Foundation
import IsctCore
import Observation
import PDFKit
import SwiftUI
import UIKit
import UserNotifications

/// A file to open the print screen on: one a reader shows, or one the
/// "Print via campus printer" action handed over (`transfer`, discarded
/// when the screen closes).
struct CampusPrintRequest: Identifiable {
    let id = UUID()
    let url: URL
    let name: String
    var transfer: SharedInbox.Transfer?

    /// The print the action handed over, if one waits.
    static func shared() -> CampusPrintRequest? {
        guard let transfer = SharedInbox.take(from: .print) else { return nil }
        guard let file = transfer.files.first else {
            transfer.discard()
            return nil
        }
        return CampusPrintRequest(url: file.url(in: transfer.folder), name: file.name, transfer: transfer)
    }
}

extension EnvironmentValues {
    /// Opens the print screen on a file and its name for the person: a
    /// reader's print button.
    @Entry var openCampusPrint: (URL, String) -> Void = { _, _ in }
}

/// The shared or opened file as the print screen reads it. `pages` is 0
/// when a PDF's pages cannot be counted (one locked with a password): it
/// prints whole.
struct PrintDocument {
    enum Kind {
        case pdf(PDFDocument?)
        case image(UIImage)
    }

    let url: URL
    let name: String
    let kind: Kind
    let pages: Int

    var isPDF: Bool {
        if case .pdf = kind { return true }
        return false
    }

    /// A PDF or a picture; `nil` for anything else.
    static func load(_ url: URL, name: String) -> PrintDocument? {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? handle.close() }
        guard let head = try? handle.read(upToCount: 1024) else { return nil }
        if head.range(of: Data("%PDF-".utf8)) != nil {
            let pdf = PDFDocument(url: url)
            let pages = (pdf?.isLocked ?? true) ? 0 : (pdf?.pageCount ?? 0)
            return PrintDocument(url: url, name: name, kind: .pdf(pages > 0 ? pdf : nil), pages: pages)
        }
        guard let image = UIImage(contentsOfFile: url.path) else { return nil }
        return PrintDocument(url: url, name: name, kind: .image(image), pages: 1)
    }
}

/// How a picture is put on its page: the paper, which way the page lies,
/// and how many quarter turns clockwise the person gave the picture.
struct ImageLayout: Equatable {
    var media: CampusPrintMedia
    var landscape: Bool
    var turns: Int

    /// The page in points.
    var page: CGSize {
        let (width, height): (CGFloat, CGFloat) = switch media {
        case .a4: (595, 842)
        case .letter: (612, 792)
        }
        return landscape ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
    }

    /// The picture upright, then turned.
    static func turned(_ image: UIImage, _ turns: Int) -> UIImage {
        let quarter = ((turns % 4) + 4) % 4
        let upright = image.size
        let size = quarter % 2 == 0 ? upright : CGSize(width: upright.height, height: upright.width)
        let format = UIGraphicsImageRendererFormat()
        format.scale = image.scale
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            let canvas = context.cgContext
            canvas.translateBy(x: size.width / 2, y: size.height / 2)
            canvas.rotate(by: CGFloat(quarter) * .pi / 2)
            image.draw(in: CGRect(x: -upright.width / 2, y: -upright.height / 2, width: upright.width, height: upright.height))
        }
    }

    /// `size` scaled to fit inside `page` less `margin` on every side,
    /// centred.
    static func fit(_ size: CGSize, in page: CGSize, margin: CGFloat) -> CGRect {
        let room = CGSize(width: max(page.width - 2 * margin, 1), height: max(page.height - 2 * margin, 1))
        let scale = min(room.width / max(size.width, 1), room.height / max(size.height, 1))
        let drawn = CGSize(width: size.width * scale, height: size.height * scale)
        return CGRect(x: (page.width - drawn.width) / 2, y: (page.height - drawn.height) / 2, width: drawn.width, height: drawn.height)
    }
}

/// A document larger than Core takes (`maxDocumentBytes`): not read whole,
/// and not sent.
struct DocumentTooLarge: Error {}

extension PrintDocument {
    /// Whether the file is larger than Core takes, by its size on disk.
    func tooLarge(_ limit: UInt64) -> Bool {
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize else { return false }
        return UInt64(size) > limit
    }

    /// The bytes to send: a PDF as it is, read at most one byte past
    /// `limit` so a larger one is refused without being read whole; a
    /// picture as `layout` puts it, as large as fits inside the printer's
    /// margin and centred on one page.
    func bytes(_ layout: ImageLayout, marginMils: Int32, limit: UInt64) throws -> Data? {
        switch kind {
        case .pdf:
            guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
            defer { try? handle.close() }
            guard let data = try? handle.read(upToCount: Int(clamping: limit) + 1) else { return nil }
            if UInt64(data.count) > limit { throw DocumentTooLarge() }
            return data
        case let .image(image):
            let picture = ImageLayout.turned(image, layout.turns)
            let page = CGRect(origin: .zero, size: layout.page)
            let margin = CGFloat(marginMils) * 72 / 1000
            return UIGraphicsPDFRenderer(bounds: page).pdfData { context in
                context.beginPage()
                picture.draw(in: ImageLayout.fit(picture.size, in: page.size, margin: margin))
            }
        }
    }
}

/// One send at a time, in the app rather than the screen, so closing the
/// screen does not stop it. iOS gives a backgrounded app a short while to
/// finish (`beginBackgroundTask`). A send that ends off the print screen
/// is shown by the app until dismissed (`unseen`), and one that ends while
/// the app is in the background also says how in a notification.
@MainActor
@Observable
final class CampusPrintRun {
    enum State: Equatable {
        case idle
        case sending(title: String)
        case ended(title: String, printed: Bool, message: String)
    }

    static let shared = CampusPrintRun()
    private(set) var state = State.idle
    /// A send that ended while the print screen was not on screen: the app
    /// shows it until the person dismisses it, so the result is not lost
    /// when the screen was closed and the app stayed open.
    var unseen: State?
    /// Set while the print screen is on screen, where the result shows.
    var watching = false

    private init() {}

    func forget() {
        if case .ended = state { state = .idle }
    }

    /// `document` reads the bytes to send; it throws `DocumentTooLarge`
    /// past Core's limit.
    func start(_ model: AppModel, job: CampusPrintJob, document: @escaping @Sendable () throws -> Data?) {
        if case .sending = state { return }
        state = .sending(title: job.title)
        let background = UIApplication.shared.beginBackgroundTask(withName: "campus-print")
        Task {
            let read = await Task.detached(priority: .userInitiated) { Result { try document() } }.value
            let ended: State
            switch read {
            case let .success(bytes?):
                if let report = await model.campusPrint(bytes, job: job) {
                    ended = .ended(title: job.title, printed: report.printed, message: Self.text(report.message))
                } else {
                    ended = .ended(title: job.title, printed: false, message: t("campusPrint.notice.queueFailed"))
                }
            case .failure(is DocumentTooLarge):
                ended = .ended(title: job.title, printed: false, message: Self.tooLargeText)
            default:
                ended = .ended(title: job.title, printed: false, message: t("campusPrint.window.couldNotReadTheFile"))
            }
            state = ended
            // On screen, the print screen shows the result; off it, the app
            // shows it until dismissed. In the background a notification
            // says it too.
            if !watching { unseen = ended }
            if UIApplication.shared.applicationState != .active { await notify(ended) }
            if background != .invalid { UIApplication.shared.endBackgroundTask(background) }
        }
    }

    /// Core's words for a document past its limit.
    static var tooLargeText: String {
        let limit = (try? campusPrinterCapabilities())?.maxDocumentBytes ?? 64 * 1024 * 1024
        return t("campusPrint.notice.tooLarge", ["megabytes": limit / (1024 * 1024)])
    }

    private func notify(_ ended: State) async {
        guard case let .ended(title, _, message) = ended,
              await NotificationScheduler.allowed() else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = message
        content.sound = .default
        try? await UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: "campus-print", content: content, trigger: nil))
    }

    /// Core's words for how a print ended or why it would not go through.
    static func text(_ message: CampusPrintMessage) -> String {
        t(message.key, message.args.mapValues { $0 as Any })
    }
}

/// "1-3, 5" as page numbers, or `nil` if it is not a valid list within
/// `total` pages. Spaces, full-width commas and en dashes are taken.
func parsePages(_ text: String, total: Int) -> Set<Int>? {
    var pages = Set<Int>()
    let parts = text.split(whereSeparator: { ",，、 \t\n".contains($0) })
    for part in parts {
        let bounds = part.split(omittingEmptySubsequences: false, whereSeparator: { "-–~〜".contains($0) })
        guard (1...2).contains(bounds.count),
              let first = Int(bounds[0]),
              let last = Int(bounds[bounds.count - 1]),
              first >= 1, last <= total, first <= last
        else { return nil }
        pages.formUnion(first...last)
    }
    return pages.isEmpty ? nil : pages
}

/// Page numbers as runs: {1, 2, 3, 5} is "1-3, 5".
func formatPages(_ pages: Set<Int>) -> String {
    let sorted = pages.sorted()
    var parts: [String] = []
    var index = 0
    while index < sorted.count {
        var end = index
        while end + 1 < sorted.count, sorted[end + 1] == sorted[end] + 1 { end += 1 }
        parts.append(end > index ? "\(sorted[index])-\(sorted[end])" : "\(sorted[index])")
        index = end + 1
    }
    return parts.joined(separator: ", ")
}
