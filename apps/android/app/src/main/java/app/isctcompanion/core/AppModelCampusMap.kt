package app.isctcompanion.core

import app.isctcompanion.design.t
import android.util.Log
import kotlinx.coroutines.CancellationException
import app.isctcompanion.core.AppModel.Companion.describe

// Campus map

/** Whether Core's map knows a room string, or with null whether a map is
 * stored at all; false (also before Core has the map's file) means no
 * map button. A failure also hides the button, but is logged. */
internal suspend fun AppModel.campusMapHas(room: String?): Boolean {
    val session = session ?: return false
    if (room?.isEmpty() == true) return false
    return try {
        (session.campusMapHas(room).value() as? OperationResult.Flag)?.value == true
    } catch (error: CancellationException) {
        throw error
    } catch (error: SdkException.Unavailable) {
        false
    } catch (error: Exception) {
        Log.w("isct", "campus map has", error)
        false
    }
}

/** Opens Core's map page on a room string, or on none to browse. */
internal suspend fun AppModel.campusMapOpen(room: String?): CampusMapOpening {
    val session = session ?: return CampusMapOpening.Failed(t("error.internal"))
    return try {
        val scene = (session.campusMapOpen(room).value() as? OperationResult.CampusMapScene)?.scene
        if (scene != null) CampusMapOpening.Ready(scene) else CampusMapOpening.Failed(t("error.internal"))
    } catch (error: CancellationException) {
        throw error
    } catch (error: SdkException.Unavailable) {
        CampusMapOpening.Missing
    } catch (error: Exception) {
        CampusMapOpening.Failed(describe(error))
    }
}

/** One intent on an open map page; null when nothing drawn changed or
 * the page is gone. A failure is logged: the page keeps its last scene. */
/** Sends one floor photo to the maintainers' inbox; `null` once stored,
 * else what to show. */
internal suspend fun AppModel.campusMapUploadPhoto(buildingId: String, jpeg: ByteArray): String? {
    val session = session ?: return t("error.internal")
    return try {
        session.campusMapUploadPhoto(buildingId, jpeg).value()
        null
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        Log.w("isct", "campus map photo", error)
        describe(error)
    }
}

internal suspend fun AppModel.campusMapSend(page: ULong, intent: CampusMapIntent): CampusMapScene? {
    val session = session ?: return null
    return try {
        (session.campusMapSend(page, intent).value() as? OperationResult.CampusMapScene)?.scene
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        Log.w("isct", "campus map send", error)
        null
    }
}

/** What a map search's text names; Core echoes the text so the page can
 * drop answers to text since changed. A failure carries the text to show. */
internal suspend fun AppModel.campusMapSearch(query: String): Result<CampusMapSearch> {
    val session = session ?: return Result.failure(IllegalStateException(t("error.internal")))
    return try {
        val search = (session.campusMapSearch(query).value() as? OperationResult.CampusMapSearch)?.search
        if (search != null) Result.success(search) else Result.failure(IllegalStateException(t("error.internal")))
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        Result.failure(IllegalStateException(describe(error)))
    }
}

/** Drops Core's map page, and its open if it is still loading. */
internal fun AppModel.campusMapClose() {
    session?.campusMapClose()
}
