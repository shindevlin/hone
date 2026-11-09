package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.data.HoneChain
import network.hone.app.data.Plain
import network.hone.app.ui.theme.*
import org.json.JSONObject
import java.net.URL

/** Your devices: this phone, your other HONE devices (from the status index), things you own, and the node card. */
@Composable
fun DevicesScreen(vm: NodeViewModel = viewModel(), onNode: () -> Unit, onSensors: () -> Unit, onRoles: () -> Unit, onFindMy: () -> Unit) {
    val ctx = LocalContext.current
    val s by vm.state.collectAsStateWithLifecycle()
    val acct = remember { runCatching { java.io.File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("") }
    val myDevice = remember { runCatching { network.hone.app.telemetry.Telemetry.deviceId(ctx) }.getOrDefault("") }
    var others by remember { mutableStateOf<List<Pair<String, Boolean>>?>(null) }
    var things by remember { mutableStateOf<List<Pair<String, String>>>(emptyList()) }
    var myRoles by remember { mutableStateOf<List<String>?>(null) }
    LaunchedEffect(acct) {
        while (true) {
            withContext(Dispatchers.IO) {
                if (acct.isNotEmpty()) {
                    others = runCatching {
                        val a = JSONObject(URL("https://honemesh.net/api/devices/status/$acct").readText()).getJSONArray("devices")
                        (0 until a.length()).map { a.getJSONObject(it) }.filter { it.optString("device") != myDevice }
                            .map { it.optString("device") to it.optBoolean("online") }
                    }.getOrNull() ?: others
                    myRoles = HoneChain.positions(acct)?.filter { HoneChain.isMine(acct, it.node) }?.map { it.role }?.distinct() ?: myRoles
                }
                things = runCatching { (network.hone.app.sightings.SightingsEngine.storeOrNull()
                    ?: network.hone.app.sightings.SightingStore(ctx.applicationContext)).ownedDevices() }.getOrDefault(things)
            }
            delay(60_000)
        }
    }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        HoneTitle("Your devices")
        val roles = myRoles.orEmpty().map { Plain.role(it).lowercase() }.sorted()
        DeviceCard("This phone", if (s.running) "Running" else "Stopped", if (s.running) HoneGreen else HoneAmber,
            sub = if (s.running) "Helping the network" + (s.seat?.let { " · timekeeper seat" } ?: "") else "Not helping right now",
            detail = "Its sensors and microphone report every 15 minutes while it runs. Open Sensors for each one.",
            tech = "node · ${s.seat ?: "no seat"} · device ${myDevice.take(8)}") { LineButton("Sensors on this phone") { onSensors() } }
        others?.forEach { (d, on) ->
            DeviceCard(if (d.startsWith("pc") || d.contains("listener")) "PC listener" else "Device ${d.take(6)}",
                if (on) "Online" else "Quiet", if (on) HoneGreen else HoneAmber,
                sub = if (on) "Reported in the last 15 minutes" else "No report lately",
                detail = if (on) "Reporting normally." else "Probably asleep or off Wi-Fi. Nothing is lost; it catches up when it wakes.",
                tech = "device $d")
        }
        things.forEach { (label, pub) ->
            DeviceCard(label, "Yours", HoneBlue, sub = "Spotted by HONE devices that pass by",
                detail = "Its readings are relayed by any HONE device that hears it. Only you can see where it was.",
                tech = "owned device · ${pub.take(16)}…") { LineButton("Find it") { onFindMy() } }
        }
        if (others == null && acct.isNotEmpty()) Dim("Asking for your other devices…")
        NodeCard(s, onToggle = { vm.toggleNode(s) }, onNode = onNode)
        HoneBox(onClick = onRoles) {
            Text("Help run the network", fontWeight = FontWeight.Bold, color = HoneText)
            Dim(if (roles.isEmpty()) "Timekeeper, checking panel and sensor jobs." else "Your account's jobs: " + roles.joinToString(", ") + ".")
        }
    }
}

@Composable
private fun DeviceCard(name: String, status: String, color: Color, sub: String, detail: String, tech: String,
                       action: (@Composable () -> Unit)? = null) {
    var open by remember { mutableStateOf(false) }
    HoneBox(border = if (open) HoneOrange else HoneOutline, onClick = { open = !open }) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Dot(color); Spacer(Modifier.width(10.dp))
            Text(name, fontWeight = FontWeight.Bold, fontSize = 15.sp, color = HoneText, modifier = Modifier.weight(1f))
            Text(status, fontSize = 12.sp, fontWeight = FontWeight.Bold, color = color)
        }
        Dim(sub)
        if (open) { Dim(detail); Tech(tech); action?.invoke() }
    }
}

/** The node card: dark in both palettes, like the design. Start/stop stays here. */
@Composable
private fun NodeCard(s: network.hone.app.data.NodeSnapshot, onToggle: () -> Unit, onNode: () -> Unit) {
    val ink = Color(0xFFF4F2ED); val dim = Color(0xFFC9CDD2)
    Column(Modifier.fillMaxWidth().background(Color(0xFF16191D), RoundedCornerShape(18.dp)).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Dot(if (s.running) Color(0xFF7ED37A) else Color(0xFFE0B860)); Spacer(Modifier.width(8.dp))
            Text(if (s.running) "This phone's helper is connected" else "This phone's helper is stopped", fontWeight = FontWeight.Bold, color = ink)
        }
        Text(buildString {
            append("Talking to ${s.peers} devices")
            s.epoch?.let { append(" · round %,d".format(it.toLong())) }
            s.sealedToday?.let { append(" · $it sealed today") }
        }, fontSize = 13.sp, color = dim)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onToggle, Modifier.weight(1f).height(44.dp), shape = RoundedCornerShape(22.dp)) {
                Text(if (s.running) "Stop helper" else "Start helper", color = ink, fontWeight = FontWeight.Bold)
            }
            OutlinedButton(onNode, Modifier.weight(1f).height(44.dp), shape = RoundedCornerShape(22.dp)) {
                Text("Node details", color = Color(0xFFFFB677), fontWeight = FontWeight.Bold)
            }
        }
    }
}
