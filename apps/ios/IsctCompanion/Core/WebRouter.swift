import Foundation
import IsctCore
import Observation
import UIKit

/// Which school page is on screen. Two entries share the resident views: the
/// ウェブ tab (site picker) and pushes from a course or assignment. Opening a
/// page asks Core (`openSchool`), which ensures and navigates the view; the
/// visibility fact goes back to Core so hidden pulls yield to it.
@MainActor
@Observable
final class WebRouter {
    /// Sites in the ウェブ tab's picker, in order.
    static let tabSites: [SchoolSite] = [.sciencePortal, .scienceLms, .kyomu, .tokyoMail, .library, .tsubame]

    /// Where the site the ウェブ tab opens on is kept on this device (設定 › ウェブ).
    static let firstSiteKey = "web.firstSite"

    /// The site the ウェブ tab opens on at launch.
    static var firstSite: SchoolSite {
        get { tabSites.first { wireName($0) == UserDefaults.standard.string(forKey: firstSiteKey) } ?? .scienceLms }
        set { UserDefaults.standard.set(wireName(newValue), forKey: firstSiteKey) }
    }

    private(set) var tabSite: SchoolSite = WebRouter.firstSite
    private(set) var titles: [SchoolSite: String] = [:]
    private(set) var loading: SchoolSite?
    private(set) var error: String?
    /// Bumped when a view appears for a site so containers re-attach it.
    private(set) var attachRevision = 0
    private(set) var downloads: [URL] = []
    @ObservationIgnored private var visibleOwner: UUID?
    @ObservationIgnored weak var session: Session?
    @ObservationIgnored let browser: SchoolBrowserHost
    @ObservationIgnored private var opened: Set<SchoolSite> = []

    init(browser: SchoolBrowserHost) {
        self.browser = browser
        browser.onTitleChange = { [weak self] site, title in self?.titles[site] = title }
        browser.onOpenExternal = { url in UIApplication.shared.open(url) }
        browser.onDownload = { [weak self] file in self?.downloads.append(file) }
        browser.onDownloadError = { [weak self] error in self?.error = AppModel.describe(error) }
    }

    /// A page pushed from context (course → LMS, assignment → submission).
    struct PushedPage: Identifiable, Hashable {
        let site: SchoolSite
        let target: String?
        let title: String
        var id: String { "\(site)-\(target ?? "")" }
    }

    /// The ウェブ tab shows `site`, restoring its last page.
    func showTab(_ site: SchoolSite) {
        tabSite = site
        open(site, target: nil, restore: true)
    }

    /// Opens `site` in its resident view. A site already open keeps its page
    /// unless a target is given.
    func open(_ site: SchoolSite, target: String?, restore: Bool = false) {
        guard let session else { return }
        if restore, target == nil, opened.contains(site), browser.webView(for: site) != nil { return }
        loading = site
        error = nil
        Task {
            do {
                _ = try await session.openSchool(site: site, target: target).value()
                opened.insert(site)
                attachRevision += 1
            } catch {
                self.error = AppModel.describe(error)
            }
            if loading == site { loading = nil }
        }
    }

    /// Back to the site's own start page (Core's `start_url`), as tapping the
    /// tab or the site again does.
    func goHome(_ site: SchoolSite) {
        tabSite = site
        open(site, target: try? schoolStartUrl(site: site))
    }

    /// Picks a site in the picker; picking the shown one returns it home.
    func choose(_ site: SchoolSite) {
        if site == tabSite, opened.contains(site) {
            goHome(site)
        } else {
            showTab(site)
        }
        setVisible(site)
    }

    /// A web page appeared or disappeared on screen.
    private func setVisible(_ site: SchoolSite?) {
        _ = try? session?.schoolBrowsing(site: site)
    }

    func appear(_ site: SchoolSite, owner: UUID) {
        visibleOwner = owner
        setVisible(site)
    }

    func disappear(owner: UUID) {
        // SwiftUI may mount the next page before unmounting the old one.
        guard visibleOwner == owner else { return }
        visibleOwner = nil
        setVisible(nil)
    }

    func dismissDownload() {
        guard !downloads.isEmpty else { return }
        let file = downloads.removeFirst()
        try? FileManager.default.removeItem(at: file.deletingLastPathComponent())
    }

    func back(_ site: SchoolSite) { _ = try? session?.backSchool(site: site) }
    func forward(_ site: SchoolSite) { _ = try? session?.forwardSchool(site: site) }
    func reload(_ site: SchoolSite) { _ = try? session?.reloadSchool(site: site) }

    /// Core asks the shell to present another site (a cross-site popup).
    func handleOpenEvent(_ payload: String) {
        guard let row = Self.object(payload), let site = Self.site(row["site"] as? String) else { return }
        tabSite = site
        open(site, target: row["url"] as? String)
    }

    static func name(_ site: SchoolSite) -> String {
        switch site {
        case .sciencePortal: return t("Portal")
        case .scienceLms: return "LMS"
        case .kyomu: return t("Kyomu Web")
        case .tokyoMail: return t("Mail")
        case .library: return t("Library")
        case .tokyoPortal: return t("Tokyo Tech Portal")
        case .tsubame: return "TSUBAME"
        case .syllabus: return t("Syllabus")
        case .studyGuide: return t("Study guide")
        }
    }

    private static func object(_ json: String) -> [String: Any]? {
        guard let data = json.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    /// Core's wire name for a site, the inverse of `site(_:)`.
    static func wireName(_ site: SchoolSite) -> String {
        switch site {
        case .sciencePortal: "science-portal"
        case .scienceLms: "science-lms"
        case .tokyoPortal: "tokyo-portal"
        case .tokyoMail: "tokyo-mail"
        case .kyomu: "kyomu"
        case .library: "library"
        case .tsubame: "tsubame"
        case .syllabus: "syllabus"
        case .studyGuide: "study-guide"
        }
    }

    /// Core's wire names for sites (`SchoolSite` serde names).
    private static func site(_ raw: String?) -> SchoolSite? {
        switch raw {
        case "science-portal", "portal": return .sciencePortal
        case "science-lms", "lms": return .scienceLms
        case "tokyo-portal": return .tokyoPortal
        case "tokyo-mail": return .tokyoMail
        case "kyomu": return .kyomu
        case "library": return .library
        case "tsubame": return .tsubame
        case "syllabus": return .syllabus
        case "study-guide": return .studyGuide
        default: return nil
        }
    }
}
