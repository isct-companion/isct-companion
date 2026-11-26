package app.isctcompanion.design

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow

/** Brief messages for the app's one snackbar (`Tabs` in App.kt), so every
 * page reports the same way instead of each choosing a Toast or a strip of
 * its own. Callable from outside composition (downloads, exports); a message
 * sent while no host is on screen waits for the next one. */
object Notices {
    private val pending = Channel<String>(Channel.BUFFERED)
    internal val messages: Flow<String> = pending.receiveAsFlow()

    fun show(message: String) {
        if (message.isNotEmpty()) pending.trySend(message)
    }
}
