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
import kotlinx.coroutines.launch

/** Which school page is on screen. Two entries share the resident views: the
 * ウェブ tab (site switcher) and pushes from a course or assignment. Opening a
 * page asks Core (`openSchool`), which ensures and navigates the view; the
 * visibility fact goes back to Core (`schoolBrowsing`) so background sync
 * yields to it. */
class WebRouter(val browser: SchoolBrowserHost, private val scope: CoroutineScope, context: Context) {
    /** The site the ウェブ tab opens on at launch, kept on this device (設定 › ウェブ). */
    private val preferences = context.getSharedPreferences("web", Context.MODE_PRIVATE)
    var firstSite: SchoolSite by mutableStateOf(
        tabSites.firstOrNull { wireName(it) == preferences.getString(FIRST_SITE, null) } ?: SchoolSite.SCIENCE_LMS,
    )
        private set
    var tabSite by mutableStateOf(firstSite)
        private set
    val titles = mutableStateMapOf<SchoolSite, String>()
    var loading by mutableStateOf<SchoolSite?>(null)
        private set
    var error by mutableStateOf<String?>(null)
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
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            }
            if (loading == site) loading = null
        }
    }

    /** A web page appeared or disappeared on screen. */
    fun setVisible(site: SchoolSite?) {
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

    companion object {
        private const val FIRST_SITE = "firstSite"

        /** Sites in the ウェブ tab's switcher, in order. */
        val tabSites = listOf(
            SchoolSite.SCIENCE_PORTAL, SchoolSite.SCIENCE_LMS, SchoolSite.KYOMU, SchoolSite.TOKYO_MAIL, SchoolSite.LIBRARY,
            SchoolSite.TSUBAME,
        )

        fun name(site: SchoolSite): String = when (site) {
            SchoolSite.SCIENCE_PORTAL -> t("Portal")
            SchoolSite.SCIENCE_LMS -> "LMS"
            SchoolSite.KYOMU -> t("Kyomu Web")
            SchoolSite.TOKYO_MAIL -> t("Mail")
            SchoolSite.LIBRARY -> t("Library")
            SchoolSite.TOKYO_PORTAL -> t("Tokyo Tech Portal")
            SchoolSite.TSUBAME -> "TSUBAME"
            SchoolSite.SYLLABUS -> t("Syllabus")
            SchoolSite.STUDY_GUIDE -> t("Study guide")
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
