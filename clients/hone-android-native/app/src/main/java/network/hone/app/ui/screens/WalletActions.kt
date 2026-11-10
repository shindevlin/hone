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
import network.hone.app.data.VaultPrompt
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

private fun Context.activity(): FragmentActivity? {
    var c: Context? = this
    while (c is ContextWrapper) { if (c is FragmentActivity) return c; c = c.baseContext }
    return null
}

/** A reviewed action: rows to show, the message to sign (nonce fetched at confirm time), and the submit call. */
internal class Plan(
    val title: String,
    val rows: List<Pair<String, String>>,
    val warning: String?,
    val sign: (nonce: Long, chainId: String) -> String = { _, _ -> error("no message") },
    val submit: (nonce: Long, epoch: Long, sig: String) -> WalletChain.Submit,
    /** For entries whose signed bytes include the epoch: sign and submit with the same epoch. */
    val signAt: ((nonce: Long, chainId: String, epoch: Long) -> String)? = null,
    /** What the password screen shows. Defaults to the title, the "Amount" row and the "To"/"Who" row. */
    confirm: VaultPrompt.Confirm? = null,
) {
    val confirm: VaultPrompt.Confirm = confirm ?: VaultPrompt.Confirm(title, null,
        rows.firstOrNull { it.first == "To" || it.first == "Who" }?.second ?: "-",
        amountLabel = rows.firstOrNull { it.first == "Amount" }?.second)
}

internal sealed class Step {
    data object Form : Step()
    data class Review(val plan: Plan) : Step()
    data class Working(val text: String) : Step()
    data class Status(val hash: String, val status: String, val reason: String?) : Step()
    data class Failed(val reason: String) : Step()
}
@Composable
internal fun Flow(account: String, title: String, step: Step, setStep: (Step) -> Unit, inline: Boolean = false, form: @Composable () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    Column(if (inline) Modifier.fillMaxWidth() else Modifier.fillMaxWidth().imePadding().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (!inline) Text(title, style = MaterialTheme.typography.titleLarge)
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
                    val act = ctx.activity() ?: run { setStep(Step.Failed("No activity for the password prompt")); return@Button }
                    scope.launch { runPlan(act, account, p, setStep) }
                }) { Text("Continue to password", fontWeight = FontWeight.SemiBold) }
                OutlinedButton(onClick = { setStep(Step.Form) }, modifier = Modifier.fillMaxWidth()) { Text("Back") }
            }
            is Step.Working -> Row { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Spacer(Modifier.width(12.dp)); Text(step.text) }
            is Step.Status -> {
                val color = when (step.status) { "committed" -> MaterialTheme.colorScheme.primary; "rejected" -> MaterialTheme.colorScheme.error; else -> HoneTextDim }
                Text(when (step.status) { "committed" -> "Done"; "rejected" -> "Not accepted"; "pending" -> "Sent, waiting for the network"; else -> step.status },
                    color = color, fontWeight = FontWeight.SemiBold)
                step.reason?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
                Text("Entry ${step.hash.take(16)}… · ${step.status}", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                if (step.status == "pending") Text("Usually within a minute.", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                if (step.status == "committed" || step.status == "rejected") OutlinedButton(onClick = { setStep(Step.Form) }, modifier = Modifier.fillMaxWidth()) { Text("Done") }
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
    val msg = p.signAt?.invoke(nonce, info.chainId, info.epoch) ?: p.sign(nonce, info.chainId)
    when (val s = ActiveKeyVault.sign(act, msg, p.confirm)) {
        is ActiveKeyVault.Signed.Failed -> setStep(Step.Failed(s.reason))
        is ActiveKeyVault.Signed.Ok -> {
            setStep(Step.Working("Submitting…"))
            val r = withContext(Dispatchers.IO) {
                runCatching { p.submit(nonce, if (p.signAt != null) info.epoch else WalletChain.nodeInfo().epoch, s.signatureHex) }
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
fun SendFlow(account: String, inline: Boolean = false) {
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
    Flow(account, "Send HONE", step, { step = it }, inline) {
        if (!hasKey) { ActiveKeyCard(account) { keyTick++ }; return@Flow }
        OutlinedTextField(to, { to = it.trim(); err = null }, label = { Text("To (name or HONE account)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(amount, { amount = it.trim(); err = null }, label = { Text("Amount (HONE)") }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.fillMaxWidth())
        OutlinedTextField(memo, { memo = it }, label = { Text("Note (optional, everyone can see it)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
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
                            memo.takeIf { it.isNotBlank() }?.let { "Note" to it }),
                        if (dest == account) "You can't send to yourself" else if (after < 0) "Not enough balance (${WalletChain.fmt(bal)})" else null,
                        sign = { n, chain -> CanonicalMessage.transfer(chain, account, dest, amt, WalletChain.TOKEN, n) },
                        submit = { n, _, sig -> WalletChain.submitTransfer(account, dest, amt, memo.ifBlank { null }, n, sig) },
                        confirm = VaultPrompt.Confirm("Send HONE", amt, "@$dest", listOf("Fee" to WalletChain.fmt(fee)))))
                }
            }
        }) { Text(if (busy) "Checking…" else "Review") }
        Text("Arrives in about a minute. The network fee is shown before you confirm.", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
    }
}
@Composable
fun DelegateFlow(account: String, inline: Boolean = false, initialNode: String? = null, initialRole: String = "clock", initialUnstake: Boolean = false) {
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var step by remember { mutableStateOf<Step>(Step.Form) }
    var node by remember { mutableStateOf(initialNode ?: account) }
    var role by remember { mutableStateOf(initialRole) }
    var amount by remember { mutableStateOf("") }
    var unstake by remember { mutableStateOf(initialUnstake) }
    var err by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var stakes by remember { mutableStateOf<List<WalletChain.Stake>?>(null) }
    var keyTick by remember { mutableIntStateOf(0) }
    val hasKey = remember(keyTick) { ActiveKeyVault.has(ctx) }
    LaunchedEffect(node, role, step) {
        stakes = null
        if (WalletChain.validName(node)) stakes = withContext(Dispatchers.IO) { runCatching { WalletChain.stakes(node, role) }.getOrNull() }
    }
    Flow(account, "Back someone", step, { step = it }, inline) {
        if (!hasKey) ActiveKeyCard(account) { keyTick++ } else {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(!unstake, { unstake = false }, label = { Text("Back") })
                FilterChip(unstake, { unstake = true }, label = { Text("Take back") })
            }
            OutlinedTextField(node, { node = it.trim().lowercase(); err = null }, label = { Text("Who (HONE account, or account@device)") },
                singleLine = true, modifier = Modifier.fillMaxWidth())
            Text("For which job", style = MaterialTheme.typography.labelMedium, color = HoneTextDim)
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                WalletChain.ROLES.forEach { r -> FilterChip(role == r, { role = r }, label = { Text(network.hone.app.data.Plain.role(r)) }) }
            }
            Text("role: $role", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint, fontFamily = network.hone.app.ui.theme.HoneMono)
            OutlinedTextField(amount, { amount = it.trim(); err = null }, label = { Text("Amount (HONE)") }, singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.fillMaxWidth())
            val s = stakes
            val mine = s?.filter { it.staker == account }?.sumOf { it.amount } ?: 0L
            Text(if (s == null) "Asking the network…"
                else "${network.hone.app.data.Plain.role(role)} @$node: ${WalletChain.fmt(s.sumOf { it.amount })} from ${s.size} backer(s) · yours ${WalletChain.fmt(mine)}",
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
                            unstake && amt > myStake -> "You back ${network.hone.app.data.Plain.role(role).lowercase()} @$node with ${WalletChain.fmt(myStake)}"
                            !unstake && after < 0 -> "Not enough balance (${WalletChain.fmt(bal)})"
                            else -> null
                        }
                        val verb = if (unstake) "Take back" else "Back"
                        val n0 = node; val r0 = role; val u0 = unstake
                        step = Step.Review(Plan("$verb ${WalletChain.fmt(amt)}",
                            listOf("Action" to verb, "Who" to "@$n0", "Job" to "${network.hone.app.data.Plain.role(r0)} ($r0)", "Amount" to WalletChain.fmt(amt),
                                "Fee" to WalletChain.fmt(fee), "Balance after" to (if (after >= 0) WalletChain.fmt(after) else "-")),
                            warn,
                            sign = { n, chain -> CanonicalMessage.nodeRoleStake(chain, u0, n0, r0, account, amt, n) },
                            submit = { n, epoch, sig -> WalletChain.submitRoleStake(u0, n0, r0, account, amt, epoch, n, sig) },
                            confirm = VaultPrompt.Confirm(if (u0) "Take back stake ($r0)" else "Stake ($r0)", amt,
                                if (u0) "@$account (from @$n0)" else "@$n0", listOf("Fee" to WalletChain.fmt(fee)))))
                    }
                }
            }) { Text(if (busy) "Checking…" else "Review") }
            Text("Backing lends your HONE to someone's device so it can do its job and earn. It stays yours; take it back any time.",
                style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
        }
    }
}