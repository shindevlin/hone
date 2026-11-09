package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.data.ActiveKeyVault
import network.hone.app.sightings.FindMy
import network.hone.app.sightings.RoleKeys
import network.hone.app.telemetry.Telemetry
import network.hone.app.ui.theme.*

/** Keys, grants and node, for experts: plain label, technical name under it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeysScreen(onBack: () -> Unit, onFindMy: () -> Unit, onNode: () -> Unit, vm: NodeViewModel = viewModel()) {
    val ctx = LocalContext.current
    val s by vm.state.collectAsStateWithLifecycle()
    val acct = remember { runCatching { java.io.File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("") }
    var keySheet by remember { mutableStateOf(false) }
    var tick by remember { mutableIntStateOf(0) }
    var grants by remember { mutableStateOf<Int?>(null) }
    var owned by remember { mutableIntStateOf(0) }
    LaunchedEffect(tick) {
        withContext(Dispatchers.IO) {
            val store = network.hone.app.sightings.SightingsEngine.storeOrNull() ?: network.hone.app.sightings.SightingStore(ctx.applicationContext)
            owned = runCatching { store.ownedDevices().size }.getOrDefault(0)
            grants = runCatching { store.sharedDevices().size }.getOrNull()
        }
    }
    val hasActive = remember(tick) { ActiveKeyVault.has(ctx) }
    fun has(r: String) = RoleKeys.has(ctx, r)
    val showName = remember { ctx.getSharedPreferences("hone_birds", android.content.Context.MODE_PRIVATE).getBoolean("show_name", false) }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Settings · for experts", fontSize = 12.sp, color = HoneTextDim)
                HoneTitle("Keys, grants and node")
            }
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        Dim("You never need these to earn or send. Each row says what it is in plain words, with the technical name under it.")
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(HoneSurface).border(1.dp, HoneOutline, RoundedCornerShape(18.dp))) {
            ExpertRow("Your account", "account · owner key", if (acct.isEmpty()) "None" else "@$acct",
                detail = "The owner key controls the account and stays off this phone. Keep its backup safe; nobody at Honemesh can recover it.")
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Sending key", "active key", if (hasActive) "On this phone" else "Not here", if (hasActive) HoneGreen else HoneTextDim,
                detail = "Sends HONE and backs people. Locked to your fingerprint or screen lock; every use asks first.") {
                LineButton(if (hasActive) "Manage" else "Add sending key") { keySheet = true }
            }
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Posting key", "posting key", if (has("posting")) "On this phone" else "Not here",
                detail = "Signs your bird reports and photos. It cannot move HONE.")
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Share key", "memo key", if (has("memo")) "On this phone" else "Not here",
                detail = "Opens locations other people share with you.")
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Hide key", "hide key · hardware grant · Find My", if (has("hide")) "On this phone" else "Granted or not set",
                detail = "Lets your devices say \"this is mine\" without telling anyone else.")
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Sharing permissions", "grants · monthly keys", grants?.let { if (it == 1) "1 shared with you" else "$it shared with you" } ?: "…",
                detail = "Who can see where your things were, and whose things you can see. Revoke any time; past months stay readable until they end.") {
                LineButton("Keys and sharing") { onFindMy() }
            }
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Device keys", "role keys · seat key · sensor ids", if (owned == 0) "This phone" else "${owned + 1} devices",
                detail = "One key per device. This phone: ${Telemetry.deviceId(ctx)}.")
            HorizontalDivider(color = HoneOutline)
            ExpertRow("This phone's helper", "node · bootstrap peers · rounds 30 s", if (s.running) "Connected" else "Stopped",
                if (s.running) HoneGreen else HoneAmber,
                detail = "Talking to ${s.peers} devices" + (s.epoch?.let { " · round %,d".format(it.toLong()) } ?: "") + (s.seat?.let { " · seat $it" } ?: "") + ".") {
                LineButton("Node details") { onNode() }
            }
            HorizontalDivider(color = HoneOutline)
            ExpertRow("Name on birds you named", "namer identity", if (showName && acct.isNotEmpty()) "@$acct" else "Hidden",
                detail = "Whether your account name shows on birds you named (Settings › Birds).")
        }
    }
    if (keySheet) ModalBottomSheet({ keySheet = false; tick++ }, containerColor = HoneSurface) {
        Column(Modifier.padding(24.dp)) { ActiveKeyCard(acct) { tick++ } }
    }
}
