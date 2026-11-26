package app.isctcompanion

import app.isctcompanion.design.AppNoticeLayer
import app.isctcompanion.design.CollapsingBar
import app.isctcompanion.design.reducedMotion
import app.isctcompanion.design.LocalReducedMotion
import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import android.app.Application
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import androidx.lifecycle.lifecycleScope
import android.net.Uri
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.LibraryBooks
import androidx.compose.material.icons.rounded.CalendarToday
import androidx.compose.material.icons.rounded.FileCopy
import androidx.compose.material.icons.rounded.GridView
import androidx.compose.material.icons.rounded.Language
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import kotlin.math.min
import androidx.compose.ui.text.style.TextAlign
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.navigation.NamedNavArgument
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.navigation
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import app.isctcompanion.core.AppModel
import androidx.compose.ui.platform.LocalContext
import app.isctcompanion.core.NotificationScheduler
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WidgetRoute
import app.isctcompanion.core.Viewer
import app.isctcompanion.core.WebRouter
import app.isctcompanion.features.handin.InboxSheet
import app.isctcompanion.features.course.CourseArchiveScreen
import app.isctcompanion.features.course.CourseFileScreen
import app.isctcompanion.design.Guide
import app.isctcompanion.design.IsctTheme
import app.isctcompanion.design.LocalDestinationScope
import app.isctcompanion.design.LocalSharedTransition
import app.isctcompanion.design.courseZoomDestination
import app.isctcompanion.design.Localizer
import app.isctcompanion.design.t
import app.isctcompanion.design.Theme
import app.isctcompanion.features.assignments.AssignmentListScreen
import app.isctcompanion.features.assignments.RecordScreen
import app.isctcompanion.features.course.AssignmentScreen
import app.isctcompanion.features.course.CourseDetailScreen
import app.isctcompanion.features.lectures.LecturesScreen
import app.isctcompanion.features.pastexams.ExamDocumentScreen
import app.isctcompanion.features.pastexams.ExamRequestScreen
import app.isctcompanion.features.pastexams.ExamSubjectEntry
import app.isctcompanion.features.pastexams.ExamSubjectScreen
import app.isctcompanion.features.pastexams.PastExamsScreen
import app.isctcompanion.features.map.CampusMapScreen
import app.isctcompanion.features.settings.SETTINGS_PAGE_ROUTE
import app.isctcompanion.features.settings.SettingsPage
import app.isctcompanion.features.settings.SettingsPageScreen
import app.isctcompanion.features.settings.SettingsScreen
import app.isctcompanion.features.settings.settingsPageRoute
import app.isctcompanion.features.timetable.TimetableScreen
import app.isctcompanion.features.today.TodayScreen
import app.isctcompanion.features.web.WebPageScreen
import app.isctcompanion.features.web.WebTabScreen
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch

/** Holds the one `AppModel` for the process, so it outlives activities. */
class IsctApplication : Application() {
    val scope = MainScope()
    val model by lazy { AppModel(this, scope) }

    override fun onCreate() {
        super.onCreate()
        Localizer.attach(this)
        Guide.attach(this)
        // The periodic wake of earlier releases; Core's directive schedules
        // one-time work now (`BackgroundWake.schedule`).
        androidx.work.WorkManager.getInstance(this).cancelUniqueWork("core-background-wake")
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) = model.foreground(true)
            override fun onStop(owner: LifecycleOwner) = model.foreground(false)
        })
    }
}

class MainActivity : ComponentActivity() {
    private var widgetLink by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        holdPhonesUpright()
        enableEdgeToEdge()
        val app = application as IsctApplication
        val model = app.model
        model.browser.attachParking(this)
        model.drive.attach(this)
        if (savedInstanceState == null) receiveShared(intent)
        widgetLink = savedInstanceState?.getString("widgetLink")
        if (savedInstanceState == null) receiveWidget(intent)
        app.scope.launch { model.start() }
        setContent {
            // The synced background theme; a change recomposes at once.
            IsctTheme(model.records.settings.optString("theme").ifEmpty { "warm" }) {
                val reduced = remember { reducedMotion(this) }
                CompositionLocalProvider(LocalModel provides model, LocalReducedMotion provides reduced) { RootView(widgetLink) { widgetLink = null } }
            }
        }
    }

    override fun onDestroy() {
        (application as IsctApplication).model.browser.detachActivity(this)
        super.onDestroy()
    }

    // Folding or unfolding recreates the activity, which asks again in
    // onCreate; this covers a size change delivered without recreating.
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        holdPhonesUpright()
    }

    /** A phone stays upright, as iPhone does; a tablet or an unfolded
     * foldable turns freely, as iPad does. Only the manifest's `portrait`
     * would also lock tablets on targetSdk 35. */
    private fun holdPhonesUpright() {
        requestedOrientation =
            if (largeScreen()) ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED else ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        receiveShared(intent)
        receiveWidget(intent)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("widgetLink", widgetLink)
        super.onSaveInstanceState(outState)
    }

    private fun receiveWidget(intent: Intent?) {
        val uri = intent?.data ?: return
        if (intent.action == Intent.ACTION_VIEW && uri.scheme == "isctcompanion" && uri.host == "widget") {
            widgetLink = uri.toString()
        }
    }

    /** Copies the current system share while its grant lasts. */
    private fun receiveShared(intent: Intent?) {
        val uris = when (intent?.action) {
            Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java))
            Intent.ACTION_SEND_MULTIPLE ->
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> return
        }
        if (uris.isNotEmpty()) lifecycleScope.launch { SharedInbox.receive(this@MainActivity, uris) }
    }
}

val LocalModel = staticCompositionLocalOf<AppModel> { error("No AppModel") }
val LocalNavigator = staticCompositionLocalOf<Navigator> { error("No navigator") }

/** Pushes onto the current tab's stack: a course, an assignment, or a school page. */
class Navigator(private val controller: NavHostController) {
    /** Opens a course; with `zoom`, it grows out of the block or card that
     * carries the same `courseZoomSource`. */
    fun openCourse(id: String, zoom: String? = null) =
        controller.navigate("course/${Uri.encode(id)}?zoom=${Uri.encode(zoom.orEmpty())}")

    fun openAssignment(id: String) = controller.navigate("assignment/${Uri.encode(id)}")

    fun openPage(page: WebRouter.PushedPage) = controller.navigate(
        "page/${page.site.name}?target=${Uri.encode(page.target.orEmpty())}&title=${Uri.encode(page.title)}&upload=${Uri.encode(page.uploadId.orEmpty())}",
    )

    var timetableDate by mutableStateOf<String?>(null)

    fun openAssignments() = controller.navigate("assignments")

    fun openRecord() = controller.navigate("record")
    fun openPastExams(courseId: String? = null) =
        controller.navigate("past-exams/detail?course=${Uri.encode(courseId.orEmpty())}") { launchSingleTop = true }

    /** A past-exam subject by its code, scoped to one class when given. */
    fun openExamSubject(code: String, classId: UInt? = null) =
        controller.navigate("past-exams/subject/${Uri.encode(code)}?class=${classId?.toString().orEmpty()}")

    /** A downloaded course file in the app's own viewer for it. */
    fun openCourseFile(path: String, title: String, viewer: Viewer) =
        controller.navigate("course-file?path=${Uri.encode(path)}&title=${Uri.encode(title)}&viewer=${viewer.name}")

    /** A downloaded ZIP's files, listed as a folder. */
    fun openCourseArchive(courseId: String, source: String, title: String) =
        controller.navigate(
            "course-archive?course=${Uri.encode(courseId)}&source=${Uri.encode(source)}&title=${Uri.encode(title)}",
        )

    /** A past-exam document, full screen. */
    fun openExamDocument(id: String) = controller.navigate("past-exams/document/${Uri.encode(id)}")

    /** One request of the board, `mine` naming which board it was listed on. */
    fun openExamRequest(id: String, mine: Boolean) =
        controller.navigate("past-exams/request/${Uri.encode(id)}?mine=$mine")

    /** Opens the campus map full screen, on a room string or on none to browse. */
    fun openMap(room: String?) = controller.navigate("map?room=${Uri.encode(room.orEmpty())}")

    /** Switches to the 設定 tab, as tapping it would, or onto an account page. */
    fun openSettings(account: SchoolAuthProfile? = null) {
        openTab("tab/settings")
        if (account != null) {
            controller.navigate(settingsPageRoute(SettingsPage.Account(account))) { launchSingleTop = true }
        }
    }

    /** A page below 設定's root, pushed inside the tab's graph. */
    internal fun openSettingsPage(page: SettingsPage) = controller.navigate(settingsPageRoute(page))

    /** Switches tabs, keeping each tab's stack. */
    fun openTab(route: String) = controller.navigate(route) {
        popUpTo(controller.graph.findStartDestination().id) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }

    fun back() = controller.popBackStack()
}

private const val PAST_EXAMS_DETAIL = "past-exams/detail?course={course}"

/** Phones have five tabs; tablets add 授業 and show all six in a side rail,
 * as the iPad and the desktop do. */
private enum class AppTab(
    val route: String, val start: String, val label: String, val icon: ImageVector, val wideOnly: Boolean = false,
) {
    Today("tab/today", "today", "common.today", Icons.Rounded.CalendarToday),
    Timetable("tab/timetable", "timetable", "common.timetable", Icons.Rounded.GridView),
    Lectures("tab/lectures", "lectures", "common.courses", Icons.AutoMirrored.Rounded.LibraryBooks, wideOnly = true),
    Web("tab/web", "web", "common.web", Icons.Rounded.Language),
    PastExams("tab/past-exams", "past-exams", "examShare.title", Icons.Rounded.FileCopy),
    Settings("tab/settings", "settings", "common.settings", Icons.Rounded.Settings),
}

/** Smallest width from which the device is a tablet, as iOS tells iPad. */
private const val WIDE_DP = 600

/** Whether this is a tablet or an unfolded foldable: the largest window the
 * display allows is at least [WIDE_DP] on its shorter side. Told by size
 * alone, never by model (AGENTS.md §3), and the same in split screen or a
 * rotated window, as iPad's idiom is. Needs the activity as context. */
private fun Context.largeScreen(): Boolean {
    val metrics = getSystemService(WindowManager::class.java).maximumWindowMetrics
    return min(metrics.bounds.width(), metrics.bounds.height()) / metrics.density >= WIDE_DP
}

@Composable
private fun RootView(widgetLink: String?, consumed: () -> Unit) {
    val model = LocalModel.current
    NotificationPermissionAsk()
    when (val status = model.status) {
        AppModel.Status.Opening -> Box(Modifier.fillMaxSize().background(Theme.colors.bg), Alignment.Center) {
            CircularProgressIndicator()
        }
        is AppModel.Status.Failed -> Column(
            Modifier.fillMaxSize().padding(Theme.space11),
            verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Icon(Icons.Rounded.WarningAmber, null, tint = Theme.colors.muted)
            Text(t("common.couldNotStart"), style = MaterialTheme.typography.titleLarge)
            Text(status.message, color = Theme.colors.muted, textAlign = TextAlign.Center)
            val scope = rememberCoroutineScope()
            TextButton(onClick = { scope.launch { model.start() } }) { Text(t("common.retry")) }
        }
        AppModel.Status.Ready -> Tabs(widgetLink, consumed)
    }
}

/** Reminder switches sync from the other devices, so this phone may have
 * reminders to post without ever having been asked: it asks once when Core
 * has some, then schedules them. */
@Composable
private fun NotificationPermissionAsk() {
    val model = LocalModel.current
    val context = LocalContext.current
    var asked by rememberSaveable { mutableStateOf(false) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) model.resyncNotifications()
    }
    LaunchedEffect(model.notificationsBlocked) {
        // Once refused, only the settings page leads on (as iOS asks only
        // while undetermined).
        if (model.notificationsBlocked && !asked && !NotificationScheduler.denied(context)) {
            asked = true
            NotificationScheduler.asking(context)
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}

@Composable
private fun Tabs(widgetLink: String?, consumed: () -> Unit) {
    val controller = rememberNavController()
    val navigator = remember(controller) { Navigator(controller) }
    val entry by controller.currentBackStackEntryAsState()
    val destination = entry?.destination
    // Courses and pages are pushed above a tab's graph, so the selected tab is
    // the nearest tab graph down the back stack. Deriving it (rather than
    // remembering the last tap) keeps it right after a system back.
    val stack by controller.currentBackStack.collectAsState()
    val current = stack.asReversed().firstNotNullOfOrNull { e ->
        AppTab.entries.firstOrNull { it.route == e.destination.parent?.route }
    } ?: AppTab.Today
    // The device's class, not this window's width: the tab set stays put in
    // split screen and only changes when a foldable folds.
    val configuration = LocalConfiguration.current
    val context = LocalContext.current
    val wide = remember(configuration) { context.largeScreen() }
    LaunchedEffect(wide, current) {
        val courseId = entry?.arguments?.getString("course")?.ifEmpty { null }
        if (!wide && current == AppTab.Lectures) {
            val tab = if (destination?.route == PAST_EXAMS_DETAIL) AppTab.PastExams else AppTab.Today
            navigator.openTab(tab.route)
            if (tab == AppTab.PastExams && courseId != null) navigator.openPastExams(courseId)
        }
    }
    val tabs = AppTab.entries.filter { wide || !it.wideOnly }
    // A pushed school page fills the screen, like a browser tab; so do the
    // campus map and a past-exam document.
    val showsBar = destination?.route?.let {
        it.startsWith("page/") || it.startsWith("map?") || it.startsWith("past-exams/document/")
    } != true
    val snackbar = remember { SnackbarHostState() }
    val model = LocalModel.current
    // A file a school page handed over opens in the app's viewer for it,
    // as a browser opens what it downloaded (WebView shows no PDF).
    val viewing = model.web.viewing
    LaunchedEffect(viewing) {
        viewing ?: return@LaunchedEffect
        model.web.viewing = null
        navigator.openCourseFile(viewing.path, viewing.name, viewing.viewer)
    }
    LaunchedEffect(widgetLink, destination != null) {
        if (widgetLink == null || destination == null) return@LaunchedEffect
        when (val route = model.widgetRoute(widgetLink)) {
            is WidgetRoute.Course -> {
                navigator.openTab(AppTab.Today.route)
                navigator.openCourse(route.courseId)
            }
            is WidgetRoute.Assignment -> {
                navigator.openTab(AppTab.Today.route)
                navigator.openAssignment(route.id)
            }
            is WidgetRoute.Exam -> {
                navigator.timetableDate = route.date
                navigator.openTab(AppTab.Timetable.route)
            }
            WidgetRoute.Timetable, WidgetRoute.UnavailableCourse -> navigator.openTab(AppTab.Timetable.route)
            WidgetRoute.Assignments, WidgetRoute.UnavailableAssignment -> {
                navigator.openTab(AppTab.Today.route)
                navigator.openAssignments()
            }
            WidgetRoute.Sync -> navigator.openSettings(SchoolAuthProfile.SCIENCE)
            null -> Unit
        }
        consumed()
    }
    // Tapping the current tab returns it to its root; for ウェブ, to the
    // shown site's start page.
    fun select(tab: AppTab) {
        if (tab == current) {
            controller.popBackStack(tab.start, inclusive = false)
            if (tab == AppTab.Web) model.web.goHome(model.web.tabSite)
        } else {
            navigator.openTab(tab.route)
        }
    }
    CompositionLocalProvider(LocalNavigator provides navigator) {
        Scaffold(
            containerColor = Theme.colors.bg,
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                // Six entries crowd a bottom bar: a tablet's sit in the side rail.
                // It folds away while a school page is read down, with the
                // page's own top bar.
                if (showsBar && !wide) {
                    CollapsingBar(!(current == AppTab.Web && model.web.barsHidden)) {
                        NavigationBar {
                            for (tab in tabs) {
                                NavigationBarItem(
                                    selected = tab == current,
                                    onClick = { select(tab) },
                                    icon = { Icon(tab.icon, null) },
                                    label = { Text(t(tab.label)) },
                                )
                            }
                        }
                    }
                }
            },
        ) { padding ->
            Row(Modifier.fillMaxSize()) {
            if (showsBar && wide) {
                NavigationRail(
                    containerColor = Theme.colors.bg,
                    modifier = Modifier.windowInsetsPadding(WindowInsets.statusBars),
                ) {
                    for (tab in tabs) {
                        NavigationRailItem(
                            selected = tab == current,
                            onClick = { select(tab) },
                            icon = { Icon(tab.icon, null) },
                            label = { Text(t(tab.label)) },
                        )
                    }
                }
            }
            Box(Modifier.weight(1f).fillMaxHeight()) {
            // Shared bounds let a course grow out of what was tapped (`design/Motion.kt`).
            SharedTransitionLayout {
            CompositionLocalProvider(LocalSharedTransition provides this) {
            Box(Modifier.fillMaxSize()) {
            NavHost(controller, startDestination = AppTab.Today.route, modifier = Modifier.padding(bottom = padding.calculateBottomPadding())) {
                tab(AppTab.Today) { TodayScreen() }
                tab(AppTab.Timetable) { TimetableScreen() }
                tab(AppTab.Lectures) { LecturesScreen() }
                tab(AppTab.Web) { WebTabScreen() }
                tab(AppTab.PastExams) { PastExamsScreen() }
                tab(AppTab.Settings, {
                    destination(SETTINGS_PAGE_ROUTE, listOf(navArgument("page") { type = NavType.StringType })) {
                        SettingsPageScreen(it.arguments?.getString("page").orEmpty())
                    }
                }) { SettingsScreen() }
                destination(
                    PAST_EXAMS_DETAIL,
                    listOf(navArgument("course") { type = NavType.StringType; defaultValue = "" }),
                ) {
                    val courseId = it.arguments?.getString("course")?.ifEmpty { null }
                    key(courseId) {
                        if (courseId == null) PastExamsScreen(onBack = { navigator.back() })
                        else ExamSubjectScreen(ExamSubjectEntry.Course(courseId))
                    }
                }
                destination(
                    "past-exams/subject/{code}?class={class}",
                    listOf(
                        navArgument("code") { type = NavType.StringType },
                        navArgument("class") { type = NavType.StringType; defaultValue = "" },
                    ),
                ) {
                    val code = it.arguments?.getString("code").orEmpty()
                    ExamSubjectScreen(ExamSubjectEntry.Code(code, it.arguments?.getString("class")?.toUIntOrNull()))
                }
                destination(
                    "course-file?path={path}&title={title}&viewer={viewer}",
                    listOf("path", "title", "viewer").map { name ->
                        navArgument(name) { type = NavType.StringType; defaultValue = "" }
                    },
                ) {
                    val arguments = it.arguments
                    CourseFileScreen(
                        arguments?.getString("path").orEmpty(),
                        arguments?.getString("title").orEmpty(),
                        runCatching { Viewer.valueOf(arguments?.getString("viewer").orEmpty()) }.getOrDefault(Viewer.EXTERNAL),
                    )
                }
                destination(
                    "course-archive?course={course}&source={source}&title={title}",
                    listOf("course", "source", "title").map { name ->
                        navArgument(name) { type = NavType.StringType; defaultValue = "" }
                    },
                ) {
                    val arguments = it.arguments
                    CourseArchiveScreen(
                        arguments?.getString("course").orEmpty(),
                        arguments?.getString("source").orEmpty(),
                        arguments?.getString("title").orEmpty(),
                    )
                }
                destination("past-exams/document/{id}", listOf(navArgument("id") { type = NavType.StringType })) {
                    ExamDocumentScreen(it.arguments?.getString("id").orEmpty())
                }
                destination(
                    "past-exams/request/{id}?mine={mine}",
                    listOf(
                        navArgument("id") { type = NavType.StringType },
                        navArgument("mine") { type = NavType.BoolType; defaultValue = false },
                    ),
                ) {
                    ExamRequestScreen(it.arguments?.getString("id").orEmpty(), it.arguments?.getBoolean("mine") ?: false)
                }
                destination(
                    "course/{id}?zoom={zoom}",
                    listOf(
                        navArgument("id") { type = NavType.StringType },
                        navArgument("zoom") { type = NavType.StringType; defaultValue = "" },
                    ),
                ) {
                    val zoom = it.arguments?.getString("zoom")?.ifEmpty { null }
                    Box(Modifier.fillMaxSize().courseZoomDestination(zoom).background(Theme.colors.bg)) {
                        CourseDetailScreen(it.arguments?.getString("id").orEmpty())
                    }
                }
                destination("assignment/{id}", listOf(navArgument("id") { type = NavType.StringType })) {
                    AssignmentScreen(it.arguments?.getString("id").orEmpty())
                }
                destination("assignments") { AssignmentListScreen() }
                destination("record") { RecordScreen() }
                destination("map?room={room}", listOf(navArgument("room") { type = NavType.StringType; defaultValue = "" })) {
                    CampusMapScreen(it.arguments?.getString("room")?.ifEmpty { null })
                }
                destination(
                    "page/{site}?target={target}&title={title}&upload={upload}",
                    listOf(
                        navArgument("site") { type = NavType.StringType },
                        navArgument("target") { type = NavType.StringType; defaultValue = "" },
                        navArgument("title") { type = NavType.StringType; defaultValue = "" },
                        navArgument("upload") { type = NavType.StringType; defaultValue = "" },
                    ),
                ) {
                    val args = it.arguments
                    val site = runCatching { SchoolSite.valueOf(args?.getString("site").orEmpty()) }
                        .getOrDefault(SchoolSite.SCIENCE_LMS)
                    WebPageScreen(
                        WebRouter.PushedPage(
                            site, args?.getString("target")?.ifEmpty { null }, args?.getString("title").orEmpty(),
                            args?.getString("upload")?.ifEmpty { null },
                        ),
                    )
                }
            }
            InboxSheet()
            }
            }
            }
            }
            }
        }
        AppNoticeLayer(snackbar, userFlow = SharedInbox.selection != null) { navigator.openSettings(SchoolAuthProfile.SCIENCE) }
    }
}

/** A tab's graph; `pages` adds destinations that belong to the tab (設定's
 * pages), so the tab stays selected on them. */
private fun NavGraphBuilder.tab(tab: AppTab, pages: NavGraphBuilder.() -> Unit = {}, content: @Composable () -> Unit) {
    navigation(startDestination = tab.start, route = tab.route) {
        destination(tab.start) { content() }
        pages()
    }
}

/** A destination whose animation scope reaches `courseZoomSource` and
 * `courseZoomDestination` inside it. */
private fun NavGraphBuilder.destination(
    route: String,
    arguments: List<NamedNavArgument> = emptyList(),
    content: @Composable (NavBackStackEntry) -> Unit,
) {
    composable(route, arguments) { entry ->
        CompositionLocalProvider(LocalDestinationScope provides this) { content(entry) }
    }
}
