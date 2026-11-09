package network.hone.app.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
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
import org.json.JSONObject

/** Bring people in: invite link, back a new listener, vouchers (chain-gated), and who you back. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SponsorScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val acct = remember { runCatching { java.io.File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("") }
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var backSheet by remember { mutableStateOf(false) }
    var vouchers by remember { mutableStateOf(ChainGate.Unknown) }
    var free by remember { mutableStateOf<Long?>(null) }
    var backed by remember { mutableStateOf<List<Pair<HoneChain.Position, JSONObject?>>?>(null) }
    LaunchedEffect(acct) {
        withContext(Dispatchers.IO) {
            vouchers = when (HoneChain.vouchersOn()) { true -> ChainGate.Live; false -> ChainGate.Off; null -> ChainGate.Unknown }
            if (acct.isNotEmpty()) {
                free = network.hone.app.data.ChainHttp.balanceHunits(acct)?.toLong()
                backed = HoneChain.positions(acct)?.filterNot { HoneChain.isMine(acct, it.node) }
                    ?.map { p -> p to (if ('@' in p.node) HoneChain.seat(p.node)?.takeIf { it.optBoolean("registered") } else null) }
            }
        }
    }
    val link = "https://honemesh.net/join?ref=$acct"

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle("Bring people in", Modifier.weight(1f))
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        Segmented(listOf("Invite", "Vouchers", "Backed"), tab) { tab = it }
        when (tab) {
            0 -> {
                HoneBox {
                    Dim("Your invite link")
                    Text(link.removePrefix("https://"), fontFamily = HoneMono, fontWeight = FontWeight.Bold, fontSize = 14.sp, color = HoneText)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        LineButton("Copy link", Modifier.weight(1f), enabled = acct.isNotEmpty()) {
                            (ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("HONE invite", link))
                        }
                        LineButton("Share", Modifier.weight(1f), enabled = acct.isNotEmpty()) {
                            ctx.startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, link), "Share invite"))
                        }
                    }
                    Dim("The link tells HONE who invited them. It holds no key and moves no HONE.", color = HoneTextFaint)
                }
                HoneBox {
                    Text("Back a new listener", fontWeight = FontWeight.Bold, color = HoneText)
                    Dim("Lend HONE to someone's listener so it earns from day one. It stays yours and comes back as their own savings grow.")
                    Mono(free?.let { "${Plain.hone(it)} free to use" } ?: "—", size = 13, bold = false)
                    InkButton("Back a listener", enabled = acct.isNotEmpty()) { backSheet = true }
                }
            }
            1 -> {
                ComingSoon("Vouchers", "Pay for new listeners in advance. A single-use voucher is for one person; a multi-use one " +
                    "(for a club or a class) has a number of places. Each has a claim window; unclaimed HONE comes back to you " +
                    "when it closes, and you can cancel and get a refund any time before.", vouchers)
                if (vouchers == ChainGate.Live) Dim("Making vouchers from the phone arrives in a coming app update.")
                Tech("SPONSOR_VOUCHER_CREATE / CANCEL · SENSOR_SEAT_CLAIM · up to 100 places, 30-day window")
            }
            else -> {
                Dim("Listeners you back. Your HONE returns as each one's own savings grow.")
                val b = backed
                when {
                    acct.isEmpty() -> Dim("This phone has no HONE account.")
                    b == null -> Dim("Asking the network…")
                    b.isEmpty() -> Dim("You don't back anyone yet.")
                }
                b?.forEach { (p, seat) -> BackedCard(p, seat) }
            }
        }
    }
    if (backSheet) ModalBottomSheet({ backSheet = false }, containerColor = HoneSurface) {
        DelegateFlow(acct, initialNode = "", initialRole = "sensor")
    }
}

@Composable
private fun BackedCard(p: HoneChain.Position, seat: JSONObject?) {
    HoneBox {
        Row {
            Text("@" + p.node.substringBefore('@') + if ('@' in p.node) " · device ${p.node.substringAfter('@').take(6)}" else "",
                fontWeight = FontWeight.Bold, color = HoneText, modifier = Modifier.weight(1f))
            Mono(Plain.hone(p.amount), size = 13)
        }
        if (seat != null) {
            val target = seat.optLong("payback_target").coerceAtLeast(1)
            val own = seat.optLong("owner_stake")
            val txt = "${Plain.hone(own.coerceAtMost(target))} of ${Plain.hone(target)} paid back"
            ProgressLine(own.toFloat() / target, label = txt)
            Dim(txt)
        } else Dim("${Plain.role(p.role)} · you can take it back from Wallet › Back someone")
        Tech("${p.role} stake on ${p.node.take(40)}")
    }
}
