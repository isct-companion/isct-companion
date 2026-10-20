package app.isctcompanion

import android.app.Application
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CalendarToday
import androidx.compose.material.icons.outlined.GridView
import androidx.compose.material.icons.outlined.Language
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.WarningAmber
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
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
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.TlsVerifier
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.IsctTheme
import app.isctcompanion.design.Localizer
import app.isctcompanion.design.t
import app.isctcompanion.design.Theme
import app.isctcompanion.features.assignments.RecordScreen
import app.isctcompanion.features.course.CourseDetailScreen
import app.isctcompanion.features.settings.SettingsScreen
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
        // Before the session opens: Core's HTTPS verifies through Android.
        TlsVerifier.install(this)
        scope.launch { model.start() }
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
        setContent {
            IsctTheme {
                CompositionLocalProvider(LocalModel provides model) { RootView() }
            }
        }
    }
}

val LocalModel = staticCompositionLocalOf<AppModel> { error("No AppModel") }
val LocalNavigator = staticCompositionLocalOf<Navigator> { error("No navigator") }

/** Pushes onto the current tab's stack: a course, or a school page. */
class Navigator(private val controller: NavHostController) {
    fun openCourse(id: String) = controller.navigate("course/${Uri.encode(id)}")

    fun openPage(page: WebRouter.PushedPage) = controller.navigate(
        "page/${page.site.name}?target=${Uri.encode(page.target.orEmpty())}&title=${Uri.encode(page.title)}",
    )

    fun openRecord() = controller.navigate("record")

    /** Switches to the 設定 tab, as tapping it would. */
    fun openSettings() = controller.navigate("tab/settings") {
        popUpTo(controller.graph.findStartDestination().id) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }

    fun back() = controller.popBackStack()
}

/** Assignments live on 今日, and 設定 is the last tab. */
private enum class AppTab(val route: String, val start: String, val label: String, val icon: ImageVector) {
    Today("tab/today", "today", "Today", Icons.Outlined.CalendarToday),
    Timetable("tab/timetable", "timetable", "Timetable", Icons.Outlined.GridView),
    Web("tab/web", "web", "Web", Icons.Outlined.Language),
    Settings("tab/settings", "settings", "Settings", Icons.Outlined.Settings),
}

@Composable
private fun RootView() {
    val model = LocalModel.current
    when (val status = model.status) {
        AppModel.Status.Opening -> Box(Modifier.fillMaxSize().background(Theme.colors.bg), Alignment.Center) {
            CircularProgressIndicator()
        }
        is AppModel.Status.Failed -> Column(
            Modifier.fillMaxSize().padding(32.dp),
            verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Icon(Icons.Outlined.WarningAmber, null, tint = Theme.colors.muted)
            Text(t("Could not start"), style = MaterialTheme.typography.titleLarge)
            Text(status.message, color = Theme.colors.muted, textAlign = TextAlign.Center)
        }
        AppModel.Status.Ready -> Tabs()
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
            t("Hid “{{title}}”", "title" to undo.title), actionLabel = t("Undo"), duration = SnackbarDuration.Short,
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
                        for (tab in AppTab.entries) {
                            NavigationBarItem(
                                selected = tab == current,
                                onClick = {
                                    if (tab == current) {
                                        // Tapping the current tab returns it to its root.
                                        controller.popBackStack(tab.start, inclusive = false)
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
            NavHost(controller, startDestination = AppTab.Today.route, modifier = Modifier.padding(bottom = padding.calculateBottomPadding())) {
                tab(AppTab.Today) { TodayScreen() }
                tab(AppTab.Timetable) { TimetableScreen() }
                tab(AppTab.Web) { WebTabScreen() }
                tab(AppTab.Settings) { SettingsScreen() }
                composable("course/{id}", listOf(navArgument("id") { type = NavType.StringType })) {
                    CourseDetailScreen(it.arguments?.getString("id").orEmpty())
                }
                composable("record") { RecordScreen() }
                composable(
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
        }
    }
}

private fun NavGraphBuilder.tab(tab: AppTab, content: @Composable () -> Unit) {
    navigation(startDestination = tab.start, route = tab.route) {
        composable(tab.start) { content() }
    }
}
