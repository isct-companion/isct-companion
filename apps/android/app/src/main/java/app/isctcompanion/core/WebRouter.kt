package app.isctcompanion.core

import android.content.Context
import android.content.Intent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Which school page is on screen. Two entries share the resident views: the
 * ウェブ tab (site switcher) and pushes from a course or assignment. Opening a
 * page asks Core (`openSchool`), which ensures and navigates the view; the
 * visibility fact goes back to Core so hidden pulls yield to it. */
class WebRouter(val browser: SchoolBrowserHost, private val scope: CoroutineScope, context: Context) {
    var tabSite by mutableStateOf(SchoolSite.SCIENCE_LMS)
        private set
    val titles = mutableStateMapOf<SchoolSite, String>()
    var loading by mutableStateOf<SchoolSite?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var isVisible by mutableStateOf(false)
        private set
    /** Bumped when Core opened a page, so containers re-attach its view. */
    var attachRevision by mutableIntStateOf(0)
        private set
    var session: Session? = null
    private val opened = mutableSetOf<SchoolSite>()

    init {
        browser.onTitleChange = { site, title -> titles[site] = title }
        browser.onOpenExternal = { uri ->
            context.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    /** A page pushed from context (course → T2SCHOLA, assignment → submission). */
    data class PushedPage(val site: SchoolSite, val target: String?, val title: String)

    /** The ウェブ tab shows `site`, restoring its last page. */
    fun showTab(site: SchoolSite) {
        tabSite = site
        open(site, target = null, restore = true)
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
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            }
            if (loading == site) loading = null
        }
    }

    /** A web page appeared or disappeared on screen. */
    fun setVisible(site: SchoolSite?) {
        isVisible = site != null
        runCatching { session?.schoolBrowsing(site) }
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

    /** Core asks the shell to present another site (a cross-site popup). */
    fun handleOpenEvent(payload: String) {
        val row = runCatching { JSONObject(payload) }.getOrNull() ?: return
        val site = site(row.text("site")) ?: return
        tabSite = site
        open(site, row.text("url"))
    }

    companion object {
        /** Sites in the ウェブ tab's switcher, in order. */
        val tabSites = listOf(
            SchoolSite.SCIENCE_PORTAL, SchoolSite.SCIENCE_LMS, SchoolSite.KYOMU, SchoolSite.TOKYO_MAIL, SchoolSite.LIBRARY,
        )

        fun name(site: SchoolSite): String = when (site) {
            SchoolSite.SCIENCE_PORTAL -> "ポータル"
            SchoolSite.SCIENCE_LMS -> "T2SCHOLA"
            SchoolSite.KYOMU -> "教務Web"
            SchoolSite.TOKYO_MAIL -> "メール"
            SchoolSite.LIBRARY -> "図書館"
            SchoolSite.TOKYO_PORTAL -> "東工大ポータル"
            SchoolSite.TSUBAME -> "TSUBAME"
            SchoolSite.SYLLABUS -> "シラバス"
            SchoolSite.STUDY_GUIDE -> "学修案内"
        }

        /** Core's wire names for sites (`SchoolSite` serde names). */
        fun site(raw: String?): SchoolSite? = when (raw) {
            "science-portal", "portal" -> SchoolSite.SCIENCE_PORTAL
            "science-lms", "lms" -> SchoolSite.SCIENCE_LMS
            "tokyo-portal" -> SchoolSite.TOKYO_PORTAL
            "tokyo-mail" -> SchoolSite.TOKYO_MAIL
            "kyomu" -> SchoolSite.KYOMU
            "library" -> SchoolSite.LIBRARY
            "tsubame" -> SchoolSite.TSUBAME
            "syllabus" -> SchoolSite.SYLLABUS
            "study-guide" -> SchoolSite.STUDY_GUIDE
            else -> null
        }
    }
}
