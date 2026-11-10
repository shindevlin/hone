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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.data.ActiveKeyVault
import network.hone.app.data.AgentKey
import network.hone.app.data.AgentKeyMessage
import network.hone.app.data.VaultPrompt
import network.hone.app.data.WalletChain
import network.hone.app.ui.theme.HoneMono
import network.hone.app.ui.theme.HoneOrange
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

private const val UPGRADE = "Available after network upgrade"

@Composable
private fun rememberAgentStatus(acct: String, tick: Int): AgentKey.Status? {
    var st by remember { mutableStateOf<AgentKey.Status?>(null) }
    LaunchedEffect(acct, tick) {
        st = withContext(Dispatchers.IO) { runCatching { AgentKey.status(acct) }.getOrDefault(AgentKey.Status(false, null, null, null, null, null, null)) }
    }
    return st
}

private fun revokePlan(acct: String, current: String?) = Plan(
    "Revoke agent key", listOf("Action" to "Revoke agent key (REVOKE_AGENT_KEY)", "Account" to "@$acct", "Agent key" to (current?.take(16)?.plus("…") ?: "-")),
    null,
    signAt = { n, chain, ep -> AgentKeyMessage.revoke(chain, acct, n, ep) },
    submit = { n, ep, sig -> AgentKey.submitRevoke(acct, n, ep, sig) },
    confirm = VaultPrompt.Confirm("Revoke agent key (REVOKE_AGENT_KEY)", null, "@$acct agent key ${current?.take(12) ?: ""}…".trim()))

@Composable
private fun Header(title: String, onBack: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
        TextButton(onBack) { Text("Back", color = HoneOrange) }
    }
}

@Composable
private fun GateNote(st: AgentKey.Status?) {
    val text = when {
        st == null -> "Asking the network…"
        st.gateActive -> null
        else -> "$UPGRADE. Signing is turned off until the chain reports agent keys active."
    }
    text?.let { Text(it, color = if (st == null) HoneTextDim else MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
}

/** Settings > Account keys > Add agent key. */
@Composable
fun AgentKeyScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val acct = remember { witnessAccount(ctx) }
    var tick by remember { mutableIntStateOf(0) }
    val st = rememberAgentStatus(acct, tick)
    var keyTick by remember { mutableIntStateOf(0) }
    val hasActive = remember(keyTick) { ActiveKeyVault.has(ctx) }
    var fromPhone by remember { mutableStateOf(true) }
    var phonePub by remember { mutableStateOf(AgentKey.phonePub(ctx)) }
    var pasted by remember { mutableStateOf("") }
    var step by remember { mutableStateOf<Step>(Step.Form) }
    val gate = st?.gateActive == true
    val line = MaterialTheme.typography.labelSmall

    Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Header("Add agent key", onBack)
        Text(AgentKeyText.scope(acct), fontWeight = FontWeight.Medium)
        Text("Stake cap: ${WalletChain.fmt(AgentKey.CAP_HUNITS)} (${"%,d".format(AgentKey.CAP_HUNITS)} hunits). " +
            "The agent acts as @$acct itself, only for @$acct.", style = line, color = HoneTextDim)
        GateNote(st)
        st?.pubkey?.let { Text("Current agent key on chain: ${it.take(16)}…", style = line, color = HoneTextDim, fontFamily = HoneMono) }
        if (!hasActive) { Text("Setting the agent key needs your sending key first.", color = HoneTextDim); ActiveKeyCard(acct) { keyTick++ }; return@Column }

        Flow(acct, "Set agent key", step, { step = it; if (it is Step.Status && it.status == "committed") tick++ }, inline = true) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(fromPhone, { fromPhone = true }, label = { Text("Make one on this phone") })
                FilterChip(!fromPhone, { fromPhone = false }, label = { Text("From my agents") })
            }
            if (fromPhone) {
                val p = phonePub
                if (p == null) OutlinedButton({ phonePub = AgentKey.generateOnPhone(ctx) }) { Text("Generate agent key") }
                else {
                    Text("Agent public key (private half stays in this phone's keystore):", style = line, color = HoneTextDim)
                    Text(p, style = line, fontFamily = HoneMono)
                    TextButton({ AgentKey.removePhoneKey(ctx); phonePub = null }) { Text("Discard and make another") }
                }
            } else {
                OutlinedTextField(pasted, { pasted = it.trim().lowercase() }, singleLine = true,
                    label = { Text("Agent public key (64 hex)") }, isError = pasted.isNotEmpty() && !AgentKey.validPub(pasted),
                    supportingText = { Text("${pasted.length}/64 · public key only, never a private key") }, modifier = Modifier.fillMaxWidth())
            }
            val pub = if (fromPhone) phonePub else pasted.takeIf { AgentKey.validPub(it) }
            Button(enabled = gate && pub != null, modifier = Modifier.fillMaxWidth().height(52.dp), onClick = {
                val k = pub!!
                step = Step.Review(Plan("Set agent key",
                    listOf("Action" to "Set agent key (SET_AGENT_KEY)", "Account" to "@$acct", "Agent key" to k.take(16) + "…",
                        "Stake cap" to WalletChain.fmt(AgentKey.CAP_HUNITS), "Moves HONE" to "No"),
                    if (st?.pubkey == k) "This key is already the agent key" else null,
                    signAt = { n, chain, ep -> AgentKeyMessage.set(chain, acct, k, AgentKey.CAP_HUNITS, n, ep) },
                    submit = { n, ep, sig -> AgentKey.submitSet(acct, k, AgentKey.CAP_HUNITS, n, ep, sig) },
                    confirm = VaultPrompt.Confirm("Set agent key (SET_AGENT_KEY)", null, "@$acct agent key ${k.take(12)}…",
                        listOf("Stake cap" to WalletChain.fmt(AgentKey.CAP_HUNITS)))))
            }) { Text(if (gate) "Review and sign" else UPGRADE) }
            OutlinedButton(enabled = gate && st?.pubkey != null, modifier = Modifier.fillMaxWidth(), onClick = {
                step = Step.Review(revokePlan(acct, st?.pubkey))
            }) { Text(if (gate) "Revoke agent key" else "Revoke · $UPGRADE") }
        }
    }
}

/** What the agent key has staked against its cap, and where. */
@Composable
fun AgentWalletScreen(onBack: () -> Unit, onAddAgent: () -> Unit) {
    val ctx = LocalContext.current
    val acct = remember { witnessAccount(ctx) }
    var tick by remember { mutableIntStateOf(0) }
    val st = rememberAgentStatus(acct, tick)
    var step by remember { mutableStateOf<Step>(Step.Form) }
    val line = MaterialTheme.typography.labelSmall
    val gate = st?.gateActive == true

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Header("Agent wallet", onBack)
        Text(AgentKeyText.scope(acct), style = line, color = HoneTextDim)
        GateNote(st)
        if (st == null) return@Column
        if (st.pubkey == null) {
            Text(if (gate) "No agent key is set for @$acct." else "No agent key yet.", color = HoneTextDim)
            OutlinedButton(onAddAgent) { Text("Add agent key") }
            return@Column
        }
        val cap = st.cap ?: AgentKey.CAP_HUNITS
        val staked = st.staked ?: 0L
        Text("Agent key ${st.pubkey.take(16)}…" + if (st.pubkey == AgentKey.phonePub(ctx)) " · made on this phone" else "",
            style = line, fontFamily = HoneMono)
        Text("Staked ${WalletChain.fmt(staked)} of ${WalletChain.fmt(cap)}", fontWeight = FontWeight.SemiBold)
        LinearProgressIndicator(progress = { (staked.toDouble() / cap).toFloat().coerceIn(0f, 1f) }, modifier = Modifier.fillMaxWidth())
        Text("Room left: ${WalletChain.fmt((cap - staked).coerceAtLeast(0))}", style = line, color = HoneTextDim)
        Text("Seats", fontWeight = FontWeight.SemiBold)
        val stakes = st.stakes
        when {
            stakes == null -> Text("The network does not list the agent's stakes per seat yet.", style = line, color = HoneTextFaint)
            stakes.isEmpty() -> Text("Nothing staked by the agent.", style = line, color = HoneTextFaint)
            else -> stakes.forEach { s ->
                Row { Text("@${s.node}", Modifier.weight(1f), fontFamily = HoneMono, style = line)
                    Text("${network.hone.app.data.Plain.role(s.role)} · ${WalletChain.fmt(s.amount)}", style = line) }
            }
        }
        HorizontalDivider()
        if (!ActiveKeyVault.has(ctx)) Text("Revoking needs your sending key on this phone (Account keys).", style = line, color = HoneTextDim)
        else Flow(acct, "Revoke agent key", step, { step = it; if (it is Step.Status && it.status == "committed") tick++ }, inline = true) {
            OutlinedButton(enabled = gate, modifier = Modifier.fillMaxWidth(), onClick = { step = Step.Review(revokePlan(acct, st.pubkey)) }) {
                Text(if (gate) "Revoke agent key" else "Revoke · $UPGRADE")
            }
            Text("Revoking stops the agent at once. Stake it placed stays where it is and stays yours.", style = line, color = HoneTextFaint)
        }
    }
}
