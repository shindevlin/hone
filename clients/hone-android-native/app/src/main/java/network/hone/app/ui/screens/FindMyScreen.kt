package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import network.hone.app.data.ChainHttp
import network.hone.app.sightings.DeviceSighting
import network.hone.app.sightings.FindMy
import network.hone.app.sightings.LocalSightingIndex
import network.hone.app.sightings.RoleKeys
import network.hone.app.sightings.SightingStore
import network.hone.app.sightings.SightingsEngine
import network.hone.app.ui.theme.HoneGreen
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private fun ByteArray.hx() = joinToString("") { "%02x".format(it) }

/** Find my things: own devices (from the hide key), devices shared with me (grants), sharing. */
@Composable
fun FindMyScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val store = remember { SightingsEngine.storeOrNull() ?: SightingStore(ctx.applicationContext) }
    val scope = rememberCoroutineScope()
    var results by remember { mutableStateOf<List<DeviceSighting>?>(null) }
    var owned by remember { mutableStateOf(emptyList<Pair<String, String>>()) }
    var shared by remember { mutableStateOf(emptyList<SightingStore.Shared>()) }
    var status by remember { mutableStateOf("") }
    var refresh by remember { mutableIntStateOf(0) }
    var keyRole by remember { mutableStateOf("hide") }
    var keyHex by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }
    var pub by remember { mutableStateOf("") }
    var grantee by remember { mutableStateOf("") }
    var shareDev by remember { mutableStateOf("This phone") }
    val myPub = remember { RoleKeys.devicePub(ctx).hx() }

    LaunchedEffect(refresh) {
        withContext(Dispatchers.IO) {
            owned = store.ownedDevices(); shared = store.sharedDevices()
            results = FindMy.search(ctx, store, listOf(LocalSightingIndex(store)))
        }
    }
    fun devicePubFor(name: String): String? = if (name == "This phone") myPub else owned.firstOrNull { it.first == name }?.second
    val line = MaterialTheme.typography.labelSmall

    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Find my things", style = MaterialTheme.typography.headlineMedium,
                    color = MaterialTheme.colorScheme.primary, modifier = Modifier.weight(1f))
                TextButton(onClick = onBack) { Text("Back") }
            }
            Text("Last 7 days. Sightings: this phone only (indexer later). Account: ${FindMy.account(ctx).ifEmpty { "?" }}",
                style = line, color = HoneTextDim)
            if (status.isNotEmpty()) Text(status, style = line, color = HoneGreen)
        }
        // ── keys ──
        item {
            Text("Wallet keys on this phone", fontWeight = FontWeight.SemiBold)
            Text(RoleKeys.ROLES.joinToString(" · ") { "$it ${if (RoleKeys.has(ctx, it)) "yes" else "no"}" }, style = line, color = HoneTextDim)
            Row { RoleKeys.ROLES.forEach { r -> FilterChip(selected = keyRole == r, onClick = { keyRole = r }, label = { Text(r) }, modifier = Modifier.padding(end = 6.dp)) } }
            OutlinedTextField(value = keyHex, onValueChange = { keyHex = it.trim() }, singleLine = true,
                label = { Text("$keyRole private key (64 hex)") }, modifier = Modifier.fillMaxWidth())
            TextButton(enabled = keyHex.length == 64, onClick = {
                val role = keyRole; val hex = keyHex; keyHex = ""
                scope.launch {
                    val chainPub = withContext(Dispatchers.IO) {
                        ChainHttp.get("/api/account/${FindMy.account(ctx)}")?.let { JSONObject(it).optJSONObject("keys")?.optString(role) }
                    }
                    val err = withContext(Dispatchers.IO) { RoleKeys.import(ctx, role, hex, chainPub) }
                    status = err ?: ("$role key imported" + if (chainPub.isNullOrEmpty()) " (not registered on chain)" else " (matches chain)")
                    if (err == null && role == "hide") withContext(Dispatchers.IO) { SightingsEngine.reloadIdentity() }
                    if (err == null && role == "memo") withContext(Dispatchers.IO) { runCatching { FindMy.refreshShared(ctx, store) } }
                    refresh++
                }
            }) { Text("Import key") }
        }
        // ── my devices ──
        item {
            Text("My devices", fontWeight = FontWeight.SemiBold)
            Text("This phone · ${myPub.take(16)}…", style = line, color = HoneTextDim)
        }
        items(owned, key = { "own-" + it.first }) { (l, p) ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("$l · ${p.take(16)}…", style = line, modifier = Modifier.weight(1f))
                TextButton(onClick = { store.removeOwnedDevice(l); refresh++ }) { Text("Remove") }
            }
        }
        item {
            OutlinedTextField(value = label, onValueChange = { label = it }, singleLine = true,
                label = { Text("Device name") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(value = pub, onValueChange = { pub = it.trim().lowercase() }, singleLine = true,
                label = { Text("Device public key (64 hex)") }, modifier = Modifier.fillMaxWidth())
            TextButton(enabled = label.isNotBlank() && pub.matches(Regex("[0-9a-f]{64}")), onClick = {
                store.putOwnedDevice(label.trim(), pub); label = ""; pub = ""; refresh++
            }) { Text("Add device") }
        }
        // ── sharing ──
        item {
            Text("Share a device's location", fontWeight = FontWeight.SemiBold)
            Row { (listOf("This phone") + owned.map { it.first }).forEach { n ->
                FilterChip(selected = shareDev == n, onClick = { shareDev = n }, label = { Text(n) }, modifier = Modifier.padding(end = 6.dp)) } }
            OutlinedTextField(value = grantee, onValueChange = { grantee = it.trim() }, singleLine = true,
                label = { Text("Account to share with") }, modifier = Modifier.fillMaxWidth())
            Row {
                TextButton(enabled = grantee.isNotBlank(), onClick = {
                    scope.launch {
                        status = withContext(Dispatchers.IO) {
                            runCatching {
                                val dev = devicePubFor(shareDev) ?: error("unknown device")
                                val (k, v) = FindMy.buildGrant(ctx, grantee, dev.chunked(2).map { it.toInt(16).toByte() }.toByteArray(), shareDev)
                                "share $shareDev -> $grantee: " + FindMy.submitMemory(ctx, k, v)
                            }.getOrElse { it.message ?: "error" }
                        }
                    }
                }) { Text("Share (this month + next)") }
                TextButton(enabled = grantee.isNotBlank(), onClick = {
                    scope.launch {
                        status = withContext(Dispatchers.IO) {
                            runCatching {
                                val dev = devicePubFor(shareDev) ?: error("unknown device")
                                "revoke $shareDev -> $grantee: " + FindMy.submitMemory(ctx, FindMy.grantKeyName(grantee, dev), null)
                            }.getOrElse { it.message ?: "error" }
                        }
                    }
                }) { Text("Revoke") }
            }
            Text("Revoking deletes the grant; month keys already received stay valid until those months end.",
                style = line, color = HoneTextFaint)
        }
        // ── shared with me ──
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Shared with me", fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                TextButton(onClick = { scope.launch {
                    val n = withContext(Dispatchers.IO) { runCatching { FindMy.refreshShared(ctx, store) }.getOrDefault(-1) }
                    status = if (RoleKeys.has(ctx, "memo")) "shared with me: $n" else "import your memo key to receive shares"; refresh++
                } }) { Text("Refresh") }
            }
            Text("from: ${FindMy.grantors(ctx).joinToString(", ")}", style = line, color = HoneTextFaint)
            if (shared.isEmpty()) Text("none", style = line, color = HoneTextFaint)
        }
        items(shared, key = { "sh-" + it.grantor + it.device }) { s ->
            val months = runCatching { JSONObject(s.keysJson).keys().asSequence().toList().sorted() }.getOrDefault(emptyList())
            Text("@${s.grantor} · ${s.label} · ${s.device.take(12)}… · months ${months.joinToString(",")}", style = line)
        }
        // ── results ──
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Sightings", fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                TextButton(onClick = { refresh++ }) { Text("Search again") }
            }
            when {
                results == null -> Text("Searching…", color = HoneTextFaint)
                results!!.isEmpty() -> Text("No sightings yet.", style = line, color = HoneTextFaint)
            }
        }
        val fmt = SimpleDateFormat("MMM d HH:mm", Locale.US)
        items(results ?: emptyList()) { s ->
            Column(Modifier.fillMaxWidth()
                .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(12.dp)).padding(12.dp)) {
                Text("${s.device} (@${s.owner}) · ${fmt.format(Date(s.ts))}", fontWeight = FontWeight.Medium)
                Text("cell ${s.cellf ?: s.cell ?: "unlocated"}${s.accM?.let { " · ±$it m" } ?: ""} · ${s.rssi} dBm · window ${s.window} · via ${s.source}",
                    style = line, color = HoneGreen)
            }
        }
    }
}
