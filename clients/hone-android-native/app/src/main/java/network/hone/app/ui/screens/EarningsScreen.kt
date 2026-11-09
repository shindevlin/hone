package network.hone.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.data.HoneChain
import network.hone.app.data.Plain
import network.hone.app.ui.theme.*

/**
 * Earnings: finds proven useful vs waiting to be checked (striped), with the reason per find.
 * Source: GET /chain/api/sensor/usefulness/:owner. Points, not HONE: the chain pays HONE from them.
 */
@Composable
fun EarningsScreen(onBack: (() -> Unit)? = null) {
    val ctx = LocalContext.current
    val owner = remember { runCatching { java.io.File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("") }
    var u by remember { mutableStateOf<HoneChain.Usefulness?>(null) }
    var failed by remember { mutableStateOf(false) }
    var open by remember { mutableIntStateOf(-1) }
    LaunchedEffect(owner) {
        while (owner.isNotEmpty()) {
            val r = withContext(Dispatchers.IO) { HoneChain.usefulness(owner) }
            if (r != null) { u = r; failed = false } else failed = u == null
            delay(60_000)
        }
    }
    val hours = ((u?.claimWindowEpochs ?: 2880) * 30 / 3600).coerceAtLeast(1)
    val claims = remember(u) { u?.claims?.sortedByDescending { it.claimedEpoch }.orEmpty() }

    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp), contentPadding = PaddingValues(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                HoneTitle("Earnings", Modifier.weight(1f))
                onBack?.let { TextButton(it) { Text("Back", color = HoneOrange) } }
            }
        }
        item {
            HoneBox {
                val x = u
                Row(Modifier.fillMaxWidth()) {
                    Column(Modifier.weight(1f)) {
                        Text("PAID · PROVEN USEFUL", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = HoneGreen)
                        Mono(x?.let { "%,d".format(it.proven) } ?: "—", size = 26)
                        Dim(x?.let { "finds · %,d points".format(it.provenPoints) } ?: "")
                    }
                    Column(Modifier.weight(1f)) {
                        Text("WAITING TO BE CHECKED", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = HoneAmber)
                        Mono(x?.let { "%,d".format(it.pending) } ?: "—", size = 26, color = HoneTextDim)
                        Dim(x?.let { "finds" + if (it.queuedPoints > 0) " · %,d points queued".format(it.queuedPoints) else "" } ?: "")
                    }
                }
                if (x != null) SplitBar(x.proven.toFloat(), x.pending.toFloat(),
                    label = "${x.proven} finds paid, ${x.pending} waiting to be checked")
                Dim("HONE pays for information that is new and checked by someone else's device. Finds wait up to about $hours h to be checked. Repeats earn nothing.")
                if (x != null && x.void > 0) Dim("${x.void} finds were not confirmed in time and earn nothing.", color = HoneTextFaint)
            }
        }
        when {
            owner.isEmpty() -> item { Dim("This phone has no HONE account, so there are no earnings to show.") }
            failed -> item { Dim("Can't reach the network right now. Earnings will show when it answers.") }
            u == null -> item { Dim("Asking the network…") }
            claims.isEmpty() -> item { Dim("No finds yet. Your devices report every 15 minutes.") }
            else -> item { Text("Latest finds", fontWeight = FontWeight.Bold, fontSize = 14.sp, color = HoneText) }
        }
        itemsIndexed(claims, key = { _, c -> "${c.cell}-${c.metric}-${c.window}" }) { i, c ->
            val (col, hollow) = when (c.status) { "proven" -> HoneGreen to false; "pending" -> HoneAmber to true; else -> HoneTextFaint to true }
            HoneBox(onClick = { open = if (open == i) -1 else i }) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Dot(col, 14.dp, hollow)
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text(Plain.metric(c.metric), fontWeight = FontWeight.Bold, fontSize = 14.sp, color = HoneText)
                        Dim(when (c.status) { "proven" -> "Paid"; "pending" -> "Waiting to be checked"; "void" -> "Not paid"; else -> c.status })
                    }
                    Mono(if (c.status == "void") "0 pts" else "${c.points} pts", color = if (c.status == "proven") HoneGreen else HoneTextDim)
                }
                if (open == i) {
                    Dim(Plain.reason(c.status, hours))
                    Tech("${c.metric} · cell ${c.cell} · window ${c.window} · claimed ${c.claimedEpoch}" + (c.settledEpoch?.let { " · settled $it" } ?: ""))
                }
            }
        }
    }
}
