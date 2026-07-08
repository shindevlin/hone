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
fun WalletScreen(vm: WalletViewModel = viewModel()) {
    val identity by vm.identity.collectAsStateWithLifecycle()
    val history by vm.history.collectAsStateWithLifecycle()
    var showReceive by remember { mutableStateOf(false) }
    var showSend by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp)) {
        Text("Wallet", style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.height(8.dp))
        Text(identity.name, style = MaterialTheme.typography.titleMedium, color = HoneTextDim)

        Spacer(Modifier.height(20.dp))
        BalanceCard(identity.balanceHone, identity.delegatedHone)

        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            ActionButton("Receive", Icons.Filled.CallReceived, Modifier.weight(1f)) { showReceive = true }
            ActionButton("Send", Icons.Filled.CallMade, Modifier.weight(1f), primary = true) { showSend = true }
        }

        Spacer(Modifier.height(24.dp))
        Text("ACTIVITY", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        Spacer(Modifier.height(8.dp))
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(history, key = { it.id }) { TxRow(it) }
        }
    }

    if (showReceive) ReceiveSheet(identity.name, identity.address) { showReceive = false }
    if (showSend) SendSheet(vm) { showSend = false }
}

@Composable
private fun BalanceCard(balance: Double, delegated: Double) {
    Column(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(20.dp))
            .padding(20.dp)
    ) {
        Text("BALANCE", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        Spacer(Modifier.height(6.dp))
        Row(verticalAlignment = Alignment.Bottom) {
            Text("%.6f".format(balance),
                style = MaterialTheme.typography.displayLarge.copy(fontFamily = HoneMono),
                color = MaterialTheme.colorScheme.onSurface)
            Spacer(Modifier.width(8.dp))
            Text("HONE", style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(bottom = 8.dp))
        }
        if (delegated > 0) {
            Spacer(Modifier.height(4.dp))
            Text("+ %.2f HONE delegated".format(delegated),
                style = MaterialTheme.typography.bodyMedium, color = HoneBlue)
        }
    }
}

@Composable
private fun ActionButton(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector,
                         modifier: Modifier, primary: Boolean = false, onClick: () -> Unit) {
    val colors = if (primary)
        ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.primary,
            contentColor = androidx.compose.ui.graphics.Color.Black)
    else ButtonDefaults.filledTonalButtonColors()
    Button(onClick = onClick, modifier = modifier.height(52.dp),
        shape = RoundedCornerShape(14.dp), colors = colors) {
        Icon(icon, contentDescription = null, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Text(label, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun TxRow(tx: Tx) {
    val inbound = tx.direction == TxDirection.IN
    val sign = if (inbound) "+" else "−"
    val amtColor = if (inbound) HoneGreen else MaterialTheme.colorScheme.onSurface
    Row(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(12.dp))
            .padding(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(if (inbound) Icons.Filled.SouthWest else Icons.Filled.NorthEast,
            contentDescription = if (inbound) "received" else "sent",
            tint = if (inbound) HoneGreen else HoneTextDim, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(tx.counterparty, style = MaterialTheme.typography.titleMedium)
            Text("epoch ${tx.epoch}${tx.memo?.let { " · $it" } ?: ""}",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }
        Column(horizontalAlignment = Alignment.End) {
            Text("$sign${"%.6f".format(tx.amountHone)}",
                style = MaterialTheme.typography.titleMedium.copy(fontFamily = HoneMono), color = amtColor)
            StatusChip(tx.status)
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SendSheet(vm: WalletViewModel, onDismiss: () -> Unit) {
    val send by vm.send.collectAsStateWithLifecycle()
    val ctx = LocalContext.current

    // Close the sheet automatically on a successful submit.
    LaunchedEffect(send.lastResult) { if (send.lastResult != null) { vm.clearResult(); onDismiss() } }

    ModalBottomSheet(onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Send HONE", style = MaterialTheme.typography.titleLarge)

            OutlinedTextField(
                value = send.recipient, onValueChange = vm::setRecipient,
                label = { Text("Recipient (@name or hh1…)") },
                singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(
                value = send.amount, onValueChange = vm::setAmount,
                label = { Text("Amount (HONE)") },
                singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(
                value = send.memo, onValueChange = vm::setMemo,
                label = { Text("Memo (optional)") },
                singleLine = true, modifier = Modifier.fillMaxWidth())

            send.error?.let { Text(it, color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium) }

            Button(
                onClick = {
                    // Biometric gate BEFORE the sign-request. Never auto-sign.
                    val activity = ctx as? FragmentActivity
                    if (activity != null && BiometricGate.canAuthenticate(activity)) {
                        BiometricGate.authenticate(activity,
                            title = "Confirm send",
                            subtitle = "Authorize sending ${send.amount} HONE",
                            onSuccess = { vm.confirmSend() },
                            onError = { /* surfaced by keeping sheet open; PIN fallback later */ })
                    } else {
                        // No biometric available: in Phase 2 this routes to a
                        // password/PIN unlock. For the mock, proceed to confirm.
                        vm.confirmSend()
                    }
                },
                enabled = vm.canSend(),
                modifier = Modifier.fillMaxWidth().height(52.dp),
                shape = RoundedCornerShape(14.dp),
            ) {
                if (send.sending) {
                    CircularProgressIndicator(strokeWidth = 2.dp, modifier = Modifier.size(20.dp))
                } else {
                    Icon(Icons.Filled.Fingerprint, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Confirm & sign", fontWeight = FontWeight.SemiBold)
                }
            }
            Spacer(Modifier.height(12.dp))
        }
    }
}

private fun copyToClipboard(ctx: Context, text: String) {
    val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    cm.setPrimaryClip(ClipData.newPlainText("HONE address", text))
}
