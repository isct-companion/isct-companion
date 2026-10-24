package app.isctcompanion.features.settings

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.SystemClock
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraControl
import androidx.camera.core.CameraSelector
import androidx.camera.core.FocusMeteringAction
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.MeteringPoint
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.MatrixCameraFrame
import app.isctcompanion.core.MatrixCameraPhase
import app.isctcompanion.core.MatrixCameraSession
import app.isctcompanion.core.MatrixCameraState
import app.isctcompanion.core.MatrixCell
import app.isctcompanion.core.MatrixPoint
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/** The outline Core fitted on an analysed frame, and that frame's size. */
private class Shown(val width: Int, val height: Int, val outline: List<MatrixPoint>)

/**
 * Live Matrix scanning: CameraX hands each YUV frame to Core's camera session
 * (the same driver as the desktop camera and photo import). As in the system
 * camera the live preview fills the screen and a tap focuses there; a
 * card-shaped guide shows where to hold the card, and the outline Core last
 * fitted is drawn over it. The grid fills in as Core confirms cells; `done`
 * gets the final cells, or null when the scan was cancelled or failed.
 */
@Composable
fun MatrixCameraDialog(session: MatrixCameraSession, done: (List<MatrixCell>?) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    var state by remember { mutableStateOf<MatrixCameraState?>(null) }
    var shown by remember { mutableStateOf<Shown?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var focusAt by remember { mutableStateOf<Offset?>(null) }
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val camera = remember { LiveCamera(session) }
    val preview = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        granted = allowed
        if (!allowed) error = t("Camera access was denied. Allow camera access in system settings, then try again.")
    }
    fun finish(cells: List<MatrixCell>?) {
        camera.stop()
        done(cells)
    }
    fun fail(message: String) {
        camera.stop()
        error = message
    }

    LaunchedEffect(Unit) { if (!granted) permission.launch(Manifest.permission.CAMERA) }
    DisposableEffect(granted) {
        if (granted) {
            camera.start(context, lifecycle, preview) { update ->
                when (update) {
                    is LiveCamera.Update.Frame -> {
                        shown = update.shown
                        state = update.state
                        if (update.state.phase == MatrixCameraPhase.COMPLETE) finish(update.state.cells)
                    }
                    is LiveCamera.Update.State -> {
                        state = update.state
                        if (update.state.phase == MatrixCameraPhase.COMPLETE) finish(update.state.cells)
                    }
                    is LiveCamera.Update.Failed -> fail(update.message)
                }
            }
        }
        // As on desktop: leaving the app stops acquisition for good.
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP && state?.phase != MatrixCameraPhase.COMPLETE) {
                fail(t("Camera scan stopped while the app was in the background. Open it again to continue."))
            }
        }
        lifecycle.lifecycle.addObserver(observer)
        onDispose {
            lifecycle.lifecycle.removeObserver(observer)
            camera.stop()
        }
    }
    // Core's deadline advances even when no frame arrives.
    LaunchedEffect(granted) {
        while (granted && isActive && !camera.stopped) {
            delay(100)
            if (SystemClock.elapsedRealtime() - camera.lastFrame >= 100) {
                withContext(Dispatchers.IO) { camera.poll() }?.let { next ->
                    state = next
                    if (next.phase == MatrixCameraPhase.COMPLETE) finish(next.cells)
                }
            }
        }
    }
    // The focus square fades as the system camera's does.
    LaunchedEffect(focusAt) {
        if (focusAt != null) {
            delay(1200)
            focusAt = null
        }
    }

    Dialog(
        onDismissRequest = { finish(null) },
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        Box(Modifier.fillMaxSize().background(Color.Black)) {
            AndroidView({ preview }, Modifier.fillMaxSize().pointerInput(Unit) {
                detectTapGestures { at ->
                    camera.focus(preview.meteringPointFactory.createPoint(at.x, at.y))
                    focusAt = at
                }
            })
            Canvas(Modifier.fillMaxSize()) {
                val frame = shown
                if (frame != null && frame.outline.size == 4) {
                    // The preview fills the screen, so the frame overflows it
                    // on one axis.
                    val scale = maxOf(size.width / frame.width, size.height / frame.height)
                    val left = (size.width - frame.width * scale) / 2
                    val top = (size.height - frame.height * scale) / 2
                    val path = Path()
                    frame.outline.forEachIndexed { index, p ->
                        val x = left + p.x.toFloat() * frame.width * scale
                        val y = top + p.y.toFloat() * frame.height * scale
                        if (index == 0) path.moveTo(x, y) else path.lineTo(x, y)
                    }
                    path.close()
                    drawPath(path, Color(0xFFFF4545), style = Stroke(width = 3.dp.toPx()))
                }
                focusAt?.let { at ->
                    val half = 40.dp.toPx()
                    drawRect(
                        Color(0xFFFFD60A),
                        topLeft = Offset(at.x - half, at.y - half),
                        size = androidx.compose.ui.geometry.Size(half * 2, half * 2),
                        style = Stroke(width = 1.5.dp.toPx()),
                    )
                }
            }
            Column(Modifier.fillMaxSize().systemBarsPadding()) {
                IconButton(
                    onClick = { finish(null) },
                    modifier = Modifier.padding(12.dp).size(44.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.45f)),
                ) {
                    Icon(Icons.Outlined.Close, t("Cancel"), tint = Color.White)
                }
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    // Where to hold the card: an ID-1 card's outline.
                    Box(
                        Modifier.padding(horizontal = 24.dp).widthIn(max = 480.dp).fillMaxWidth()
                            .aspectRatio(85.6f / 53.98f)
                            .border(2.dp, Color.White.copy(alpha = 0.7f), RoundedCornerShape(14.dp)),
                    )
                }
                Column(
                    Modifier.padding(horizontal = 12.dp, vertical = 8.dp).fillMaxWidth()
                        .clip(RoundedCornerShape(22.dp)).background(Color.Black.copy(alpha = 0.55f)).padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    LiveGrid(state?.cells.orEmpty())
                    val ready = state?.readyCells?.toInt() ?: 0
                    Text(
                        error ?: if (ready > 0 && state?.phase != MatrixCameraPhase.COMPLETE) {
                            t("Recognizing Matrix… {{count}}/70 cells confirmed.", "count" to ready)
                        } else {
                            t("Keep the Matrix framed and steady. Recognition finishes after all cells are confirmed, or within seven seconds.")
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = if (error == null) Color.White.copy(alpha = 0.85f) else Theme.colors.danger,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

/** The 7×10 letters as Core confirms them; unconfirmed reads stay faint. */
@Composable
private fun LiveGrid(cells: List<MatrixCell>) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        for (row in 0 until 7) {
            Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                for (column in 0 until 10) {
                    val cell = cells.getOrNull(row * 10 + column)
                    val stable = cell?.stable == true
                    Box(
                        Modifier.weight(1f).height(20.dp).clip(RoundedCornerShape(4.dp))
                            .background(if (stable) Theme.colors.okSoft else Color.White.copy(alpha = 0.12f)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            cell?.text.orEmpty(),
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.SemiBold,
                            style = MaterialTheme.typography.labelMedium,
                            color = if (stable) Color.Black else Color.White.copy(alpha = 0.6f),
                        )
                    }
                }
            }
        }
    }
}

/**
 * Owns the CameraX binding and one in-flight frame: frames arriving while
 * Core analyses the previous one are dropped, as on desktop.
 */
private class LiveCamera(private val scan: MatrixCameraSession) {
    sealed interface Update {
        class Frame(val shown: Shown, val state: MatrixCameraState) : Update
        class State(val state: MatrixCameraState) : Update
        class Failed(val message: String) : Update
    }

    private val executor = Executors.newSingleThreadExecutor()
    private val closed = AtomicBoolean(false)
    private var provider: ProcessCameraProvider? = null
    private var control: CameraControl? = null
    private var main: java.util.concurrent.Executor? = null
    private var nextFrame = 0u
    private val started = SystemClock.elapsedRealtime()
    @Volatile var lastFrame = SystemClock.elapsedRealtime()
        private set
    val stopped: Boolean get() = closed.get()

    fun start(context: Context, owner: androidx.lifecycle.LifecycleOwner, view: PreviewView, update: (Update) -> Unit) {
        val main = ContextCompat.getMainExecutor(context).also { this.main = it }
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            if (closed.get()) return@addListener
            val cameras = runCatching { future.get() }.getOrElse {
                update(Update.Failed(t("The camera is unavailable. Close other apps using it and try again.")))
                return@addListener
            }
            provider = cameras
            // One aspect for both, so the outline Core fits on an analysed
            // frame lands on the same picture the preview crops.
            val resolution = ResolutionSelector.Builder()
                .setAspectRatioStrategy(AspectRatioStrategy.RATIO_16_9_FALLBACK_AUTO_STRATEGY)
                .setResolutionStrategy(
                    ResolutionStrategy(Size(1920, 1080), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER),
                )
                .build()
            val preview = Preview.Builder().setResolutionSelector(resolution).build()
                .also { it.surfaceProvider = view.surfaceProvider }
            val analysis = ImageAnalysis.Builder()
                .setResolutionSelector(resolution)
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
                // Frames arrive upright for the display, so Core's normalized
                // outline maps straight onto the drawn picture.
                .setOutputImageRotationEnabled(true)
                .build()
            analysis.setAnalyzer(executor) { image ->
                image.use { analyse(it)?.let { result -> main.execute { if (!closed.get()) update(result) } } }
            }
            runCatching {
                cameras.unbindAll()
                control = cameras.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis).cameraControl
            }.onFailure {
                update(Update.Failed(t("The camera is unavailable. Close other apps using it and try again.")))
            }
        }, main)
    }

    private fun analyse(image: ImageProxy): Update? {
        if (closed.get()) return null
        lastFrame = SystemClock.elapsedRealtime()
        val frameId = nextFrame++
        val (y, u, v) = image.planes
        fun bytes(plane: ImageProxy.PlaneProxy): ByteArray {
            val buffer = plane.buffer.duplicate().apply { rewind() }
            return ByteArray(buffer.remaining()).also(buffer::get)
        }
        val frame = MatrixCameraFrame(
            frameId = frameId,
            width = image.width.toUInt(),
            height = image.height.toUInt(),
            y = bytes(y),
            yRowStride = y.rowStride.toUInt(),
            u = bytes(u),
            v = bytes(v),
            uvRowStride = u.rowStride.toUInt(),
            uvPixelStride = u.pixelStride.toUInt(),
            // Camera2 YUV is JFIF: BT.601, full range.
            bt709 = false,
            fullRange = true,
            capturedMs = (SystemClock.elapsedRealtime() - started).toDouble(),
        )
        val state = runCatching { scan.submit(frame) }.getOrElse {
            stop()
            return Update.Failed(AppModel.describe(it))
        }
        if (state.phase == MatrixCameraPhase.COMPLETE) stop()
        // Only an outline fitted on this frame; a stale one would point
        // where the card was.
        val outline = if (state.boundsFrameId == frameId) state.bounds else emptyList()
        return Update.Frame(Shown(image.width, image.height, outline), state)
    }

    /** Focuses and meters at a tapped point, as the system camera does. */
    fun focus(point: MeteringPoint) {
        if (closed.get()) return
        control?.startFocusAndMetering(FocusMeteringAction.Builder(point).build())
    }

    fun poll(): MatrixCameraState? =
        if (closed.get()) null else runCatching { scan.poll() }.getOrNull()

    fun stop() {
        if (closed.getAndSet(true)) return
        // CameraX binds on the main thread; this may run on the analyzer's.
        main?.execute { provider?.unbindAll() }
        executor.execute {
            scan.stop()
            scan.close()
        }
        executor.shutdown()
    }
}
