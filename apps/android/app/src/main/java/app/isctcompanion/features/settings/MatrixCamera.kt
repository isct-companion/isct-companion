package app.isctcompanion.features.settings

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.os.SystemClock
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
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

/** The analysed frame and the outline Core fitted on that same frame. */
private class Shown(val picture: Bitmap, val outline: List<MatrixPoint>)

/**
 * Live Matrix scanning: CameraX hands each YUV frame to Core's camera session
 * (the same driver as the desktop camera and photo import) and draws the frame
 * it analysed with the outline fitted on it, so the box never lags the
 * picture. The grid fills in as Core confirms cells; `done` gets the final
 * cells, or null when the scan was cancelled or failed.
 */
@Composable
fun MatrixCameraDialog(session: MatrixCameraSession, done: (List<MatrixCell>?) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    var state by remember { mutableStateOf<MatrixCameraState?>(null) }
    var shown by remember { mutableStateOf<Shown?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val camera = remember { LiveCamera(session) }
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
            camera.start(context, lifecycle) { update ->
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

    Dialog(onDismissRequest = { finish(null) }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(
            Modifier.fillMaxSize().background(Theme.colors.bg).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Box(Modifier.weight(1f).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Color.Black), contentAlignment = Alignment.Center) {
                shown?.let { frame ->
                    val ratio = frame.picture.width.toFloat() / frame.picture.height
                    Box(Modifier.aspectRatio(ratio)) {
                        Image(frame.picture.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
                        Canvas(Modifier.fillMaxSize()) {
                            if (frame.outline.size == 4) {
                                val path = Path()
                                frame.outline.forEachIndexed { index, p ->
                                    val at = Offset(p.x.toFloat() * size.width, p.y.toFloat() * size.height)
                                    if (index == 0) path.moveTo(at.x, at.y) else path.lineTo(at.x, at.y)
                                }
                                path.close()
                                drawPath(path, Color(0xFFFF4545), style = Stroke(width = 3.dp.toPx()))
                            }
                        }
                    }
                }
            }
            LiveGrid(state?.cells.orEmpty())
            val ready = state?.readyCells?.toInt() ?: 0
            Text(
                error ?: if (ready > 0 && state?.phase != MatrixCameraPhase.COMPLETE) {
                    t("Recognizing Matrix… {{count}}/70 cells confirmed.", "count" to ready)
                } else {
                    t("Keep the Matrix framed and steady. Recognition finishes after all cells are confirmed, or within seven seconds.")
                },
                style = MaterialTheme.typography.bodySmall,
                color = if (error == null) Theme.colors.muted else Theme.colors.danger,
                textAlign = TextAlign.Center,
            )
            TextButton(onClick = { finish(null) }) { Text(t("Cancel")) }
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
                        Modifier.weight(1f).height(22.dp).clip(RoundedCornerShape(4.dp))
                            .background(if (stable) Theme.colors.okSoft else Theme.colors.surface2),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            cell?.text.orEmpty(),
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.SemiBold,
                            style = MaterialTheme.typography.labelMedium,
                            color = if (stable) Theme.colors.ink else Theme.colors.muted,
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
    private var main: java.util.concurrent.Executor? = null
    private var nextFrame = 0u
    private val started = SystemClock.elapsedRealtime()
    @Volatile var lastFrame = SystemClock.elapsedRealtime()
        private set
    val stopped: Boolean get() = closed.get()

    fun start(context: Context, owner: androidx.lifecycle.LifecycleOwner, update: (Update) -> Unit) {
        val main = ContextCompat.getMainExecutor(context).also { this.main = it }
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            if (closed.get()) return@addListener
            val cameras = runCatching { future.get() }.getOrElse {
                update(Update.Failed(t("The camera is unavailable. Close other apps using it and try again.")))
                return@addListener
            }
            provider = cameras
            val analysis = ImageAnalysis.Builder()
                .setResolutionSelector(
                    ResolutionSelector.Builder()
                        .setResolutionStrategy(
                            ResolutionStrategy(Size(1920, 1080), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER),
                        )
                        .build(),
                )
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
                cameras.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, analysis)
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
        // Show this frame only with an outline from this same frame.
        val outline = if (state.boundsFrameId == frameId) state.bounds else emptyList()
        return Update.Frame(Shown(image.toBitmap(), outline), state)
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
