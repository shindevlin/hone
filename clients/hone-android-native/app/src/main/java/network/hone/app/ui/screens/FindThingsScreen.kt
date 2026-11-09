package network.hone.app.ui.screens

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.sightings.Cells
import network.hone.app.sightings.DeviceSighting
import network.hone.app.sightings.FindMy
import network.hone.app.sightings.LocalSightingIndex
import network.hone.app.sightings.SightingStore
import network.hone.app.sightings.SightingsEngine
import network.hone.app.ui.theme.*

/** Find my things, in plain words: each thing, when and roughly where it was last seen. */
@Composable
fun FindThingsScreen(onBack: () -> Unit, onAdvanced: () -> Unit) {
    val ctx = LocalContext.current
    var things by remember { mutableStateOf<List<Pair<String, DeviceSighting?>>?>(null) }
    var refresh by remember { mutableIntStateOf(0) }
    var pick by remember { mutableIntStateOf(0) }
    LaunchedEffect(refresh) {
        things = null
        things = withContext(Dispatchers.IO) {
            runCatching {
                val store = SightingsEngine.storeOrNull() ?: SightingStore(ctx.applicationContext)
                val seen = FindMy.search(ctx, store, listOf(LocalSightingIndex(store)))
                val names = (store.ownedDevices().map { it.first } + store.sharedDevices().map { it.label }).distinct()
                names.map { n -> n to seen.firstOrNull { it.device == n } } +
                    seen.filter { s -> s.device !in names }.distinctBy { it.device }.map { it.device to it }
            }.getOrDefault(emptyList())
        }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle("Find my things", Modifier.weight(1f))
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        val t = things
        when {
            t == null -> Dim("Looking through the last 7 days…")
            t.isEmpty() -> Dim("No things yet. Add a tag, a Flipper or an old phone under Keys and sharing.")
        }
        t?.forEachIndexed { i, (name, s) ->
            HoneBox(border = if (i == pick) HoneBlue else HoneOutline, onClick = { pick = i }) {
                Text(name, fontWeight = FontWeight.Bold, color = HoneText)
                Dim(s?.let { "Seen ${ago(it.ts)}" + (it.accM?.let { a -> " · within about $a m" } ?: "") } ?: "Not seen in the last 7 days")
                if (i == pick && s != null) {
                    val cell = s.cellf ?: s.cell
                    val ll = cell?.let { runCatching { Cells.center(it) }.getOrNull() }
                    Tech("cell ${cell ?: "unlocated"} · ${s.rssi} dBm · via ${s.source}")
                    if (ll != null) LineButton("Show on a map") {
                        ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("geo:${ll.first},${ll.second}?q=${ll.first},${ll.second}(${Uri.encode(name)})")))
                    }
                }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            LineButton("Search again", Modifier.weight(1f)) { refresh++ }
            LineButton("Keys and sharing", Modifier.weight(1f)) { onAdvanced() }
        }
        Dim("Other HONE devices spot your things without knowing what or whose they are. Only you, and people you share with, can read where they were.",
            color = HoneTextFaint)
    }
}

private fun ago(ts: Long): String {
    val m = ((System.currentTimeMillis() - ts) / 60_000).coerceAtLeast(0)
    return when { m < 2 -> "just now"; m < 60 -> "$m min ago"; m < 48 * 60 -> "${m / 60} h ago"; else -> "${m / 1440} days ago" }
}
