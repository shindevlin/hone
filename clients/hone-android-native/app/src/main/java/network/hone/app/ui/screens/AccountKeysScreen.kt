package network.hone.app.ui.screens

import android.Manifest
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BinaryBitmap
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import network.hone.app.data.ActiveKeyVault
import network.hone.app.data.VaultPrompt
import network.hone.app.data.WalletChain
import network.hone.app.ui.theme.HoneOrange
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint
import java.io.File
import java.util.concurrent.Executors

internal fun Context.fragmentActivity(): FragmentActivity? {
    var c: Context? = this
    while (c is ContextWrapper) { if (c is FragmentActivity) return c; c = c.baseContext }
    return null
}

internal fun witnessAccount(ctx: Context): String =
    runCatching { File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("")

/** Settings > Account keys: sending (active) key vault, agent key, agent wallet. */
@Composable
fun AccountKeysScreen(onBack: () -> Unit, onAddAgent: () -> Unit, onAgentWallet: () -> Unit) {
    val ctx = LocalContext.current
    val acct = remember { witnessAccount(ctx) }
    Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Account keys", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        if (acct.isEmpty()) { Text("This phone is not set up with an account yet.", color = HoneTextDim); return@Column }
        Card(Modifier.fillMaxWidth()) { Column(Modifier.padding(16.dp)) { ActiveKeyCard(acct) } }
        Card(Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Agent key", fontWeight = FontWeight.SemiBold)
                Text(AgentKeyText.scope(acct), style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onAddAgent) { Text("Add agent key") }
                    OutlinedButton(onAgentWallet) { Text("Agent wallet") }
                }
            }
        }
    }
}

internal object AgentKeyText {
    /** Josh's wording, with the account filled in. */
    fun scope(acct: String) = network.hone.app.data.AgentKey.SCOPE_TEXT.replace("josh's", "$acct's")
}

/** Import / remove the active key. Checked against the chain, sealed with a password. */
@Composable
fun ActiveKeyCard(account: String, onChanged: () -> Unit = {}) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    val has = remember(tick) { ActiveKeyVault.has(ctx) }
    val legacy = remember(tick) { ActiveKeyVault.hasLegacy(ctx) }
    var hex by remember { mutableStateOf("") }
    var pw by remember { mutableStateOf("") }
    var pw2 by remember { mutableStateOf("") }
    var bio by remember { mutableStateOf(false) }
    var err by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var scan by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    val line = MaterialTheme.typography.labelSmall

    fun submit() {
        val h = hex.trim().lowercase().removePrefix("0x")
        if (!h.matches(Regex("[0-9a-f]{64}"))) { err = "Expected 64 hex characters (0-9, a-f); got ${h.length}"; return }
        if (pw.length < ActiveKeyVault.MIN_PASSWORD) { err = "Use a password of at least ${ActiveKeyVault.MIN_PASSWORD} characters"; return }
        if (pw != pw2) { err = "The passwords do not match"; return }
        val act = ctx.fragmentActivity() ?: run { err = "No activity"; return }
        val password = pw.toCharArray()
        hex = ""; pw = ""; pw2 = ""; err = null; busy = true
        scope.launch {
            val chainPub = withContext(Dispatchers.IO) { runCatching { WalletChain.account(account)?.activePub }.getOrNull() }
            err = ActiveKeyVault.import(act, h, password, chainPub, bio)
            busy = false; tick++; onChanged()
        }
    }

    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text("Sending key", fontWeight = FontWeight.SemiBold)
        Text("active key · @$account", style = line, color = HoneTextFaint, fontFamily = network.hone.app.ui.theme.HoneMono)
        if (has) {
            Text("On this phone · ${ActiveKeyVault.pub(ctx)?.take(10)}… · encrypted with your password (Argon2id), " +
                "wrapped by the phone's ${ActiveKeyVault.protection()} keystore" +
                (if (ActiveKeyVault.usesBiometric(ctx)) " · fingerprint also required" else "") +
                ". Every signature shows what it signs and asks for the password.", style = line, color = HoneTextDim)
            TextButton(onClick = { confirmRemove = true }) { Text("Remove key") }
        } else {
            if (legacy) {
                Text("The key stored by an older version is locked by fingerprint only and can no longer sign. Import it again with a password, or remove it.",
                    style = line, color = MaterialTheme.colorScheme.error)
                TextButton(onClick = { confirmRemove = true }) { Text("Remove old key") }
            }
            Text("Sending, backing and the agent key need @$account's ACTIVE key (not the posting key used for birds). " +
                "It is checked against the chain and stored only as ciphertext.", style = line, color = HoneTextDim)
            OutlinedTextField(value = hex, onValueChange = { hex = it.trim(); err = null }, singleLine = true,
                label = { Text("active private key (64 hex)") }, isError = err != null,
                supportingText = { Text("${hex.removePrefix("0x").length}/64") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Next, autoCorrectEnabled = false),
                trailingIcon = { TextButton({ scan = true }) { Text("Scan") } },
                modifier = Modifier.fillMaxWidth())
            OutlinedTextField(value = pw, onValueChange = { pw = it; err = null }, singleLine = true,
                label = { Text("New password (${ActiveKeyVault.MIN_PASSWORD}+ characters)") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Next, autoCorrectEnabled = false),
                modifier = Modifier.fillMaxWidth())
            OutlinedTextField(value = pw2, onValueChange = { pw2 = it; err = null }, singleLine = true,
                label = { Text("Password again") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done, autoCorrectEnabled = false),
                modifier = Modifier.fillMaxWidth())
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Also require fingerprint", style = MaterialTheme.typography.bodyMedium)
                    Text("Extra lock. The password is always required.", style = line, color = HoneTextFaint)
                }
                Switch(bio, { bio = it })
            }
            err?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            Text("Nobody can recover this password. If you forget it, remove the key and import it again.", style = line, color = HoneTextFaint)
            Button(onClick = { submit() }, enabled = hex.isNotEmpty() && pw.isNotEmpty() && !busy, modifier = Modifier.fillMaxWidth()) {
                Text(if (busy) "Checking and encrypting…" else "Import active key")
            }
        }
    }
    if (scan) SeedScanDialog(onResult = { hex = it; scan = false }, onClose = { scan = false })
    if (confirmRemove) AlertDialog(onDismissRequest = { confirmRemove = false },
        title = { Text("Remove active key?") },
        text = { Text("This wipes the encrypted key from this phone. It will no longer be able to send, back anyone or set the agent key. Your account and funds are not affected.") },
        confirmButton = { TextButton(onClick = { ActiveKeyVault.remove(ctx); confirmRemove = false; tick++; onChanged() }) { Text("Remove") } },
        dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text("Cancel") } })
}

/** The confirm screen + password for every active-key signature (see [VaultPrompt]). */
@Composable
fun VaultPromptHost() {
    val req = VaultPrompt.pending ?: return
    val c = req.confirm
    var pw by remember(req) { mutableStateOf("") }
    var now by remember(req) { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(req) { while (true) { delay(500); now = System.currentTimeMillis() } }
    val wait = ((c.waitUntil - now) / 1000).coerceAtLeast(0)
    fun answer(v: CharArray?) { pw = ""; req.answer.complete(v) }
    AlertDialog(onDismissRequest = { answer(null) },
        title = { Text("Confirm and sign") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                (listOf("Action" to c.action, "Amount" to c.amountText, "Destination" to c.destination) + c.extra).forEach { (k, v) ->
                    Row { Text(k, color = HoneTextDim, modifier = Modifier.width(96.dp)); Text(v, fontWeight = FontWeight.Medium) }
                }
                Text("Signed with your active key. It is unlocked for this one signature, then locked again.",
                    style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                OutlinedTextField(pw, { pw = it }, singleLine = true, label = { Text("Password") },
                    visualTransformation = PasswordVisualTransformation(), enabled = wait == 0L,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                    modifier = Modifier.fillMaxWidth())
                (if (wait > 0) "Too many wrong passwords. Try again in $wait s." else c.error)?.let {
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = { TextButton(enabled = pw.isNotEmpty() && wait == 0L, onClick = { answer(pw.toCharArray()) }) { Text("Sign") } },
        dismissButton = { TextButton(onClick = { answer(null) }) { Text("Cancel") } })
}

/** Camera QR scan of a 64-hex seed. Frames are decoded on the phone and never stored. */
@Composable
private fun SeedScanDialog(onResult: (String) -> Unit, onClose: () -> Unit) {
    val ctx = LocalContext.current
    var granted by remember { mutableStateOf(ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    LaunchedEffect(Unit) { if (!granted) ask.launch(Manifest.permission.CAMERA) }
    var note by remember { mutableStateOf("Point the camera at the key's QR code") }
    AlertDialog(onDismissRequest = onClose, title = { Text("Scan active key") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (granted) QrPreview(Modifier.fillMaxWidth().aspectRatio(1f)) { text ->
                    val t = text.trim().lowercase().removePrefix("0x")
                    if (t.matches(Regex("[0-9a-f]{64}"))) { onResult(t); true } else { note = "That QR code is not a 64-hex key"; false }
                } else Text("Camera permission is needed to scan.", color = HoneTextDim)
                Text(note, style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            }
        },
        confirmButton = { TextButton(onClose) { Text("Cancel") } })
}

@Composable
private fun QrPreview(modifier: Modifier, onText: (String) -> Boolean) {
    val ctx = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val exec = remember { Executors.newSingleThreadExecutor() }
    var done by remember { mutableStateOf(false) }
    val providerFuture = remember { ProcessCameraProvider.getInstance(ctx) }
    val uses = remember { mutableListOf<androidx.camera.core.UseCase>() }
    DisposableEffect(Unit) {
        onDispose {
            runCatching { providerFuture.get().unbind(*uses.toTypedArray()) }
            exec.shutdown()
        }
    }
    AndroidView(modifier = modifier, factory = { c ->
        PreviewView(c).also { pv ->
            providerFuture.addListener({
                val provider = providerFuture.get()
                val preview = Preview.Builder().build().also { it.setSurfaceProvider(pv.surfaceProvider) }
                val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                val reader = QRCodeReader()
                analysis.setAnalyzer(exec) { img ->
                    try {
                        if (done) return@setAnalyzer
                        val plane = img.planes[0]
                        val buf = plane.buffer
                        val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
                        val text = runCatching {
                            val src = PlanarYUVLuminanceSource(bytes, plane.rowStride, img.height, 0, 0, img.width, img.height, false)
                            reader.decode(BinaryBitmap(HybridBinarizer(src))).text
                        }.getOrNull()
                        bytes.fill(0)
                        reader.reset()
                        if (text != null) ContextCompat.getMainExecutor(c).execute { if (!done && onText(text)) done = true }
                    } finally { img.close() }
                }
                uses.clear(); uses += preview; uses += analysis
                runCatching { provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis) }
            }, ContextCompat.getMainExecutor(c))
        }
    })
}
