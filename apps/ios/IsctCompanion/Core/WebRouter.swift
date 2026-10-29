import Foundation
import IsctCore
import Observation
import UIKit

/// Which school page is on screen. Two entries share the resident views: the
/// ウェブ tab (site picker) and pushes from a course or assignment. Opening a
/// page asks Core (`openSchool`), which ensures and navigates the view; the
/// visibility fact goes back to Core so hidden pulls yield to it. What the
/// screen says about the page (address, popups, Back and Forward, a failed
/// load) is Core's `SchoolWebPage`, never read from the views.
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
    private(set) var loading: SchoolSite?
    private(set) var error: String?
    /// Bumped when a view appears for a site so containers re-attach it.
    private(set) var attachRevision = 0
    private(set) var downloads: [URL] = []
    /// The page the person sees of each site: the higher `revision` of Core's
    /// `school-web-page` and its `schoolWebPage` snapshot.
    private(set) var pages: [SchoolSite: SchoolWebPage] = [:]
    /// A reload Core opened again without resending a form (`school-web-notice`),
    /// shown for a while. Tells two notes apart, so one's timer never clears
    /// the next.
    private(set) var reopened: [SchoolSite: UUID] = [:]
    /// The failed load the person closed for each site, hidden until Core's
    /// notice changes.
    private var dismissed: [SchoolSite: SchoolWebFailure] = [:]
    @ObservationIgnored private var visibleOwner: UUID?
    @ObservationIgnored weak var session: Session?
    @ObservationIgnored let browser: SchoolBrowserHost
    @ObservationIgnored private var opened: Set<SchoolSite> = []

    init(browser: SchoolBrowserHost) {
        self.browser = browser
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
                await readPage(site)
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

    /// A web page appeared or disappeared on screen. Core publishes
    /// `school-web-page` only for the site on screen, so once it is, the
    /// snapshot fills in what changed while it was not.
    private func setVisible(_ site: SchoolSite?) {
        guard let browsing = try? session?.schoolBrowsing(site: site), let site else { return }
        Task {
            _ = try? await browsing.value()
            await readPage(site)
        }
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

    /// Core's `schoolWebPage` snapshot of `site`. Events are already
    /// streaming, so whichever of the two is newer wins.
    private func readPage(_ site: SchoolSite) async {
        guard let session, case let .schoolWebPage(page)? = try? await session.schoolWebPage(site: site).value()
        else { return }
        adopt(page)
    }

    /// Keeps `page` unless a newer one of its site is already here.
    private func adopt(_ page: SchoolWebPage) {
        if let held = pages[page.site], held.revision >= page.revision { return }
        pages[page.site] = page
        if let closed = dismissed[page.site], closed != page.notice { dismissed[page.site] = nil }
    }

    /// The pages read before the event stream lost events, read again.
    func refreshPages() {
        for site in pages.keys { Task { await readPage(site) } }
    }

    /// The failed load the web screen shows for `site`, unless the person
    /// closed it.
    func failure(_ site: SchoolSite) -> SchoolWebFailure? {
        guard let notice = pages[site]?.notice, notice != dismissed[site] else { return nil }
        return notice
    }

    /// The address of the page the person sees, for Share and Open in browser.
    func url(_ site: SchoolSite) -> URL? {
        pages[site]?.url.flatMap(URL.init(string:))
    }

    func dismissDownload() {
        guard !downloads.isEmpty else { return }
        let file = downloads.removeFirst()
        try? FileManager.default.removeItem(at: file.deletingLastPathComponent())
    }

    /// Closes the popup in front of `site`'s page.
    func closePopup(_ site: SchoolSite) { browser.closeFrontPopup(site) }

    /// Loads a failed page again, in the page the person sees. The notice
    /// stays until Core hears how it went.
    func retry(_ site: SchoolSite) {
        guard let url = failure(site)?.url.flatMap(URL.init(string:)) else { return }
        browser.retry(site, url: url)
    }

    func dismissFailure(_ site: SchoolSite) { dismissed[site] = pages[site]?.notice }

    /// Core's `school-web-page` and `school-web-notice`.
    func receive(_ name: String, payload: String) {
        guard let row = (try? JSONSerialization.jsonObject(with: Data(payload.utf8))) as? [String: Any],
            let site = Self.site(row["site"] as? String)
        else { return }
        switch name {
        case "school-web-page":
            guard let revision = (row["revision"] as? NSNumber)?.uint64Value else { return }
            let notice = (row["notice"] as? [String: Any]).map {
                SchoolWebFailure(url: $0["url"] as? String, reason: Self.loadFailure($0["reason"] as? String))
            }
            adopt(SchoolWebPage(
                site: site, revision: revision, url: row["url"] as? String,
                popups: (row["popups"] as? NSNumber)?.uint64Value ?? 0,
                back: Self.step(row["back"] as? String), forward: Self.step(row["forward"] as? String),
                notice: notice))
        case "school-web-notice":
            guard row["kind"] as? String == "reopened" else { return }
            let note = UUID()
            reopened[site] = note
            Task {
                try? await Task.sleep(for: .seconds(6))
                if reopened[site] == note { reopened[site] = nil }
            }
        default:
            break
        }
    }

    /// Core's `SchoolWebStep` from its wire name.
    private static func step(_ wire: String?) -> SchoolWebStep {
        switch wire {
        case "yes": .yes
        case "no": .no
        default: .unknown
        }
    }

    /// Core's `LoadFailure` from its wire name.
    private static func loadFailure(_ wire: String?) -> LoadFailure {
        switch wire {
        case "cancelled": .cancelled
        case "offline": .offline
        case "host-not-found": .hostNotFound
        case "unreachable": .unreachable
        case "insecure": .insecure
        case "cleartext": .cleartext
        default: .other
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
