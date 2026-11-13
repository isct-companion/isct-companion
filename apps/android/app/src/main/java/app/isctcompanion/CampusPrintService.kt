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
import app.isctcompanion.core.CampusPrintColor
import app.isctcompanion.core.CampusPrintJob
import app.isctcompanion.core.CampusPrintMedia
import app.isctcompanion.core.CampusPrintMessage
import app.isctcompanion.core.CampusPrintSides
import app.isctcompanion.core.campusPrinterCapabilities
import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** The campus printer in the system's print dialogs. Jobs run one after
 * another: read the PDF the system rendered, hand it to Core, which bills
 * the verified student ID, sends it and words how it ended. A job cancelled
 * before its send starts is just cancelled; once sending, it finishes. */
class CampusPrintService : PrintService() {
    private val scope = MainScope()
    private val turn = Mutex()
    private val jobs = mutableMapOf<PrintJobId, Job>()
    private val sending = mutableSetOf<PrintJobId>()
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
        jobs[id] = scope.launch {
            try {
                turn.withLock {
                    if (!printJob.isStarted && !printJob.start()) return@withLock
                    val data = printJob.document.data ?: throw IllegalStateException("Print document unavailable")
                    val document = withContext(Dispatchers.IO) { read(data) }
                    sending.add(id)
                    val report = model.campusPrint(document, job(printJob))
                    when {
                        report == null -> printJob.fail(t("campusPrint.notice.queueFailed"))
                        report.printed -> printJob.complete()
                        else -> printJob.fail(text(report.message))
                    }
                }
            } catch (error: CancellationException) {
                if (id in cancelled) printJob.cancel() else throw error
            } catch (error: Exception) {
                Log.w("isct", "campus print", error)
                printJob.fail(t("campusPrint.notice.queueFailed"))
            } finally {
                jobs.remove(id)
                sending.remove(id)
                cancelled.remove(id)
            }
        }
    }

    override fun onRequestCancelPrintJob(printJob: PrintJob) {
        val id = printJob.id ?: return
        if (id in sending) return
        val job = jobs[id]
        if (job == null) {
            printJob.cancel()
            return
        }
        cancelled.add(id)
        job.cancel()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    /** The document, up to one byte past Core's limit: Core refuses a
     * larger one, and the rest is never loaded. */
    private fun read(data: ParcelFileDescriptor): ByteArray =
        ParcelFileDescriptor.AutoCloseInputStream(data).use { input ->
            input.readNBytes(CAPABILITIES.maxDocumentBytes.toInt() + 1)
        }

    /** What the dialog chose; Core gives a setting it left out the printer's default. */
    private fun job(printJob: PrintJob): CampusPrintJob {
        val info = printJob.info
        return CampusPrintJob(
            title = info.label,
            copies = info.copies.toUInt(),
            sides = when (info.attributes.duplexMode) {
                PrintAttributes.DUPLEX_MODE_NONE -> CampusPrintSides.ONE_SIDED
                PrintAttributes.DUPLEX_MODE_LONG_EDGE -> CampusPrintSides.LONG_EDGE
                PrintAttributes.DUPLEX_MODE_SHORT_EDGE -> CampusPrintSides.SHORT_EDGE
                else -> null
            },
            color = when (info.attributes.colorMode) {
                PrintAttributes.COLOR_MODE_COLOR -> CampusPrintColor.COLOR
                PrintAttributes.COLOR_MODE_MONOCHROME -> CampusPrintColor.MONO
                else -> null
            },
        )
    }

    /** Core's words for how a job ended, in the UI language. */
    private fun text(message: CampusPrintMessage): String =
        t(message.key, *message.args.map { (name, value) -> Pair<String, Any>(name, value) }.toTypedArray())

    /** The one printer, with what Core says it offers. It is always listed
     * as idle: whether it can be reached is known only by sending, and the
     * job's failure says so. */
    private fun printer(): PrinterInfo {
        val id = generatePrinterId(PRINTER)
        val builder = PrinterCapabilitiesInfo.Builder(id)
        CAPABILITIES.media.forEachIndexed { index, media ->
            builder.addMediaSize(
                when (media) {
                    CampusPrintMedia.A4 -> PrintAttributes.MediaSize.ISO_A4
                    CampusPrintMedia.LETTER -> PrintAttributes.MediaSize.NA_LETTER
                },
                index == 0,
            )
        }
        val margin = CAPABILITIES.marginMils
        val capabilities = builder
            .addResolution(PrintAttributes.Resolution("600", "600 dpi", 600, 600), true)
            .setColorModes(
                PrintAttributes.COLOR_MODE_MONOCHROME or PrintAttributes.COLOR_MODE_COLOR,
                when (CAPABILITIES.defaultColor) {
                    CampusPrintColor.MONO -> PrintAttributes.COLOR_MODE_MONOCHROME
                    CampusPrintColor.COLOR -> PrintAttributes.COLOR_MODE_COLOR
                },
            )
            .setDuplexModes(
                PrintAttributes.DUPLEX_MODE_NONE or PrintAttributes.DUPLEX_MODE_LONG_EDGE or
                    PrintAttributes.DUPLEX_MODE_SHORT_EDGE,
                when (CAPABILITIES.defaultSides) {
                    CampusPrintSides.ONE_SIDED -> PrintAttributes.DUPLEX_MODE_NONE
                    CampusPrintSides.LONG_EDGE -> PrintAttributes.DUPLEX_MODE_LONG_EDGE
                    CampusPrintSides.SHORT_EDGE -> PrintAttributes.DUPLEX_MODE_SHORT_EDGE
                },
            )
            .setMinMargins(PrintAttributes.Margins(margin, margin, margin, margin))
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
        val CAPABILITIES = campusPrinterCapabilities()
    }
}
