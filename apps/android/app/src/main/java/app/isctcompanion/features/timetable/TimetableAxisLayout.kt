package app.isctcompanion.features.timetable

import app.isctcompanion.core.TimetableAxis
import app.isctcompanion.design.Format
import java.time.Instant

/* Core's timetable axis (`timetable_axis`): every period one row, lunch half
 * a row, short breaks folded into the rules. The shell only interpolates
 * between Core's stops, so desktop and both phones place times identically. */

fun TimetableAxis.offset(minutes: Int): Double {
    val first = stops.firstOrNull() ?: return 0.0
    val last = stops.last()
    if (minutes <= first.minutes.toInt()) return first.offset
    if (minutes >= last.minutes.toInt()) return last.offset
    val index = stops.indexOfFirst { minutes <= it.minutes.toInt() }
    if (index <= 0) return last.offset
    val from = stops[index - 1]
    val to = stops[index]
    val span = maxOf(to.minutes.toDouble() - from.minutes.toDouble(), 1.0)
    return from.offset + (to.offset - from.offset) * (minutes - from.minutes.toDouble()) / span
}

fun TimetableAxis.offset(clock: String): Double = offset(clockMinutes(clock))

/** Where now sits, while it falls inside the drawn day. */
fun TimetableAxis.now(instant: Instant): Double? {
    val at = Format.tokyo(instant)
    val minutes = at.hour * 60 + at.minute
    val first = stops.firstOrNull() ?: return null
    if (minutes < first.minutes.toInt() || minutes > stops.last().minutes.toInt()) return null
    return offset(minutes)
}

fun clockMinutes(clock: String): Int {
    val parts = clock.split(":").mapNotNull { it.toIntOrNull() }
    return if (parts.size == 2) parts[0] * 60 + parts[1] else 0
}
