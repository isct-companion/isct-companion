package app.isctcompanion.design

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONObject
import java.util.Locale

/**
 * The UI language, read from `settings.locale` like the desktop's.
 *
 * Strings are keyed by namespaced message keys (`settings.reset.caches.title`)
 * and read from the catalogs the desktop uses
 * (`resources/locales/{en,ja,zh}.json`, i18next format with `{{name}}`
 * placeholders, packaged as assets). `en.json` holds every key with its
 * English copy, and a language that lacks a key reads the English one. They
 * are read directly rather than through `strings.xml` because the language is
 * an in-app setting, not the system's, and the placeholders must stay the
 * desktop's named ones so every shell shares one translation.
 *
 * `language` is snapshot state, so a composable that calls [t] recomposes
 * when it changes; nothing else has to pass it down.
 */
object Localizer {
    /** `en`, `ja` or `zh`. */
    var language by mutableStateOf("en")
        private set
    private var catalog: Map<String, String> = emptyMap()
    private var english: Map<String, String> = emptyMap()
    private var loaded: String? = null
    private var context: Context? = null

    fun attach(context: Context) {
        this.context = context.applicationContext
    }

    /** Switches to a stored `settings.locale`; anything unknown reads English. */
    fun use(stored: String?) {
        val next = stored?.takeIf { it == "ja" || it == "zh" } ?: "en"
        if (next == loaded) return
        catalog = load(next)
        if (english.isEmpty()) english = load("en")
        loaded = next
        language = next
    }

    /** The text for a key, `{{name}}` placeholders filled; null when no
     * catalog has the key (a key another component owns, such as Core's). */
    fun lookup(key: String, values: Array<out Pair<String, Any>>): String? {
        var text = catalog[key] ?: english[key] ?: return null
        for ((name, value) in values) text = text.replace("{{$name}}", value.toString())
        return text
    }

    fun string(key: String, values: Array<out Pair<String, Any>>): String = lookup(key, values) ?: key

    /** The locale dates are written in, as the desktop's `intlLocale`. */
    val locale: Locale
        get() = when (language) {
            "ja" -> Locale.JAPAN
            "zh" -> Locale.SIMPLIFIED_CHINESE
            else -> Locale.UK
        }

    /** School text in the content language: `settings.contentLanguage`, or on
     * automatic the UI language (`zh` reads Japanese), as the desktop's
     * `contentLanguage` and `pickLocalized`. The other language fills a gap. */
    fun pick(value: Map<String, String>, settings: JSONObject): String? {
        val chosen = settings.optString("contentLanguage")
        val first = chosen.takeIf { it == "ja" || it == "en" } ?: if (language == "en") "en" else "ja"
        val order = if (first == "en") listOf("en", "ja") else listOf("ja", "en")
        return order.firstNotNullOfOrNull { value[it]?.ifEmpty { null } }
    }

    private fun load(language: String): Map<String, String> = runCatching {
        val text = context?.assets?.open("$language.json")?.bufferedReader()?.use { it.readText() } ?: return emptyMap()
        val json = JSONObject(text)
        json.keys().asSequence().associateWith { json.getString(it) }
    }.getOrDefault(emptyMap())
}

/** The UI string for a catalog key, with `{{name}}` placeholders filled. */
fun t(key: String, vararg values: Pair<String, Any>): String {
    // Reading the state here is what makes callers recompose on a switch.
    Localizer.language
    return Localizer.string(key, values)
}

/** The string for a key another component owns; null when no catalog has it. */
fun lookup(key: String, vararg values: Pair<String, Any>): String? {
    Localizer.language
    return Localizer.lookup(key, values)
}
