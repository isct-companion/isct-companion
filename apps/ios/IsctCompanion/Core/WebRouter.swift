import Foundation
import IsctCore
import Observation
import UIKit

/// Which school page is on screen. Two entries share the resident views: the
/// ウェブ tab (site switcher) and pushes from a course or assignment. Opening a
/// page asks Core (`openSchool`), which ensures and navigates the view; the
/// visibility fact goes back to Core so hidden pulls yield to it.
@MainActor
@Observable
final class WebRouter {
    /// Sites in the ウェブ tab's switcher, in order.
    static let tabSites: [SchoolSite] = [.sciencePortal, .scienceLms, .kyomu, .tokyoMail, .library]

    private(set) var tabSite: SchoolSite = .scienceLms
    private(set) var titles: [SchoolSite: String] = [:]
    private(set) var loading: SchoolSite?
    private(set) var error: String?
    private(set) var isVisible = false
    /// Bumped when a view appears for a site so containers re-attach it.
    private(set) var attachRevision = 0
    @ObservationIgnored weak var session: Session?
    @ObservationIgnored let browser: SchoolBrowserHost
    @ObservationIgnored private var opened: Set<SchoolSite> = []

    init(browser: SchoolBrowserHost) {
        self.browser = browser
        browser.onTitleChange = { [weak self] site, title in self?.titles[site] = title }
        browser.onOpenExternal = { url in UIApplication.shared.open(url) }
    }

    /// A page pushed from context (course → T2SCHOLA, assignment → submission).
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

    /// Picks a site in the switcher; picking the shown one returns it home.
    func choose(_ site: SchoolSite) {
        if site == tabSite, opened.contains(site) {
            goHome(site)
        } else {
            showTab(site)
        }
        setVisible(site)
    }

    /// A web page appeared or disappeared on screen.
    func setVisible(_ site: SchoolSite?) {
        isVisible = site != nil
        _ = try? session?.schoolBrowsing(site: site)
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
        case .sciencePortal: return "ポータル"
        case .scienceLms: return "T2SCHOLA"
        case .kyomu: return "教務Web"
        case .tokyoMail: return "メール"
        case .library: return "図書館"
        case .tokyoPortal: return "東工大ポータル"
        case .tsubame: return "TSUBAME"
        case .syllabus: return "シラバス"
        case .studyGuide: return "学修案内"
        }
    }

    private static func object(_ json: String) -> [String: Any]? {
        guard let data = json.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
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
