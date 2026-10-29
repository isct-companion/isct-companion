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
    /// How many popups are open over each site's view (Core's `school-web-popups`).
    private(set) var popups: [SchoolSite: Int] = [:]
    /// What the web screen says about each site's page (Core's `school-web-notice`).
    private(set) var notices: [SchoolSite: Notice] = [:]
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

    /// Something about the page the person sees, shown next to it, outside
    /// the school page.
    struct Notice: Equatable {
        enum Kind: Equatable {
            /// The page did not load; `reason` is Core's `LoadFailure` wire name.
            case loadFailed(reason: String)
            /// A reload would have resent a form; the page was opened with GET.
            case reopened
        }
        let kind: Kind
        /// The address the page was going to, for Retry and Open in browser.
        let url: URL?
        /// Tells two notices about the same page apart, so one's timer
        /// never clears the next.
        let id = UUID()
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

    /// Closes the popup in front of `site`'s page.
    func closePopup(_ site: SchoolSite) { browser.closeFrontPopup(site) }

    /// Loads a failed page again, in the page the person sees.
    func retry(_ site: SchoolSite) {
        guard let url = notices.removeValue(forKey: site)?.url else { return }
        browser.retry(site, url: url)
    }

    func dismissNotice(_ site: SchoolSite) { notices[site] = nil }

    /// Core's `school-web-url`, `school-web-popups` and `school-web-notice`.
    func receive(_ name: String, payload: String) {
        guard let row = (try? JSONSerialization.jsonObject(with: Data(payload.utf8))) as? [String: Any],
            let site = Self.site(row["site"] as? String)
        else { return }
        switch name {
        case "school-web-url":
            // A page loaded: an earlier failure no longer describes it.
            if case .loadFailed = notices[site]?.kind { notices[site] = nil }
        case "school-web-popups":
            popups[site] = (row["open"] as? NSNumber)?.intValue ?? 0
        case "school-web-notice":
            let url = (row["url"] as? String).flatMap(URL.init(string:))
            switch row["kind"] as? String {
            case "load-failed":
                notices[site] = Notice(kind: .loadFailed(reason: row["reason"] as? String ?? "other"), url: url)
            case "reopened":
                let notice = Notice(kind: .reopened, url: url)
                notices[site] = notice
                Task {
                    try? await Task.sleep(for: .seconds(6))
                    if notices[site] == notice { notices[site] = nil }
                }
            default:
                break
            }
        default:
            break
        }
    }

    func back(_ site: SchoolSite) { _ = try? session?.backSchool(site: site) }
    func forward(_ site: SchoolSite) { _ = try? session?.forwardSchool(site: site) }
    func reload(_ site: SchoolSite) { _ = try? session?.reloadSchool(site: site) }

    static func name(_ site: SchoolSite) -> String {
        switch site {
        case .sciencePortal: return t("webRouter.portal")
        case .scienceLms: return "LMS"
        case .kyomu: return t("webRouter.kyomuWeb")
        case .tokyoMail: return t("webRouter.mail")
        case .library: return t("webRouter.library")
        case .tokyoPortal: return t("webRouter.tokyoTechPortal")
        case .tsubame: return "TSUBAME"
        case .syllabus: return t("common.syllabus")
        case .studyGuide: return t("webRouter.studyGuide")
        }
    }

    private static func site(_ wire: String?) -> SchoolSite? {
        [SchoolSite.sciencePortal, .scienceLms, .tokyoPortal, .tokyoMail, .kyomu, .library, .tsubame, .syllabus, .studyGuide]
            .first { wireName($0) == wire }
    }

    /// Core's wire name for a site (`SchoolSite` serde names).
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
}
