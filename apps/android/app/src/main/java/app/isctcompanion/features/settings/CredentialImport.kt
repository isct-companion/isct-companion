package app.isctcompanion.features.settings

import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ContentPaste
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.PhotoCamera
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.MatrixConfidence
import app.isctcompanion.core.OtpImportAccount
import app.isctcompanion.core.OtpImportException
import app.isctcompanion.core.importOtp
import app.isctcompanion.core.matrixConfidence
import app.isctcompanion.design.Theme
import com.google.android.gms.common.moduleinstall.ModuleInstall
import com.google.android.gms.common.moduleinstall.ModuleInstallRequest
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import com.google.mlkit.vision.common.InputImage
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

// Ways to bring a school second factor onto the phone. Parsing (authenticator
// links and transfers) and recognition (Matrix OCR) are Core's; the shell only
// turns the camera or a photo into text or image bytes.

@Composable
private fun SourceChip(label: String, icon: ImageVector, enabled: Boolean = true, onClick: () -> Unit) {
    AssistChip(
        onClick = onClick, enabled = enabled, label = { Text(label) },
        leadingIcon = { Icon(icon, null, modifier = Modifier.size(18.dp)) },
    )
}

private fun clipboardText(context: Context): String? =
    context.getSystemService(ClipboardManager::class.java)?.primaryClip
        ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()

// OTP

/** The OTP setup field with three sources: Google's code scanner, a
 * screenshot (the authenticator's export QR on this same phone), or the clipboard. */
@Composable
fun OtpImportField(saved: Boolean, setup: String, onSetup: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var choices by remember { mutableStateOf<List<OtpImportAccount>>(emptyList()) }
    var label by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    fun choose(account: OtpImportAccount) {
        onSetup(account.setup)
        choices = emptyList()
        label = name(account)
    }

    fun read(payload: String) {
        error = null
        val text = payload.trim()
        // A bare secret is pasted as is; Core validates it on save.
        if (!text.contains("://")) {
            onSetup(text)
            return
        }
        try {
            val accounts = importOtp(text)
            if (accounts.size == 1) choose(accounts.first()) else choices = accounts
        } catch (failure: OtpImportException) {
            error = when (failure) {
                is OtpImportException.MultiplePages -> "学校のアカウントだけをエクスポートして、もう一度読み込んでください。"
                is OtpImportException.Unsupported -> "このアカウントの認証コード形式には対応していません。"
                is OtpImportException.Invalid -> "対応している認証アプリの QR コードではありません。"
            }
        }
    }

    val photo = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            val payload = qrPayload(context, uri)
            if (payload == null) error = "QR コードが見つかりません。QR 全体が写った画像を選んでください。" else read(payload)
        }
    }

    OutlinedTextField(
        setup, { onSetup(it); label = null }, singleLine = true,
        label = { Text(if (saved) "設定済み（変更する場合のみ）" else "TOTP シークレットまたは otpauth:// リンク") },
        // Masked like a password, but not typed as one: Autofill must not offer
        // the school login here.
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii,
        ),
        modifier = Modifier.fillMaxWidth(),
    )
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SourceChip("QR を読み取る", Icons.Outlined.QrCodeScanner) {
            scope.launch {
                when (val result = scanQr(context)) {
                    null -> Unit
                    else -> result.fold(::read) { error = "スキャナーを開けませんでした。「写真から」を使ってください。" }
                }
            }
        }
        SourceChip("写真から", Icons.Outlined.Image) {
            photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
        }
        SourceChip("貼り付け", Icons.Outlined.ContentPaste) {
            clipboardText(context)?.let(::read) ?: run { error = "クリップボードが空です。" }
        }
    }
    label?.let { Text("「$it」を読み取りました。保存すると使われます。", style = MaterialTheme.typography.bodySmall, color = Theme.colors.ok) }
    error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
    Text(
        "Google Authenticator は「アカウントを移行 → エクスポート」で学校のアカウントだけを選び、表示された QR のスクリーンショットを「写真から」で読み込めます。",
        style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
    )
    if (choices.isNotEmpty()) {
        AlertDialog(
            onDismissRequest = { choices = emptyList() },
            title = { Text("どのアカウントを使いますか？") },
            text = {
                Column {
                    for (account in choices) {
                        Text(
                            name(account),
                            modifier = Modifier.fillMaxWidth().clickable { choose(account) }.padding(vertical = 12.dp),
                        )
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { choices = emptyList() }) { Text("キャンセル") } },
        )
    }
}

private fun name(account: OtpImportAccount) =
    listOf(account.issuer, account.label).filter { it.isNotEmpty() }.joinToString(" · ")

/** Google's code scanner: its own camera UI from Play services, no camera
 * permission here. `null` when the person backs out. */
private suspend fun scanQr(context: Context): Result<String>? = suspendCancellableCoroutine { done ->
    val options = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
    val scanner = GmsBarcodeScanning.getClient(context, options)
    // The scanner module downloads on first use; ask for it up front.
    ModuleInstall.getClient(context).installModules(ModuleInstallRequest.newBuilder().addApi(scanner).build())
    scanner.startScan()
        .addOnSuccessListener { code -> done.resume(code.rawValue?.let { Result.success(it) }) }
        .addOnCanceledListener { done.resume(null) }
        .addOnFailureListener { done.resume(Result.failure(it)) }
}

/** The first QR code ML Kit finds in a picked image. */
private suspend fun qrPayload(context: Context, uri: Uri): String? = suspendCancellableCoroutine { done ->
    val image = runCatching { InputImage.fromFilePath(context, uri) }.getOrNull()
    if (image == null) {
        done.resume(null)
        return@suspendCancellableCoroutine
    }
    val scanner = BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
    scanner.process(image)
        .addOnSuccessListener { codes -> done.resume(codes.firstNotNullOfOrNull { it.rawValue }) }
        .addOnFailureListener { done.resume(null) }
}

// Matrix

/** The Matrix card: photograph it (or pick a photo), Core reads the 70 cells,
 * and the person checks the grid against the card before saving. Pasting all
 * 70 letters (copied from the desktop app) fills the grid too. */
@Composable
fun MatrixImportSection(saved: Boolean, onMatrix: (String) -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val cells = remember { mutableStateListOf(*Array(70) { "" }) }
    val confidence = remember { mutableStateListOf<Double?>(*Array(70) { null }) }
    var editing by remember { mutableStateOf(false) }
    var reading by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val canScan = model.canScanMatrix

    fun publish() = onMatrix(cells.joinToString(""))

    fun scan(uri: Uri) {
        scope.launch {
            reading = true
            error = null
            note = null
            try {
                val jpeg = withContext(Dispatchers.IO) { jpeg(context, uri) }
                val result = model.scanMatrix(jpeg)
                if (result.size != 70) {
                    error = "表を見つけられませんでした。表全体が写るように撮り直してください。"
                } else {
                    result.forEachIndexed { index, cell ->
                        cells[index] = cell.text
                        confidence[index] = cell.confidence
                    }
                    publish()
                    editing = true
                    note = "色の付いたマスをカードと照らし合わせて確認してください。"
                }
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            }
            reading = false
        }
    }

    val shot = remember { File(context.cacheDir, "matrix/card.jpg") }
    val shotUri = remember {
        shot.parentFile?.mkdirs()
        FileProvider.getUriForFile(context, "${context.packageName}.files", shot)
    }
    val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
        if (taken) scan(shotUri)
    }
    val photo = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri?.let(::scan)
    }

    if (!editing) {
        Text(if (saved) "設定済み" else "未設定", color = if (saved) Theme.colors.ok else Theme.colors.muted)
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (canScan) {
            SourceChip("撮影", Icons.Outlined.PhotoCamera, enabled = !reading) { camera.launch(shotUri) }
            SourceChip("写真から", Icons.Outlined.Image, enabled = !reading) {
                photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
            }
        }
        SourceChip("貼り付け", Icons.Outlined.ContentPaste, enabled = !reading) {
            val letters = clipboardText(context).orEmpty().uppercase().filterNot { it.isWhitespace() }
            if (letters.length != 70 || !letters.all { it in 'A'..'Z' }) {
                error = "A1…J1、A2…J2 の順に 70 文字を貼り付けてください（空白と改行は無視されます）。"
            } else {
                error = null
                letters.forEachIndexed { index, c ->
                    cells[index] = c.toString()
                    confidence[index] = null
                }
                publish()
                editing = true
            }
        }
        if (!editing) SourceChip("入力", Icons.Outlined.Keyboard) { editing = true }
    }
    if (reading) {
        Text("読み取り中 ${model.ocrProgress}/70", style = MaterialTheme.typography.bodySmall)
        LinearProgressIndicator(progress = { model.ocrProgress / 70f }, modifier = Modifier.fillMaxWidth())
    }
    if (editing) {
        MatrixGrid(cells, confidence) { publish() }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Swatch(Theme.colors.okSoft, "確実")
            Swatch(Theme.colors.warnSoft, "要確認")
            Swatch(Theme.colors.dangerSoft, "読めない")
        }
    }
    note?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted) }
    error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
    Text(
        if (canScan) "カードを平らな場所に置き、表全体が写るように撮影してください。"
        else "この端末では読み取りを使えません。デスクトップ版でコピーした 70 文字を貼り付けるか、入力してください。",
        style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
    )
}

@Composable
private fun Swatch(color: Color, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Box(Modifier.size(12.dp).clip(RoundedCornerShape(3.dp)).background(color))
        Text(label, style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted)
    }
}

/** 7 rows × 10 columns (A–J), one letter each, tinted by Core's review level. */
@Composable
private fun MatrixGrid(cells: MutableList<String>, confidence: MutableList<Double?>, changed: () -> Unit) {
    val focus = remember { List(70) { FocusRequester() } }
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            Box(Modifier.width(16.dp))
            for (column in 0 until 10) {
                Text(
                    ('A' + column).toString(), modifier = Modifier.weight(1f), textAlign = TextAlign.Center,
                    style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted,
                )
            }
        }
        for (row in 0 until 7) {
            Row(horizontalArrangement = Arrangement.spacedBy(3.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("${row + 1}", modifier = Modifier.width(16.dp), style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted)
                for (column in 0 until 10) {
                    val index = row * 10 + column
                    val fill = confidence[index]?.let {
                        when (matrixConfidence(cells[index], it)) {
                            MatrixConfidence.HIGH -> Theme.colors.okSoft
                            MatrixConfidence.MEDIUM -> Theme.colors.warnSoft
                            MatrixConfidence.LOW -> Theme.colors.dangerSoft
                        }
                    } ?: Theme.colors.surface2
                    BasicTextField(
                        value = cells[index],
                        onValueChange = { value ->
                            val letter = value.uppercase().filter { it in 'A'..'Z' }.takeLast(1)
                            cells[index] = letter
                            // A corrected cell is the person's own entry now.
                            confidence[index] = null
                            changed()
                            if (letter.isNotEmpty() && index < 69) focus[index + 1].requestFocus()
                        },
                        singleLine = true,
                        textStyle = TextStyle(
                            fontFamily = FontFamily.Monospace, fontWeight = FontWeight.SemiBold, fontSize = 15.sp,
                            textAlign = TextAlign.Center, color = Theme.colors.ink,
                        ),
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false,
                            keyboardType = KeyboardType.Ascii,
                        ),
                        modifier = Modifier
                            .weight(1f)
                            .height(32.dp)
                            .clip(RoundedCornerShape(5.dp))
                            .background(fill)
                            .padding(top = 6.dp)
                            .focusRequester(focus[index]),
                    )
                }
            }
        }
    }
}

/** OCR reads PNG/JPEG/WebP; the decoder also applies the photo's rotation and
 * turns HEIF into JPEG. */
private fun jpeg(context: Context, uri: Uri): ByteArray {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, _, _ ->
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
    }
    return ByteArrayOutputStream().use { out ->
        bitmap.compress(Bitmap.CompressFormat.JPEG, 95, out)
        out.toByteArray()
    }
}
