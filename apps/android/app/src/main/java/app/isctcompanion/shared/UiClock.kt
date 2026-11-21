package app.isctcompanion.shared

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import java.time.Instant
import kotlinx.coroutines.delay

/** A clock for countdowns, ticking every `seconds`. */
@Composable
fun rememberNow(seconds: Long): Instant {
    val now by produceState(Instant.now()) {
        while (true) {
            delay(seconds * 1_000)
            value = Instant.now()
        }
    }
    return now
}
