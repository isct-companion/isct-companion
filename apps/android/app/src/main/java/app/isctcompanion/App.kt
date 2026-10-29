package app.isctcompanion

import app.isctcompanion.design.reducedMotion
import app.isctcompanion.design.LocalReducedMotion
import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import android.app.Application
import android.content.Intent
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.LibraryBooks
import androidx.compose.material.icons.rounded.CalendarToday
import androidx.compose.material.icons.rounded.GridView
import androidx.compose.material.icons.rounded.Language
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
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
import app.isctcompanion.core.TlsVerifier
import app.isctcompanion.core.WebRouter
import app.isctcompanion.features.handin.InboxTray
import app.isctcompanion.design.Guide
import app.isctcompanion.design.IsctTheme
import app.isctcompanion.design.LocalDestinationScope
import app.isctcompanion.design.LocalSharedTransition
import app.isctcompanion.design.courseZoomDestination
import app.isctcompanion.design.Localizer
import app.isctcompanion.design.t
import app.isctcompanion.design.Theme
import app.isctcompanion.features.assignments.RecordScreen
import app.isctcompanion.features.course.AssignmentScreen
import app.isctcompanion.features.course.CourseDetailScreen
import app.isctcompanion.features.lectures.LecturesScreen
import app.isctcompanion.features.settings.SettingsScreen
import app.isctcompanion.features.settings.WelcomeSheet
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
        // Before the session opens: Core's HTTPS verifies through Android.
        TlsVerifier.install(this)
        scope.launch { model.start() }
        BackgroundWake.schedule(this)
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) = model.foreground(true)
            override fun onStop(owner: LifecycleOwner) = model.foreground(false)
        })
    }
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val model = (application as IsctApplication).model
        model.browser.attachParking(this)
        model.drive.attach(this)
        receiveShared(intent)
        setContent {
            // The synced background theme; a change recomposes at once.
            IsctTheme(model.records.settings.optString("theme").ifEmpty { "warm" }) {
                val reduced = remember { reducedMotion(this) }
                CompositionLocalProvider(LocalModel provides model, LocalReducedMotion provides reduced) { RootView() }
            }
        }
    }

    override fun onDestroy() {
        (application as IsctApplication).model.browser.detachActivity(this)
        super.onDestroy()
    }

    override fun onResume() {
        super.onResume()
        lifecycleScope.launch(Dispatchers.IO) { SharedInbox.refresh(this@MainActivity) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        receiveShared(intent)
    }

    /** Copies what another app shared into the inbox while its grant lasts. */
    private fun receiveShared(intent: Intent?) {
        val uris = when (intent?.action) {
            Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java))
            Intent.ACTION_SEND_MULTIPLE ->
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> return
        }
        if (uris.isNotEmpty()) lifecycleScope.launch(Dispatchers.IO) { SharedInbox.receive(this@MainActivity, uris) }
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
        "page/${page.site.name}?target=${Uri.encode(page.target.orEmpty())}&title=${Uri.encode(page.title)}",
    )

    fun openRecord() = controller.navigate("record")

    /** The account page 設定 should open on, taken (and cleared) by it. */
    var settingsAccount by mutableStateOf<SchoolAuthProfile?>(null)

    /** Switches to the 設定 tab, as tapping it would, or onto an account page. */
    fun openSettings(account: SchoolAuthProfile? = null) {
        settingsAccount = account
        controller.navigate("tab/settings") {
            popUpTo(controller.graph.findStartDestination().id) { saveState = true }
            launchSingleTop = true
            restoreState = true
        }
    }

    fun back() = controller.popBackStack()
}

/** Assignments live on 今日, and 設定 is the last tab. 授業 is a tab only on
 * wide windows (tablets), as on iPad; a phone reaches courses from 今日 and
 * 時間割. */
private enum class AppTab(
    val route: String, val start: String, val label: String, val icon: ImageVector, val wideOnly: Boolean = false,
) {
    Today("tab/today", "today", "common.today", Icons.Rounded.CalendarToday),
    Timetable("tab/timetable", "timetable", "common.timetable", Icons.Rounded.GridView),
    Lectures("tab/lectures", "lectures", "common.courses", Icons.AutoMirrored.Rounded.LibraryBooks, wideOnly = true),
    Web("tab/web", "web", "common.web", Icons.Rounded.Language),
    Settings("tab/settings", "settings", "common.settings", Icons.Rounded.Settings),
}

/** Window width from which the layout is a tablet's, as iOS tells iPad. */
private const val WIDE_DP = 600

@Composable
private fun RootView() {
    val model = LocalModel.current
    NotificationPermissionAsk()
    when (val status = model.status) {
        AppModel.Status.Opening -> Box(Modifier.fillMaxSize().background(Theme.colors.bg), Alignment.Center) {
            CircularProgressIndicator()
        }
        is AppModel.Status.Failed -> Column(
            Modifier.fillMaxSize().padding(32.dp),
            verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Icon(Icons.Rounded.WarningAmber, null, tint = Theme.colors.muted)
            Text(t("common.couldNotStart"), style = MaterialTheme.typography.titleLarge)
            Text(status.message, color = Theme.colors.muted, textAlign = TextAlign.Center)
        }
        AppModel.Status.Ready -> Tabs()
    }
}

/** Reminder switches sync from the other devices, so this phone may have
 * reminders to post without ever having been asked: it asks once when Core
 * has some, then schedules them. */
@Composable
private fun NotificationPermissionAsk() {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var asked by rememberSaveable { mutableStateOf(false) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) scope.launch { model.resyncNotifications() }
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
private fun Tabs() {
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
    // A pushed school page fills the screen, like a browser tab.
    val showsBar = destination?.route?.startsWith("page/") != true
    val snackbar = remember { SnackbarHostState() }
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    // Hiding offers its undo in the platform snackbar, above the navigation bar.
    LaunchedEffect(model.undo) {
        val undo = model.undo ?: return@LaunchedEffect
        val result = snackbar.showSnackbar(
            t("app.hidTitle", "title" to undo.title), actionLabel = t("common.undo"), duration = SnackbarDuration.Short,
        )
        // undoHide clears `undo`, which restarts this effect; run it outside so
        // the restart cannot cancel the mutation.
        if (result == SnackbarResult.ActionPerformed) scope.launch { model.undoHide() } else model.dismissUndo()
    }
    CompositionLocalProvider(LocalNavigator provides navigator) {
        Scaffold(
            containerColor = Theme.colors.bg,
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                if (showsBar) {
                    NavigationBar {
                        val wide = LocalConfiguration.current.screenWidthDp >= WIDE_DP
                        for (tab in AppTab.entries.filter { wide || !it.wideOnly }) {
                            NavigationBarItem(
                                selected = tab == current,
                                onClick = {
                                    if (tab == current) {
                                        // Tapping the current tab returns it to its root;
                                        // for ウェブ, to the shown site's start page.
                                        controller.popBackStack(tab.start, inclusive = false)
                                        if (tab == AppTab.Web) model.web.goHome(model.web.tabSite)
                                    } else {
                                        controller.navigate(tab.route) {
                                            popUpTo(controller.graph.findStartDestination().id) { saveState = true }
                                            launchSingleTop = true
                                            restoreState = true
                                        }
                                    }
                                },
                                icon = { Icon(tab.icon, null) },
                                label = { Text(t(tab.label)) },
                            )
                        }
                    }
                }
            },
        ) { padding ->
            // Shared bounds let a course grow out of what was tapped (`design/Motion.kt`).
            @OptIn(ExperimentalSharedTransitionApi::class)
            SharedTransitionLayout {
            CompositionLocalProvider(LocalSharedTransition provides this) {
            Box(Modifier.fillMaxSize()) {
            NavHost(controller, startDestination = AppTab.Today.route, modifier = Modifier.padding(bottom = padding.calculateBottomPadding())) {
                tab(AppTab.Today) { TodayScreen() }
                tab(AppTab.Timetable) { TimetableScreen() }
                tab(AppTab.Lectures) { LecturesScreen() }
                tab(AppTab.Web) { WebTabScreen() }
                tab(AppTab.Settings) { SettingsScreen() }
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
                destination("record") { RecordScreen() }
                destination(
                    "page/{site}?target={target}&title={title}",
                    listOf(
                        navArgument("site") { type = NavType.StringType },
                        navArgument("target") { type = NavType.StringType; defaultValue = "" },
                        navArgument("title") { type = NavType.StringType; defaultValue = "" },
                    ),
                ) {
                    val args = it.arguments
                    val site = runCatching { SchoolSite.valueOf(args?.getString("site").orEmpty()) }
                        .getOrDefault(SchoolSite.SCIENCE_LMS)
                    WebPageScreen(
                        WebRouter.PushedPage(site, args?.getString("target")?.ifEmpty { null }, args?.getString("title").orEmpty()),
                    )
                }
            }
            // What was shared into the app waits in the corner, above the
            // navigation bar.
            InboxTray(
                Modifier.align(Alignment.BottomEnd)
                    .padding(end = 16.dp, bottom = padding.calculateBottomPadding() + 12.dp),
            )
            }
            }
            }
        }
        WelcomeSheet { navigator.openSettings(SchoolAuthProfile.SCIENCE) }
    }
}

private fun NavGraphBuilder.tab(tab: AppTab, content: @Composable () -> Unit) {
    navigation(startDestination = tab.start, route = tab.route) {
        destination(tab.start) { content() }
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
