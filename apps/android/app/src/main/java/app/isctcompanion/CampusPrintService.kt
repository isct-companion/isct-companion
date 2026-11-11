package app.isctcompanion

import android.os.ParcelFileDescriptor
import android.print.PrintAttributes
import android.print.PrintJobId
import android.print.PrinterCapabilitiesInfo
import android.print.PrinterId
import android.print.PrinterInfo
import android.printservice.PrintJob
import android.printservice.PrintService
import android.printservice.PrinterDiscoverySession
import android.util.Log
import app.isctcompanion.core.CampusPrintAdmission
import app.isctcompanion.core.CampusPrintJob
import app.isctcompanion.core.CampusPrintResult
import app.isctcompanion.core.CampusPrintSides
import app.isctcompanion.core.SdkException
import app.isctcompanion.core.campusPrintDocumentLimit
import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Android's system print service delegates task ownership and retry decisions to Core. */
class CampusPrintService : PrintService() {
    private val scope = MainScope()
    private val jobs = mutableMapOf<PrintJobId, Job>()
    private val readers = mutableMapOf<PrintJobId, ParcelFileDescriptor>()
    private val cancelled = mutableSetOf<PrintJobId>()
    private val model get() = (application as IsctApplication).model

    override fun onCreatePrinterDiscoverySession(): PrinterDiscoverySession = Discovery()

    override fun onConnected() {
        super.onConnected()
        activePrintJobs.forEach(::onPrintJobQueued)
    }

    override fun onPrintJobQueued(printJob: PrintJob) {
        val id = printJob.id ?: return
        if (jobs.containsKey(id)) return
        val info = printJob.info
        val job = CampusPrintJob(
            id = id.toString(), title = info.label,
            copies = info.copies.coerceAtLeast(1).toUInt(),
            sides = when (info.attributes.duplexMode) {
                PrintAttributes.DUPLEX_MODE_NONE -> CampusPrintSides.ONE_SIDED
                PrintAttributes.DUPLEX_MODE_LONG_EDGE -> CampusPrintSides.LONG_EDGE
                PrintAttributes.DUPLEX_MODE_SHORT_EDGE -> CampusPrintSides.SHORT_EDGE
                else -> null
            },
            color = when (info.attributes.colorMode) {
                PrintAttributes.COLOR_MODE_COLOR -> true
                PrintAttributes.COLOR_MODE_MONOCHROME -> false
                else -> null
            },
        )
        jobs[id] = scope.launch {
            var lease: ULong? = null
            try {
                while (true) {
                    val admission = withContext(NonCancellable) {
                        model.campusPrintReserve(job).also { if (it is CampusPrintAdmission.Ready) lease = it.lease }
                    }
                    currentCoroutineContext().ensureActive()
                    when (admission) {
                        is CampusPrintAdmission.Busy -> {
                            if (printJob.isQueued && !printJob.start()) return@launch
                            printJob.block(t("campusPrint.notice.queueFailed"))
                            model.campusPrintWaitAvailable()
                        }
                        is CampusPrintAdmission.Unknown -> {
                            if (printJob.isQueued && !printJob.start()) return@launch
                            printJob.fail(reason(CampusPrintResult.Uncertain(admission.printed)))
                            return@launch
                        }
                        is CampusPrintAdmission.Ready -> break
                    }
                }
                if (!printJob.isStarted && !printJob.start()) return@launch
                val data = printJob.document.data ?: throw IllegalStateException("Print document unavailable")
                readers[id] = data
                val document = withContext(Dispatchers.IO) { read(data, LIMIT) }
                readers.remove(id)
                val result = if (document == null) CampusPrintResult.TooLarge else model.campusPrint(document, job, checkNotNull(lease))
                if (result is CampusPrintResult.Printed) {
                    if (printJob.complete()) model.campusPrintForget(job.id)
                } else {
                    printJob.fail(reason(result))
                }
            } catch (error: CancellationException) {
                if (id in cancelled) printJob.cancel()
                else throw error
            } catch (error: SdkException.Cancelled) {
                printJob.cancel()
            } catch (error: Exception) {
                Log.w("isct", "campus print", error)
                printJob.fail(t("campusPrint.notice.queueFailed"))
            } finally {
                runCatching { readers.remove(id)?.close() }
                try {
                    lease?.let { held -> withContext(NonCancellable) { runCatching { model.campusPrintRelease(job.id, held) } } }
                } finally {
                    jobs.remove(id)
                    cancelled.remove(id)
                }
            }
        }
    }

    override fun onRequestCancelPrintJob(printJob: PrintJob) {
        val id = printJob.id ?: return
        scope.launch {
            val beforeSend = runCatching { model.campusPrintCancel(id.toString()) }.getOrDefault(false)
            if (beforeSend) {
                if (jobs.containsKey(id)) cancelled.add(id)
                runCatching { readers.remove(id)?.close() }
                jobs[id]?.cancel()
                printJob.cancel()
            }
        }
    }

    override fun onDestroy() {
        readers.values.forEach { runCatching { it.close() } }
        readers.clear()
        scope.cancel()
        super.onDestroy()
    }

    private fun read(data: ParcelFileDescriptor, limit: Int): ByteArray? =
        ParcelFileDescriptor.AutoCloseInputStream(data).use { input ->
            input.readNBytes(limit + 1).takeIf { it.size <= limit }
        }

    /** Why a job did not print, in the words the desktop's print window
     * uses; `null` is a job that never reached Core. */
    private fun reason(result: CampusPrintResult?): String = when (result) {
        null, is CampusPrintResult.Printed -> t("campusPrint.notice.queueFailed")
        is CampusPrintResult.NotConnected -> t("tools.campusPrinter.saveTheTokyoTechAccount")
        is CampusPrintResult.NotStudentId -> t("tools.campusPrinter.accountIsNotAStudentId")
        is CampusPrintResult.NeedsAttention -> t("tools.campusPrinter.accountNeedsAttention")
        is CampusPrintResult.Unverified -> t("tools.campusPrinter.accountNotVerified")
        is CampusPrintResult.NotPrintable -> t("campusPrint.window.notAPdf")
        is CampusPrintResult.TooLarge -> t("campusPrint.notice.tooLarge", "megabytes" to LIMIT / (1024 * 1024))
        is CampusPrintResult.Unreachable ->
            if (result.printed == 0u) t("tools.campusPrinter.cannotReachThePrinter")
            else t("campusPrint.window.unreachableAfter", "printed" to result.printed)
        is CampusPrintResult.NotPrinted ->
            if (result.printed == 0u) t("campusPrint.window.notPrinted")
            else t("campusPrint.window.notPrintedAfter", "printed" to result.printed)
        is CampusPrintResult.Uncertain ->
            if (result.printed == 0u) t("campusPrint.window.uncertain")
            else t("campusPrint.window.uncertainAfter", "printed" to result.printed)
    }

    /** The one printer, with what the Windows system printer offers: A4 and
     * Letter, black and white by default, two-sided on the long edge by
     * default. It is always listed as idle: whether it can be reached is
     * known only by sending, and the job's failure says so. */
    private fun printer(): PrinterInfo {
        val id = generatePrinterId(PRINTER)
        val capabilities = PrinterCapabilitiesInfo.Builder(id)
            .addMediaSize(PrintAttributes.MediaSize.ISO_A4, true)
            .addMediaSize(PrintAttributes.MediaSize.NA_LETTER, false)
            .addResolution(PrintAttributes.Resolution("600", "600 dpi", 600, 600), true)
            .setColorModes(
                PrintAttributes.COLOR_MODE_MONOCHROME or PrintAttributes.COLOR_MODE_COLOR,
                PrintAttributes.COLOR_MODE_MONOCHROME,
            )
            .setDuplexModes(
                PrintAttributes.DUPLEX_MODE_NONE or PrintAttributes.DUPLEX_MODE_LONG_EDGE or
                    PrintAttributes.DUPLEX_MODE_SHORT_EDGE,
                PrintAttributes.DUPLEX_MODE_LONG_EDGE,
            )
            .setMinMargins(PrintAttributes.Margins(MARGIN, MARGIN, MARGIN, MARGIN))
            .build()
        return PrinterInfo.Builder(id, t("tools.campusPrinter.campusPrinter"), PrinterInfo.STATUS_IDLE)
            .setDescription(t("tools.campusPrinter.printsToTheKyoceraPrinter"))
            .setCapabilities(capabilities)
            .build()
    }

    /** The system asks for printers when a print dialog opens, and for one
     * printer's capabilities when it is selected; both get the same answer. */
    private inner class Discovery : PrinterDiscoverySession() {
        override fun onStartPrinterDiscovery(priorityList: List<PrinterId>) = addPrinters(listOf(printer()))
        override fun onStopPrinterDiscovery() {}
        override fun onValidatePrinters(printerIds: List<PrinterId>) = addPrinters(listOf(printer()))
        override fun onStartPrinterStateTracking(printerId: PrinterId) = addPrinters(listOf(printer()))
        override fun onStopPrinterStateTracking(printerId: PrinterId) {}
        override fun onDestroy() {}
    }

    private companion object {
        /** The printer's local id; the system remembers the last printer by it. */
        const val PRINTER = "kyocera"
        /** The most bytes Core takes for one document (64 MiB, an `Int`). */
        val LIMIT = campusPrintDocumentLimit().toInt()
        /** 1/6 inch in thousandths of an inch, the margin the Windows
         * system printer reports (4.23 mm). */
        const val MARGIN = 167
    }
}
