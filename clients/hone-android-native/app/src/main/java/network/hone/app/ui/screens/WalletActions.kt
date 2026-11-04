package network.hone.app.ui.screens

import android.content.Context
import android.content.ContextWrapper
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import network.hone.app.data.ActiveKeyVault
import network.hone.app.data.CanonicalMessage
import network.hone.app.data.ChainHttp
import network.hone.app.data.WalletChain
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

private fun Context.activity(): FragmentActivity? {
    var c: Context? = this
    while (c is ContextWrapper) { if (c is FragmentActivity) return c; c = c.baseContext }
    return null
}

/** Active-key import / remove. Checked against the chain, stored under a biometric-gated
 *  Keystore key, never logged and never shown again. */
@Composable
fun ActiveKeyCard(account: String, onChanged: () -> Unit = {}) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var has by remember { mutableStateOf(ActiveKeyVault.has(ctx)) }
    var hex by remember { mutableStateOf("") }
    var err by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    val line = MaterialTheme.typography.labelSmall

    fun submit() {
        val h = hex.trim().lowercase()
        if (!h.matches(Regex("[0-9a-f]{64}"))) { err = "Expected 64 hex characters (0-9, a-f); got ${h.length}"; return }
        val act = ctx.activity() ?: run { err = "No activity for the fingerprint prompt"; return }
        hex = ""; err = null; busy = true
        scope.launch {
            val chainPub = withContext(Dispatchers.IO) { runCatching { WalletChain.account(account)?.activePub }.getOrNull() }
            err = ActiveKeyVault.import(act, h, chainPub)
            has = ActiveKeyVault.has(ctx); busy = false; onChanged()
        }
    }

    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Active key", fontWeight = FontWeight.SemiBold)
        if (has) {
            Text("On this phone · ${ActiveKeyVault.pub(ctx)?.take(10)}… · every send or stake asks for your fingerprint or screen lock",
                style = line, color = HoneTextDim)
            TextButton(onClick = { confirmRemove = true }) { Text("Remove key") }
        } else {
            Text("Sending and staking need @$account's ACTIVE key (not the posting key used for birds). " +
                "It is checked against the chain, stored encrypted and locked to your fingerprint or screen lock.",
                style = line, color = HoneTextDim)
            OutlinedTextField(value = hex, onValueChange = { hex = it.trim(); err = null }, singleLine = true,
                label = { Text("active private key (64 hex)") }, isError = err != null,
                supportingText = { Text(err ?: "${hex.length}/64") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done, autoCorrectEnabled = false),
                keyboardActions = KeyboardActions(onDone = { submit() }), modifier = Modifier.fillMaxWidth())
            Button(onClick = { submit() }, enabled = hex.isNotEmpty() && !busy, modifier = Modifier.fillMaxWidth()) {
                Text(if (busy) "Checking…" else "Import active key")
            }
        }
    }
    if (confirmRemove) AlertDialog(onDismissRequest = { confirmRemove = false },
        title = { Text("Remove active key?") },
        text = { Text("This phone will no longer be able to send or stake. Your account and funds are not affected.") },
        confirmButton = { TextButton(onClick = { ActiveKeyVault.remove(ctx); has = false; confirmRemove = false; onChanged() }) { Text("Remove") } },
        dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text("Cancel") } })
}

/** A reviewed action: rows to show, the message to sign (nonce fetched at confirm time), and the submit call. */
internal class Plan(
    val title: String,
    val rows: List<Pair<String, String>>,
    val warning: String?,
    val sign: (nonce: Long, chainId: String) -> String,
    val submit: (nonce: Long, epoch: Long, sig: String) -> WalletChain.Submit,
)

internal sealed class Step {
    data object Form : Step()
    data class Review(val plan: Plan) : Step()
    data class Working(val text: String) : Step()
    data class Status(val hash: String, val status: String, val reason: String?) : Step()
    data class Failed(val reason: String) : Step()
}
@Composable
internal fun Flow(account: String, title: String, step: Step, setStep: (Step) -> Unit, form: @Composable () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    Column(Modifier.fillMaxWidth().imePadding().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(title, style = MaterialTheme.typography.titleLarge)
        when (step) {
            Step.Form -> form()
            is Step.Review -> {
                val p = step.plan
                Text("Check before signing", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                p.rows.forEach { (k, v) ->
                    Row { Text(k, color = HoneTextDim, modifier = Modifier.width(120.dp)); Text(v, fontWeight = FontWeight.Medium) }
                }
                p.warning?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
                Button(enabled = p.warning == null, modifier = Modifier.fillMaxWidth().height(52.dp), shape = RoundedCornerShape(14.dp), onClick = {
                    val act = ctx.activity() ?: run { setStep(Step.Failed("No activity for the fingerprint prompt")); return@Button }
                    scope.launch { runPlan(act, account, p, setStep) }
                }) { Text("Confirm with fingerprint", fontWeight = FontWeight.SemiBold) }
                OutlinedButton(onClick = { setStep(Step.Form) }, modifier = Modifier.fillMaxWidth()) { Text("Back") }
            }
            is Step.Working -> Row { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Spacer(Modifier.width(12.dp)); Text(step.text) }
            is Step.Status -> {
                val color = when (step.status) { "committed" -> MaterialTheme.colorScheme.primary; "rejected" -> MaterialTheme.colorScheme.error; else -> HoneTextDim }
                Text("Status: ${step.status}", color = color, fontWeight = FontWeight.SemiBold)
                step.reason?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
                Text("Entry ${step.hash.take(16)}…", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                if (step.status == "pending") Text("Waiting for the next seal…", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            }
            is Step.Failed -> {
                Text(step.reason, color = MaterialTheme.colorScheme.error)
                OutlinedButton(onClick = { setStep(Step.Form) }, modifier = Modifier.fillMaxWidth()) { Text("Back") }
            }
        }
        Spacer(Modifier.height(12.dp))
    }
}

/** Fetch nonce + chain id, prompt, sign, submit with the current epoch, then poll status. */
private suspend fun runPlan(act: FragmentActivity, account: String, p: Plan, setStep: (Step) -> Unit) {
    setStep(Step.Working("Preparing…"))
    val pre = withContext(Dispatchers.IO) { runCatching { WalletChain.account(account) to WalletChain.nodeInfo() } }
    val (acct, info) = pre.getOrElse { setStep(Step.Failed(it.message ?: "no HONE node reachable")); return }
    if (acct == null) { setStep(Step.Failed("@$account is not on chain")); return }
    if (acct.activePub != ActiveKeyVault.pub(act)) {
        setStep(Step.Failed("The stored key no longer matches @$account's active key on chain. Remove it and import the current one.")); return
    }
    val nonce = acct.nonce + 1
    val msg = p.sign(nonce, info.chainId)
    when (val s = ActiveKeyVault.sign(act, msg, p.title, p.rows.joinToString(" · ") { it.second })) {
        is ActiveKeyVault.Signed.Failed -> setStep(Step.Failed(s.reason))
        is ActiveKeyVault.Signed.Ok -> {
            setStep(Step.Working("Submitting…"))
            val r = withContext(Dispatchers.IO) {
                runCatching { p.submit(nonce, WalletChain.nodeInfo().epoch, s.signatureHex) }
                    .getOrElse { WalletChain.Submit(false, null, it.message) }
            }
            val hash = r.hash
            if (!r.accepted || hash == null) { setStep(Step.Failed("Rejected: ${r.error ?: "unknown error"}")); return }
            setStep(Step.Status(hash, "pending", null))
            repeat(60) {
                delay(3_000)
                val (st, why) = withContext(Dispatchers.IO) { runCatching { WalletChain.status(hash) }.getOrDefault("unreachable" to null) }
                setStep(Step.Status(hash, st, why))
                if (st == "committed" || st == "rejected") return
            }
        }
    }
}

@Composable
fun SendFlow(account: String) {
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var step by remember { mutableStateOf<Step>(Step.Form) }
    var to by remember { mutableStateOf("") }
    var amount by remember { mutableStateOf("") }
    var memo by remember { mutableStateOf("") }
    var err by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var keyTick by remember { mutableIntStateOf(0) }
    val hasKey = remember(keyTick) { ActiveKeyVault.has(ctx) }
    Flow(account, "Send HONE", step, { step = it }) {
        if (!hasKey) { ActiveKeyCard(account) { keyTick++ }; return@Flow }
        OutlinedTextField(to, { to = it.trim(); err = null }, label = { Text("To (account or name)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(amount, { amount = it.trim(); err = null }, label = { Text("Amount (HONE)") }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.fillMaxWidth())
        OutlinedTextField(memo, { memo = it }, label = { Text("Memo (optional, public)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        err?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Button(enabled = to.isNotEmpty() && amount.isNotEmpty() && !busy, modifier = Modifier.fillMaxWidth().height(52.dp), onClick = {
            val amt = WalletChain.parseHone(amount) ?: run { err = "Enter a positive amount, up to 10 decimals"; return@Button }
            busy = true
            scope.launch {
                val r = withContext(Dispatchers.IO) { runCatching {
                    val dest = WalletChain.resolveRecipient(to).getOrThrow()
                    val bal = ChainHttp.balanceHunits(account)?.toLong() ?: error("could not read balance")
                    Triple(dest, bal, WalletChain.baseFee() * WalletChain.WEIGHT_TRANSFER)
                } }
                busy = false
                r.onFailure { err = it.message }.onSuccess { (dest, bal, fee) ->
                    val after = bal - amt - fee
                    val typed = to.removePrefix("@").lowercase()
                    step = Step.Review(Plan("Send ${WalletChain.fmt(amt)}",
                        listOfNotNull("To" to ("@$dest" + if (dest != typed) " (name \"$typed\")" else ""),
                            "Amount" to WalletChain.fmt(amt), "Fee" to WalletChain.fmt(fee),
                            "Balance after" to (if (after >= 0) WalletChain.fmt(after) else "-"),
                            memo.takeIf { it.isNotBlank() }?.let { "Memo" to it }),
                        if (dest == account) "You can't send to yourself" else if (after < 0) "Not enough balance (${WalletChain.fmt(bal)})" else null,
                        sign = { n, chain -> CanonicalMessage.transfer(chain, account, dest, amt, WalletChain.TOKEN, n) },
                        submit = { n, _, sig -> WalletChain.submitTransfer(account, dest, amt, memo.ifBlank { null }, n, sig) }))
                }
            }
        }) { Text(if (busy) "Checking…" else "Review") }
    }
}
@Composable
fun DelegateFlow(account: String) {
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var step by remember { mutableStateOf<Step>(Step.Form) }
    var node by remember { mutableStateOf(account) }
    var role by remember { mutableStateOf("clock") }
    var amount by remember { mutableStateOf("") }
    var unstake by remember { mutableStateOf(false) }
    var err by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var stakes by remember { mutableStateOf<List<WalletChain.Stake>?>(null) }
    var keyTick by remember { mutableIntStateOf(0) }
    val hasKey = remember(keyTick) { ActiveKeyVault.has(ctx) }
    LaunchedEffect(node, role, step) {
        stakes = null
        if (WalletChain.validName(node)) stakes = withContext(Dispatchers.IO) { runCatching { WalletChain.stakes(node, role) }.getOrNull() }
    }
    Flow(account, "Stake on a node", step, { step = it }) {
        if (!hasKey) ActiveKeyCard(account) { keyTick++ } else {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(!unstake, { unstake = false }, label = { Text("Stake") })
                FilterChip(unstake, { unstake = true }, label = { Text("Unstake") })
            }
            OutlinedTextField(node, { node = it.trim().lowercase(); err = null }, label = { Text("Node (account or account@device)") },
                singleLine = true, modifier = Modifier.fillMaxWidth())
            Text("Role", style = MaterialTheme.typography.labelMedium, color = HoneTextDim)
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                WalletChain.ROLES.forEach { r -> FilterChip(role == r, { role = r }, label = { Text(r) }) }
            }
            OutlinedTextField(amount, { amount = it.trim(); err = null }, label = { Text("Amount (HONE)") }, singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.fillMaxWidth())
            val s = stakes
            val mine = s?.filter { it.staker == account }?.sumOf { it.amount } ?: 0L
            Text(if (s == null) "loading stakes…"
                else "$role on @$node: ${WalletChain.fmt(s.sumOf { it.amount })} from ${s.size} backer(s) · yours ${WalletChain.fmt(mine)}",
                style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            s?.take(8)?.forEach { Text("  @${it.staker} · ${WalletChain.fmt(it.amount)}", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint) }
            err?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Button(enabled = amount.isNotEmpty() && node.isNotEmpty() && !busy, modifier = Modifier.fillMaxWidth().height(52.dp), onClick = {
                val amt = WalletChain.parseHone(amount) ?: run { err = "Enter a positive amount, up to 10 decimals"; return@Button }
                if (!WalletChain.validName(node)) { err = "Not a valid node id"; return@Button }
                busy = true
                scope.launch {
                    val r = withContext(Dispatchers.IO) { runCatching {
                        val bal = ChainHttp.balanceHunits(account)?.toLong() ?: error("could not read balance")
                        Triple(bal, WalletChain.baseFee() * WalletChain.WEIGHT_ROLE_STAKE,
                            WalletChain.stakes(node, role).filter { it.staker == account }.sumOf { it.amount })
                    } }
                    busy = false
                    r.onFailure { err = it.message }.onSuccess { (bal, fee, myStake) ->
                        val after = if (unstake) bal + amt - fee else bal - amt - fee
                        val warn = when {
                            unstake && amt > myStake -> "You have ${WalletChain.fmt(myStake)} staked on $role @$node"
                            !unstake && after < 0 -> "Not enough balance (${WalletChain.fmt(bal)})"
                            else -> null
                        }
                        val verb = if (unstake) "Unstake" else "Stake"
                        val n0 = node; val r0 = role; val u0 = unstake
                        step = Step.Review(Plan("$verb ${WalletChain.fmt(amt)}",
                            listOf("Action" to verb, "Node" to "@$n0", "Role" to r0, "Amount" to WalletChain.fmt(amt),
                                "Fee" to WalletChain.fmt(fee), "Balance after" to (if (after >= 0) WalletChain.fmt(after) else "-")),
                            warn,
                            sign = { n, chain -> CanonicalMessage.nodeRoleStake(chain, u0, n0, r0, account, amt, n) },
                            submit = { n, epoch, sig -> WalletChain.submitRoleStake(u0, n0, r0, account, amt, epoch, n, sig) }))
                    }
                }
            }) { Text(if (busy) "Checking…" else "Review") }
        }
    }
}