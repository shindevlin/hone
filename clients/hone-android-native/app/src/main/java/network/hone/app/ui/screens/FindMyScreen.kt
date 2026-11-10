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
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
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

private val ROLE_HELP = mapOf(
    "memo" to "memo key: decrypts location shares sent to your account.",
    "posting" to "posting key: signs bird uploads from this phone.",
)
// The hide key no longer goes on phones (hardware grant below); the active key lives only in the wallet.
private val PHONE_ROLES = listOf("hide", "posting", "memo")

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
    var keyErr by remember { mutableStateOf<String?>(null) }
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

    // imePadding: the key field and its Import button stay above the keyboard.
    LazyColumn(Modifier.fillMaxSize().imePadding().padding(horizontal = 20.dp).padding(top = 24.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
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
            Text(PHONE_ROLES.joinToString(" · ") { "$it ${if (RoleKeys.has(ctx, it)) "yes" else "no"}" } +
                " · active: Wallet", style = line, color = HoneTextDim)
            Row { PHONE_ROLES.forEach { r -> FilterChip(selected = keyRole == r, onClick = { keyRole = r }, label = { Text(r) }, modifier = Modifier.padding(end = 6.dp)) } }
            Text(ROLE_HELP[keyRole] ?: "", style = line, color = HoneTextDim)
            // The key is never logged and never leaves the phone: RoleKeys wraps it with a
            // non-exportable AndroidKeyStore key. Masked on screen.
            fun submit() {
                val hex = keyHex.trim().lowercase()
                if (!hex.matches(Regex("[0-9a-f]{64}"))) {
                    keyErr = "Expected 64 hex characters (0-9, a-f); got ${hex.length}"; return
                }
                keyErr = null
                val role = keyRole; keyHex = ""
                scope.launch {
                    val chainPub = withContext(Dispatchers.IO) {
                        ChainHttp.get("/api/account/${FindMy.account(ctx)}")?.let { JSONObject(it).optJSONObject("keys")?.optString(role) }
                    }
                    val err = withContext(Dispatchers.IO) { RoleKeys.import(ctx, role, hex, chainPub) }
                    keyErr = err
                    status = if (err != null) "" else ("$role key imported" + if (chainPub.isNullOrEmpty()) " (not registered on chain)" else " (matches chain)")
                    if (err == null && role == "memo") withContext(Dispatchers.IO) { runCatching { FindMy.refreshShared(ctx, store) } }
                    if (err == null && role == "hide") withContext(Dispatchers.IO) { network.hone.app.sightings.HwKeys.forgetGrant(ctx); SightingsEngine.reloadIdentity() }
                    refresh++
                }
            }
            OutlinedTextField(value = keyHex, onValueChange = { keyHex = it.trim(); keyErr = null }, singleLine = true,
                label = { Text("$keyRole private key (64 hex)") }, isError = keyErr != null,
                supportingText = { Text(keyErr ?: "${keyHex.length}/64") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done, autoCorrectEnabled = false),
                keyboardActions = KeyboardActions(onDone = { submit() }),
                modifier = Modifier.fillMaxWidth())
            Button(onClick = { submit() }, enabled = keyHex.isNotEmpty(), modifier = Modifier.fillMaxWidth()) {
                Text("Import $keyRole key")
            }
        }
        // ── hardware grant ──
        item { HardwareGrantCard(onChanged = { refresh++ }) { status = it } }
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
                                k to v
                            }
                        }.let { r -> r.fold({ (k, v) -> "share $shareDev -> $grantee: " + FindMy.submitMemory(ctx, k, v, signer(ctx, "Share location")) },
                            { it.message ?: "error" }) }
                    }
                }) { Text("Share (this month + next)") }
                TextButton(enabled = grantee.isNotBlank(), onClick = {
                    scope.launch {
                        status = runCatching {
                            val dev = devicePubFor(shareDev) ?: error("unknown device")
                            "revoke $shareDev -> $grantee: " + FindMy.submitMemory(ctx, FindMy.grantKeyName(grantee, dev), null, signer(ctx, "Revoke location share"))
                        }.getOrElse { it.message ?: "error" }
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

/** Signs with the wallet's active key (confirm screen + password per signature). */
private fun signer(ctx: android.content.Context, title: String): suspend (String) -> String? = { msg ->
    var c: android.content.Context? = ctx
    while (c is android.content.ContextWrapper && c !is androidx.fragment.app.FragmentActivity) c = c.baseContext
    val act = c as? androidx.fragment.app.FragmentActivity
    if (act == null || !network.hone.app.data.ActiveKeyVault.has(ctx)) null
    else (network.hone.app.data.ActiveKeyVault.sign(act, msg,
            network.hone.app.data.VaultPrompt.Confirm(title, null, "Your account memory (Find My sharing entry)"))
        as? network.hone.app.data.ActiveKeyVault.Signed.Ok)?.signatureHex
}

/** Standing hardware grant: export the attested hardware key, import the wrapped seeds. */
@Composable
private fun HardwareGrantCard(onChanged: () -> Unit, setStatus: (String) -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    val line = MaterialTheme.typography.labelSmall
    val info = remember(tick) { runCatching { network.hone.app.sightings.HwKeys.info() }.getOrNull() }
    val grant = remember(tick) { network.hone.app.sightings.HwKeys.grant(ctx) }
    val devs = remember(tick) { network.hone.app.sightings.HwKeys.granted(ctx) }
    fun run(label: String, f: () -> String) = scope.launch {
        setStatus(withContext(Dispatchers.IO) { runCatching(f).getOrElse { "$label failed: ${it.message}" } }); tick++; onChanged()
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Hardware grant", fontWeight = FontWeight.SemiBold)
        Text("Your hide key stays on your wallet host. It grants this phone's hardware key; device seeds are " +
            "imported into the phone's secure hardware and never leave it.", style = line, color = HoneTextDim)
        Text(if (info == null) "hardware key: not created yet" else "hardware key ${info.hwKey.take(12)}… in ${info.securityLevel}",
            style = line, color = HoneTextDim)
        Text(if (grant == null) "no grant imported" else
            "grant from @${grant.optString("owner")}: ${devs.size} device(s)" + if (grant.optBoolean("dry_run")) " · TEST grant" else "",
            style = line, color = if (grant == null) HoneTextFaint else HoneGreen)
        Row {
            TextButton(onClick = { run("export") {
                val f = network.hone.app.sightings.HwKeys.export(ctx, FindMy.account(ctx)); "hardware key exported: ${f.path}" } }) { Text("Export hardware key") }
            TextButton(onClick = { run("import") { network.hone.app.sightings.HwKeys.importBundle(ctx).also { SightingsEngine.reloadIdentity() } } }) { Text("Import grant") }
        }
        if (grant != null) Row {
            if (grant.has("selftest")) TextButton(onClick = { run("self-test") {
                val (ok, n, pt) = network.hone.app.sightings.HwKeys.selfTest(ctx)
                "self-test: lids $ok/$n match · test sighting " + (if (pt != null) "decrypted" else "NOT decrypted")
            } }) { Text("Self-test") }
            TextButton(onClick = { run("forget") { network.hone.app.sightings.HwKeys.forgetGrant(ctx); SightingsEngine.reloadIdentity(); "grant removed from this phone" } }) { Text("Remove grant") }
        }
    }
}
