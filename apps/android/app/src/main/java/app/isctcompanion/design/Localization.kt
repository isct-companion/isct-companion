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
internal class MessageCatalog(context: Context, stored: String?) {
    val language = language(stored)
    private val english = load(context, "en")
    private val messages = if (language == "en") english else load(context, language)

    val locale: Locale
        get() = when (language) {
            "ja" -> Locale.JAPAN
            "zh" -> Locale.SIMPLIFIED_CHINESE
            else -> Locale.UK
        }

    fun lookup(key: String, values: Array<out Pair<String, Any>>): String? {
        var text = messages[key] ?: english[key] ?: return null
        for ((name, value) in values) text = text.replace("{{$name}}", value.toString())
        return text
    }

    fun string(key: String, vararg values: Pair<String, Any>) = lookup(key, values) ?: key

    companion object {
        fun language(stored: String?): String = stored?.takeIf { it == "ja" || it == "zh" } ?: "en"

        private fun load(context: Context, language: String): Map<String, String> = runCatching {
            val text = context.assets.open("$language.json").bufferedReader().use { it.readText() }
            val json = JSONObject(text)
            json.keys().asSequence().associateWith { json.getString(it) }
        }.getOrDefault(emptyMap())
    }
}

object Localizer {
    var language by mutableStateOf("en")
        private set
    private var catalog: MessageCatalog? = null
    private var context: Context? = null

    fun attach(context: Context) {
        this.context = context.applicationContext
        use(context.resources.configuration.locales[0].language)
    }

    fun use(stored: String?) {
        val context = context ?: return
        val next = MessageCatalog.language(stored)
        if (next == catalog?.language) return
        catalog = MessageCatalog(context, next)
        language = next
    }

    fun lookup(key: String, values: Array<out Pair<String, Any>>): String? = catalog?.lookup(key, values)
    fun string(key: String, values: Array<out Pair<String, Any>>): String = lookup(key, values) ?: key
    val locale: Locale get() = catalog?.locale ?: Locale.UK

    /** School text in the content language: `settings.contentLanguage`, or on
     * automatic the UI language (`zh` reads Japanese), as the desktop's
     * `contentLanguage` and `pickLocalized`. The other language fills a gap. */
    fun pick(value: Map<String, String>, settings: JSONObject): String? {
        val chosen = settings.optString("contentLanguage")
        val first = chosen.takeIf { it == "ja" || it == "en" } ?: if (language == "en") "en" else "ja"
        val order = if (first == "en") listOf("en", "ja") else listOf("ja", "en")
        return order.firstNotNullOfOrNull { value[it]?.ifEmpty { null } }
    }


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
