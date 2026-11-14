package network.hone.app.ui.screens

import android.app.LocaleManager
import android.os.Build
import android.os.LocaleList
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import network.hone.app.R

/** App languages (res/xml/locales_config.xml). Empty tag = follow the phone. */
val APP_LANGUAGES = listOf("" to null, "en" to "English", "es" to "Español")

/**
 * Follows the phone language; the override uses Android's per-app language (13+). On older phones the
 * system language applies (the picker explains that).
 */
@Composable
fun LanguagePicker() {
    val ctx = LocalContext.current
    if (Build.VERSION.SDK_INT < 33) { Text(stringResource(R.string.lang_system), style = MaterialTheme.typography.bodyMedium); return }
    val lm = ctx.getSystemService(LocaleManager::class.java)
    var cur by remember { mutableStateOf(lm?.applicationLocales?.toLanguageTags()?.substringBefore(",")?.substringBefore("-") ?: "") }
    Column {
        APP_LANGUAGES.forEach { (tag, name) ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                RadioButton(cur == tag, {
                    cur = tag
                    lm?.applicationLocales = if (tag.isEmpty()) LocaleList.getEmptyLocaleList() else LocaleList.forLanguageTags(tag)
                })
                Text(name ?: stringResource(R.string.lang_system), style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}
