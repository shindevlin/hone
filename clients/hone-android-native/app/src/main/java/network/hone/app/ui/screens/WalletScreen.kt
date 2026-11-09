package network.hone.app.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import network.hone.app.data.Tx
import network.hone.app.data.TxDirection
import network.hone.app.data.TxStatus
import network.hone.app.ui.BiometricGate
import network.hone.app.ui.components.QrCode
import network.hone.app.ui.theme.*

/**
 * Wallet screen. Built around "push button, no copy-paste":
 *   - Receive: shows a QR of the address; a single tap copies it (with haptic).
 *   - Send: button-driven, gated by BiometricGate before the sign-request.
 *   - History: tx list with direction + status.
 * The private key never surfaces here — it stays in the Rust keystore.
 */
@Composable
fun WalletScreen(vm: WalletViewModel = viewModel(), onEarnings: () -> Unit = {}, onSponsor: (() -> Unit)? = null, onKeys: (() -> Unit)? = null) {
    val identity by vm.identity.collectAsStateWithLifecycle()
    val history by vm.history.collectAsStateWithLifecycle()
    var showReceive by remember { mutableStateOf(false) }
    var showKey by remember { mutableStateOf(false) }
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val acct = identity.address
    var positions by remember { mutableStateOf<List<network.hone.app.data.HoneChain.Position>?>(null) }
    LaunchedEffect(acct, identity.balanceHunits) {
        if (acct.isNotEmpty()) positions = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            runCatching { network.hone.app.data.HoneChain.positions(acct) }.getOrNull() } ?: positions
    }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("HONE", fontFamily = OrnDisplay, fontWeight = FontWeight.SemiBold, fontSize = 18.sp, letterSpacing = 3.sp,
                color = HoneOrange, modifier = Modifier.weight(1f))
            Text(identity.name, fontSize = 13.sp, color = HoneTextDim)
        }
        YouHave(identity.balanceHunits.toLong().takeIf { identity.balanceKnown }, positions, acct, onEarnings)

        Segmented(listOf("Send", "Back someone"), tab) { tab = it }
        HoneBox {
            if (acct.isEmpty()) Dim("This phone has no HONE account yet.")
            else if (tab == 0) SendFlow(acct, inline = true) else DelegateFlow(acct, inline = true)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            LineButton("Receive", Modifier.weight(1f)) { showReceive = true }
            LineButton("Sending key", Modifier.weight(1f)) { showKey = true }
        }
        if (onSponsor != null) HoneBox(onClick = onSponsor) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Bring people in", fontWeight = FontWeight.Bold, color = HoneText, modifier = Modifier.weight(1f))
                Icon(Icons.Filled.ChevronRight, contentDescription = null, tint = HoneTextDim)
            }
            Dim("Invite someone, back a new listener, or make vouchers.")
        }

        Text("Recent", fontWeight = FontWeight.Bold, fontSize = 14.sp, color = HoneText)
        if (history.isEmpty()) Dim(if (identity.balanceKnown) "Nothing yet." else "Asking the network…")
        if (history.isNotEmpty()) HoneBox { groupTx(history).take(20).forEach { (tx, n, sum) -> TxRow(tx, n, sum) } }
        if (onKeys != null) TextButton(onKeys) { Text("Keys and grants (for experts)", color = HoneOrange, fontWeight = FontWeight.Bold) }
    }

    if (showReceive) ReceiveSheet(identity.name, identity.address) { showReceive = false }
    if (showKey) ActionSheet({ showKey = false }) { Column(Modifier.padding(24.dp)) { ActiveKeyCard(identity.address) } }
}

/** "You have": everything, split into free to use, set aside for own roles, and backing others. */
@Composable
private fun YouHave(free: Long?, positions: List<network.hone.app.data.HoneChain.Position>?, acct: String, onEarnings: () -> Unit) {
    val own = positions?.filter { network.hone.app.data.HoneChain.isMine(acct, it.node) }?.sumOf { it.amount }
    val others = positions?.filterNot { network.hone.app.data.HoneChain.isMine(acct, it.node) }?.sumOf { it.amount }
    fun f(h: Long?) = h?.let { network.hone.app.data.Plain.hone(it) } ?: "—"
    HoneBox {
        Dim("You have")
        Row(verticalAlignment = Alignment.Bottom) {
            Text(f(if (free == null || own == null || others == null) null else free + own + others),
                fontFamily = HoneMono, fontWeight = FontWeight.Bold, fontSize = 34.sp, color = HoneText)
            Spacer(Modifier.width(6.dp))
            Text("HONE", fontSize = 16.sp, color = HoneOrange, fontFamily = HoneMono, modifier = Modifier.padding(bottom = 6.dp))
        }
        Row(Modifier.fillMaxWidth()) {
            listOf("Free to use" to free, "Set aside" to own, "Backing others" to others).forEach { (k, v) ->
                Column(Modifier.weight(1f)) { Dim(k); Mono(f(v)) }
            }
        }
        TextButton(onEarnings, contentPadding = PaddingValues(0.dp)) {
            Text("Earnings: paid and waiting", color = HoneOrange, fontWeight = FontWeight.Bold)
            Icon(Icons.Filled.ChevronRight, contentDescription = null, tint = HoneOrange)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ActionSheet(onDismiss: () -> Unit, content: @Composable () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface) { content() }
}

/** The same kind of reward collapses into one row ("Paid for keeping time ×12"). Transfers stay single. */
private fun groupTx(h: List<Tx>): List<Triple<Tx, Int, Long>> {
    val out = mutableListOf<Triple<Tx, Int, Long>>()
    val at = HashMap<String, Int>()
    h.forEach { t ->
        val k = t.counterparty + t.direction
        val i = if (t.counterparty.startsWith("@")) null else at[k]
        if (i != null) out[i] = Triple(out[i].first, out[i].second + 1, out[i].third + t.amountHunits.toLong())
        else { if (!t.counterparty.startsWith("@")) at[k] = out.size; out += Triple(t, 1, t.amountHunits.toLong()) }
    }
    return out
}

@Composable
private fun TxRow(tx: Tx, count: Int = 1, sum: Long = tx.amountHunits.toLong()) {
    val inbound = tx.direction == TxDirection.IN
    val sign = if (inbound) "+" else "−"
    val isTransfer = tx.counterparty.startsWith("@")
    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(network.hone.app.data.Plain.entry(if (isTransfer) "transfer" else tx.counterparty, !inbound, tx.counterparty) + if (count > 1) " ×$count" else "",
                fontSize = 14.sp, color = HoneText)
            tx.memo?.let { Text(it, fontSize = 12.sp, color = HoneTextFaint) }
        }
        Column(horizontalAlignment = Alignment.End) {
            Text("$sign${network.hone.app.data.Plain.hone(sum)}", fontFamily = HoneMono, fontSize = 14.sp, color = if (inbound) HoneGreen else HoneText)
            if (tx.status != TxStatus.CONFIRMED) StatusChip(tx.status)
        }
    }
}

@Composable
private fun StatusChip(status: TxStatus) {
    val (text, color) = when (status) {
        TxStatus.CONFIRMED -> "confirmed" to HoneGreen
        TxStatus.PENDING -> "pending" to HoneAmber
        TxStatus.FAILED -> "failed" to HoneRed
    }
    Text(text, style = MaterialTheme.typography.labelSmall, color = color)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ReceiveSheet(name: String, address: String, onDismiss: () -> Unit) {
    val ctx = LocalContext.current
    val haptics = LocalHapticFeedback.current
    var copied by remember { mutableStateOf(false) }
    ModalBottomSheet(onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally) {
            Text("Receive HONE", style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.height(4.dp))
            Text(name, style = MaterialTheme.typography.bodyMedium, color = HoneTextDim)
            Spacer(Modifier.height(20.dp))
            QrCode(content = address)
            Spacer(Modifier.height(20.dp))
            // Tap-to-copy: no manual selection/copy — one button, haptic confirm.
            Button(onClick = {
                copyToClipboard(ctx, address)
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                copied = true
            }, shape = RoundedCornerShape(14.dp)) {
                Icon(if (copied) Icons.Filled.Check else Icons.Filled.ContentCopy,
                    contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(if (copied) "Copied" else "Copy address")
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

private fun copyToClipboard(ctx: Context, text: String) {
    val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    cm.setPrimaryClip(ClipData.newPlainText("HONE address", text))
}
