package network.hone.app.ui.screens

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
import network.hone.app.data.HoneChain
import network.hone.app.data.Plain
import network.hone.app.ui.theme.*

/** Help run the network: Timekeeper (clock), Checking panel (decide), Sensor (sensor). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RolesScreen(onBack: () -> Unit, onKeys: (() -> Unit)? = null) {
    val ctx = LocalContext.current
    val acct = remember { runCatching { java.io.File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("") }
    var mine by remember { mutableStateOf<List<HoneChain.Position>?>(null) }
    var sheet by remember { mutableStateOf<Pair<String, Boolean>?>(null) }
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(acct, tick) {
        if (acct.isNotEmpty()) mine = withContext(Dispatchers.IO) { HoneChain.positions(acct)?.filter { HoneChain.isMine(acct, it.node) } }
    }
    val roles = listOf(
        Triple("clock", "Keeps the network's clock honest by agreeing on time with other timekeepers.", "a device that stays plugged in and online, and HONE set aside."),
        Triple("decide", "Votes on whether a find is real before anyone is paid for it.", "HONE set aside for each device that votes."),
        Triple("sensor", "Lets your listeners and sensors earn at the full rate.", "HONE set aside on your sensor devices; it can grow from your earnings."),
    )
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle("Help run the network", Modifier.weight(1f))
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        Dim("Set HONE aside for a job and your device does it. Jobs earn a share of rewards. Dishonest work can cost part of what you set aside.")
        roles.forEach { (role, what, needs) ->
            val held = mine?.filter { it.role == role }
            val on = !held.isNullOrEmpty()
            HoneBox(border = if (on) HoneGreen else HoneOutline) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(Plain.role(role), fontWeight = FontWeight.Bold, fontSize = 16.sp, color = HoneText, modifier = Modifier.weight(1f))
                    Text(when { mine == null -> "…"; on -> "ON"; else -> "OFF" }, fontSize = 12.sp, fontWeight = FontWeight.Bold,
                        color = if (on) HoneGreen else HoneTextFaint)
                }
                Tech(when (role) { "clock" -> "clock / witness seat"; "decide" -> "decide jury · seat"; else -> "sensor stake" })
                Dim(what)
                Text("Needs: $needs", fontSize = 13.sp, color = HoneText)
                if (on) Dim("Set aside: " + held!!.joinToString(" · ") { "${Plain.hone(it.amount)} on " +
                    (if ('@' in it.node) "device ${it.node.substringAfter('@').take(6)}" else "@${it.node}") })
                if (on) LineButton("Stop this job", Modifier.fillMaxWidth(), enabled = acct.isNotEmpty()) { sheet = role to true }
                else InkButton("Take this job", enabled = acct.isNotEmpty()) { sheet = role to false }
            }
        }
        onKeys?.let { TextButton(it) { Text("Details for experts: keys, grants, node", color = HoneOrange, fontWeight = FontWeight.Bold) } }
    }
    sheet?.let { (role, stop) ->
        ModalBottomSheet({ sheet = null; tick++ }, containerColor = HoneSurface) {
            val node = mine?.firstOrNull { it.role == role }?.node ?: acct
            DelegateFlow(acct, initialNode = node, initialRole = role, initialUnstake = stop)
        }
    }
}
