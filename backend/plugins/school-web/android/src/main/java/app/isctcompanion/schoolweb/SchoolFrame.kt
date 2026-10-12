package app.isctcompanion.schoolweb

import kotlin.math.roundToInt

/** Physical pixels relative to the overlay, clipped to the device safe area. */
data class SchoolFrame(val x: Int, val y: Int, val width: Int, val height: Int) {
    companion object {
        fun calculate(x: Double, y: Double, width: Double, height: Double, scale: Double,
            shellX: Int, shellY: Int, safeLeft: Int, safeTop: Int, safeRight: Int, safeBottom: Int): SchoolFrame {
            val rawX = shellX + (x * scale).roundToInt()
            val rawY = shellY + (y * scale).roundToInt()
            val left = rawX.coerceAtLeast(safeLeft)
            val top = rawY.coerceAtLeast(safeTop)
            val right = (rawX + (width * scale).roundToInt()).coerceAtMost(safeRight)
            val bottom = (rawY + (height * scale).roundToInt()).coerceAtMost(safeBottom)
            return SchoolFrame(left, top, (right - left).coerceAtLeast(0), (bottom - top).coerceAtLeast(0))
        }
    }
}
