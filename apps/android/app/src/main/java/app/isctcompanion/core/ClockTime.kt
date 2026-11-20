package app.isctcompanion.core

fun clockMinutes(clock: String): Int {
    val parts = clock.split(":").mapNotNull { it.toIntOrNull() }
    return if (parts.size == 2) parts[0] * 60 + parts[1] else 0
}
