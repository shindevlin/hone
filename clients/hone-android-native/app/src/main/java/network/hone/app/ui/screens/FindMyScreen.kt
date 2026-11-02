package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.sightings.FindMy
import network.hone.app.sightings.LocalSightingIndex
import network.hone.app.sightings.OwnedSighting
import network.hone.app.sightings.SightingStore
import network.hone.app.sightings.SightingsEngine
import network.hone.app.ui.theme.HoneGreen
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Find my things: recompute the rids of the owner's devices (this phone's seed, plus any
 * seed added here) and look them up. Local sightings only for now; the indexer comes later.
 */
@Composable
fun FindMyScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val store = remember { SightingsEngine.storeOrNull() ?: SightingStore(ctx.applicationContext) }
    var results by remember { mutableStateOf<List<OwnedSighting>?>(null) }
    var devices by remember { mutableStateOf(emptyList<String>()) }
    var label by remember { mutableStateOf("") }
    var seed by remember { mutableStateOf("") }
    var refresh by remember { mutableIntStateOf(0) }

    LaunchedEffect(refresh) {
        withContext(Dispatchers.IO) {
            devices = FindMy.ownedSeeds(ctx, store).map { it.first }
            results = FindMy.search(ctx, store, listOf(LocalSightingIndex(store)))
        }
    }

    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp)) {
        Row {
            Text("Find my things", style = MaterialTheme.typography.headlineMedium,
                color = MaterialTheme.colorScheme.primary, modifier = Modifier.weight(1f))
            TextButton(onClick = onBack) { Text("Back") }
        }
        Text("Rids recomputed from your device seeds, last 7 days. Sightings: this phone only (indexer later).",
            style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
        Spacer(Modifier.height(12.dp))
        Text("Devices: ${devices.joinToString(", ")}", style = MaterialTheme.typography.bodyMedium)
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(value = label, onValueChange = { label = it }, singleLine = true,
            label = { Text("Device name") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(value = seed, onValueChange = { seed = it.trim().lowercase() }, singleLine = true,
            label = { Text("Device id_seed (64 hex)") }, modifier = Modifier.fillMaxWidth())
        Row {
            TextButton(enabled = label.isNotBlank() && seed.matches(Regex("[0-9a-f]{64}")), onClick = {
                store.putOwnedDevice(label.trim(), seed); label = ""; seed = ""; refresh++
            }) { Text("Add device") }
            TextButton(onClick = { refresh++ }) { Text("Search again") }
        }
        Spacer(Modifier.height(8.dp))
        val r = results
        when {
            r == null -> Text("Searching…", color = HoneTextFaint)
            r.isEmpty() -> Text("No sightings of your devices in local data yet.",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }
        val fmt = remember { SimpleDateFormat("MMM d HH:mm", Locale.US) }
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(r ?: emptyList()) { s ->
                Column(Modifier.fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(12.dp)).padding(12.dp)) {
                    Text("${s.device} · ${fmt.format(Date(s.ts))}", fontWeight = FontWeight.Medium)
                    Text("cell ${s.cellf?.ifEmpty { null } ?: s.cell9 ?: "unlocated"} · ${s.rssi} dBm · window ${s.window} · via ${s.source}",
                        style = MaterialTheme.typography.labelSmall, color = HoneGreen)
                }
            }
        }
    }
}
