package app.isctcompanion.core

import app.isctcompanion.design.t
import android.content.Context
import android.content.Intent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Which school page is on screen. Two entries share the resident views: the
 * ウェブ tab (site switcher) and pushes from a course or assignment. Opening a
 * page asks Core (`openSchool`), which ensures and navigates the view; the
 * visibility fact goes back to Core (`schoolBrowsing`) so background sync
 * yields to it. What the screen says about the page (address, popups, Back
 * and Forward, a failed load) is Core's `SchoolWebPage`, never read from the
 * views. */
class WebRouter(val browser: SchoolBrowserHost, private val scope: CoroutineScope, context: Context) {
    /** The site the ウェブ tab opens on at launch, kept on this device (設定 › ウェブ). */
    private val preferences = context.getSharedPreferences("web", Context.MODE_PRIVATE)
    var firstSite: SchoolSite by mutableStateOf(
        tabSites.firstOrNull { wireName(it) == preferences.getString(FIRST_SITE, null) } ?: SchoolSite.SCIENCE_LMS,
    )
        private set
    var tabSite by mutableStateOf(firstSite)
        private set
    /** The page the person sees of each site: the higher `revision` of
     * Core's `school-web-page` and its `schoolWebPage` snapshot. */
    val pages = mutableStateMapOf<SchoolSite, SchoolWebPage>()
    /** Sites whose reload Core opened again without resending a form
     * (`school-web-notice`), each with the job that hides the note again. */
    val reopened = mutableStateMapOf<SchoolSite, Job>()
    var loading by mutableStateOf<SchoolSite?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    /** Bumped when Core opened a page, so containers re-attach its view. */
    var attachRevision by mutableIntStateOf(0)
        private set
    var session: Session? = null
    private val opened = mutableSetOf<SchoolSite>()

    private val downloads = SchoolDownloads(context)

    init {
        // Plain HTTP and other sites Core sends out; cleartext stays off in the app.
        browser.onOpenExternal = { uri ->
            runCatching {
                context.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
        browser.onDownload = { url, referer, contentDisposition, mimeType ->
            session?.let { session ->
                scope.launch {
                    downloads.save(session, url, referer, contentDisposition, mimeType) { uri ->
                        browser.onOpenExternal?.invoke(uri)
                    }
                }
            }
        }
    }

    /** Core's school web events, from the app's event stream. */
    fun handle(name: String, payload: String) {
        val row = runCatching { JSONObject(payload) }.getOrNull() ?: return
        val site = site(row.optString("site")) ?: return
        when (name) {
            "school-web-page" -> {
                val notice = row.optJSONObject("notice")?.let {
                    SchoolWebFailure(it.text("url"), loadFailure(it.optString("reason")))
                }
                adopt(
                    SchoolWebPage(
                        site, row.optLong("revision").toULong(), row.text("url"), row.optLong("popups").toULong(),
                        step(row.optString("back")), step(row.optString("forward")), notice,
                    ),
                )
            }
            "school-web-notice" -> if (row.optString("kind") == "reopened") {
                reopened.remove(site)?.cancel()
                reopened[site] = scope.launch {
                    delay(6_000)
                    reopened.remove(site)
                }
            }
        }
    }

    /** Core's `schoolWebPage` snapshot of `site`. Events are already
     * streaming, so whichever of the two is newer wins. */
    private suspend fun readPage(site: SchoolSite) {
        val session = session ?: return
        val page = runCatching { session.schoolWebPage(site).value() as? OperationResult.SchoolWebPage }.getOrNull()
        page?.let { adopt(it.page) }
    }

    /** Keeps `page` unless a newer one of its site is already here. */
    private fun adopt(page: SchoolWebPage) {
        val held = pages[page.site]
        if (held == null || page.revision > held.revision) pages[page.site] = page
    }

    /** The pages read before the event stream lost events, read again. */
    fun refreshPages() {
        for (site in pages.keys.toList()) scope.launch { readPage(site) }
    }

    /** A page pushed from context (course → LMS, assignment → submission). */
    data class PushedPage(val site: SchoolSite, val target: String?, val title: String)

    /** The ウェブ tab shows `site`, restoring its last page. */
    fun showTab(site: SchoolSite) {
        tabSite = site
        open(site, target = null, restore = true)
    }

    /** 設定 › ウェブ › 最初のサイト; takes effect at the next launch. */
    fun chooseFirstSite(site: SchoolSite) {
        firstSite = site
        preferences.edit().putString(FIRST_SITE, wireName(site)).apply()
    }

    /** Back to the site's own start page (Core's `schoolStartUrl`), as tapping
     * the tab or the site again does. */
    fun goHome(site: SchoolSite) {
        tabSite = site
        open(site, target = runCatching { schoolStartUrl(site) }.getOrNull())
    }

    /** Picks a site in the switcher; picking the shown one returns it home. */
    fun choose(site: SchoolSite) {
        if (site == tabSite && site in opened) goHome(site) else showTab(site)
        setVisible(site)
    }

    /** Opens `site` in its resident view. A site already open keeps its page
     * unless a target is given. */
    fun open(site: SchoolSite, target: String?, restore: Boolean = false) {
        val session = session ?: return
        if (restore && target == null && site in opened && browser.webView(site) != null) return
        loading = site
        error = null
        scope.launch {
            try {
                session.openSchool(site, target).value()
                opened.add(site)
                attachRevision++
                readPage(site)
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            }
            if (loading == site) loading = null
        }
    }

    /** A web page appeared or disappeared on screen. Core publishes
     * `school-web-page` only for the site on screen, so once it is, the
     * snapshot fills in what changed while it was not. */
    fun setVisible(site: SchoolSite?) {
        val browsing = runCatching { session?.schoolBrowsing(site) }.getOrNull() ?: return
        site ?: return
        scope.launch {
            runCatching { browsing.value() }
            readPage(site)
        }
    }

    fun back(site: SchoolSite) {
        runCatching { session?.backSchool(site) }
    }

    fun forward(site: SchoolSite) {
        runCatching { session?.forwardSchool(site) }
    }

    fun reload(site: SchoolSite) {
        runCatching { session?.reloadSchool(site) }
    }

    /** The notice's Reopen: the engine's reload of the failed entry, which
     * clears the failure in Core. A form result WebView would not show again
     * asks onFormResubmission, and Core opens it with GET. */
    fun retry(site: SchoolSite) {
        reload(site)
    }

    /** The close bar over a popup; Core hears it closed and closes its own. */
    fun closePopup(site: SchoolSite) {
        browser.closeFront(site)
    }

    companion object {
        private const val FIRST_SITE = "firstSite"

        /** Sites in the ウェブ tab's switcher, in order. */
        val tabSites = listOf(
            SchoolSite.SCIENCE_PORTAL, SchoolSite.SCIENCE_LMS, SchoolSite.KYOMU, SchoolSite.TOKYO_MAIL, SchoolSite.LIBRARY,
            SchoolSite.TSUBAME,
        )

        fun name(site: SchoolSite): String = when (site) {
            SchoolSite.SCIENCE_PORTAL -> t("webRouter.portal")
            SchoolSite.SCIENCE_LMS -> "LMS"
            SchoolSite.KYOMU -> t("webRouter.kyomuWeb")
            SchoolSite.TOKYO_MAIL -> t("webRouter.mail")
            SchoolSite.LIBRARY -> t("webRouter.library")
            SchoolSite.TOKYO_PORTAL -> t("webRouter.tokyoTechPortal")
            SchoolSite.TSUBAME -> "TSUBAME"
            SchoolSite.SYLLABUS -> t("common.syllabus")
            SchoolSite.STUDY_GUIDE -> t("webRouter.studyGuide")
        }

        fun site(raw: String?): SchoolSite? = SchoolSite.entries.firstOrNull { wireName(it) == raw }

        /** Core's `SchoolWebStep` from its wire name. */
        private fun step(raw: String): SchoolWebStep = when (raw) {
            "yes" -> SchoolWebStep.YES
            "no" -> SchoolWebStep.NO
            else -> SchoolWebStep.UNKNOWN
        }

        /** Core's `LoadFailure` from its wire name. */
        private fun loadFailure(raw: String): LoadFailure = when (raw) {
            "cancelled" -> LoadFailure.CANCELLED
            "offline" -> LoadFailure.OFFLINE
            "host-not-found" -> LoadFailure.HOST_NOT_FOUND
            "unreachable" -> LoadFailure.UNREACHABLE
            "insecure" -> LoadFailure.INSECURE
            "cleartext" -> LoadFailure.CLEARTEXT
            else -> LoadFailure.OTHER
        }

        /** Core's wire name for a site (`SchoolSite` serde names). */
        fun wireName(site: SchoolSite): String = when (site) {
            SchoolSite.SCIENCE_PORTAL -> "science-portal"
            SchoolSite.SCIENCE_LMS -> "science-lms"
            SchoolSite.TOKYO_PORTAL -> "tokyo-portal"
            SchoolSite.TOKYO_MAIL -> "tokyo-mail"
            SchoolSite.KYOMU -> "kyomu"
            SchoolSite.LIBRARY -> "library"
            SchoolSite.TSUBAME -> "tsubame"
            SchoolSite.SYLLABUS -> "syllabus"
            SchoolSite.STUDY_GUIDE -> "study-guide"
        }
    }
}
