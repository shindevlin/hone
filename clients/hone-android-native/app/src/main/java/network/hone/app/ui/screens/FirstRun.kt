package network.hone.app.ui.screens

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import network.hone.app.R
import network.hone.app.account.AccountStore
import network.hone.app.station.*
import network.hone.app.ui.theme.HoneGreen
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

/** First run: account, welcome, add a camera (or the phone mic), privacy tick, listening. No crypto words here. */
@Composable
fun FirstRunScreen(onDone: () -> Unit, onImportKey: () -> Unit) {
    val ctx = LocalContext.current
    var step by remember { mutableStateOf(if (AccountStore.signedIn(ctx)) "welcome" else "account") }
    when (step) {
        "account" -> AccountFlow(onSignedIn = { step = "welcome" }, onSkip = { step = "welcome" }, onImportKey = onImportKey)
        "welcome" -> Page {
            val n = AccountStore.name(ctx)
            Title(if (n != null) stringResource(R.string.fr_welcome, n) else stringResource(R.string.fr_welcome_noname))
            Spacer(Modifier.height(8.dp))
            AddCameraFlow(onDone = { AccountStore.setOnboarded(ctx); onDone() })
        }
    }
}

@Composable
fun Page(content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 28.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp), content = content)
}

@Composable
fun Title(t: String) = Text(t, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold,
    color = MaterialTheme.colorScheme.onBackground)

@Composable
fun Body(t: String, dim: Boolean = false) = Text(t, style = MaterialTheme.typography.bodyLarge,
    color = if (dim) HoneTextDim else MaterialTheme.colorScheme.onBackground)

@Composable
fun BigButton(t: String, enabled: Boolean = true, onClick: () -> Unit) =
    Button(onClick, enabled = enabled, modifier = Modifier.fillMaxWidth().height(56.dp), shape = RoundedCornerShape(16.dp)) {
        Text(t, style = MaterialTheme.typography.titleMedium)
    }

@Composable
fun AccountFlow(onSignedIn: () -> Unit, onSkip: () -> Unit, onImportKey: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var mode by remember { mutableStateOf("signup") }
    var name by remember { mutableStateOf("") }
    var email by remember { mutableStateOf("") }
    var pw by remember { mutableStateOf("") }
    var ref by remember { mutableStateOf(AccountStore.pendingRef(ctx) ?: "") }
    var tick by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var err by remember { mutableStateOf<String?>(null) }
    var verify by remember { mutableStateOf(false) }
    val consent = if (ref.isBlank() || ref == "none") stringResource(R.string.ob_consent_pool) else stringResource(R.string.ob_consent_ref, ref.trim().lowercase())

    if (verify) { VerifyForm(onDone = onSignedIn); return }
    Page {
        Title(stringResource(R.string.ob_welcome_title))
        Body(stringResource(R.string.ob_welcome_sub), dim = true)
        if (mode == "signup") OutlinedTextField(name, { name = it }, label = { Text(stringResource(R.string.ob_name)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(email, { email = it.trim() }, label = { Text(stringResource(R.string.ob_email)) }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email), modifier = Modifier.fillMaxWidth())
        OutlinedTextField(pw, { pw = it }, label = { Text(stringResource(R.string.ob_password)) }, singleLine = true,
            visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth())
        if (mode == "signup") {
            OutlinedTextField(ref, { ref = it.trim() }, label = { Text(stringResource(R.string.ob_invite_code)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Row(verticalAlignment = Alignment.Top) {
                Checkbox(tick, { tick = it })
                Text(consent, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 12.dp))
            }
        }
        err?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
        val ok = email.contains("@") && pw.length >= (if (mode == "signup") 10 else 1) && (mode == "login" || tick)
        BigButton(stringResource(if (mode == "signup") R.string.ob_create else R.string.ob_sign_in), enabled = ok && !busy) {
            busy = true; err = null
            scope.launch {
                val r = withContext(Dispatchers.IO) {
                    runCatching { if (mode == "signup") AccountStore.signup(ctx, name, email, pw, ref, consent) else AccountStore.login(ctx, email, pw) }
                }
                busy = false
                r.onSuccess { pw = ""; if (mode == "signup") verify = true else onSignedIn() }.onFailure { err = it.message }
            }
        }
        if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        TextButton({ mode = if (mode == "signup") "login" else "signup"; err = null }) {
            Text(stringResource(if (mode == "signup") R.string.ob_have_account else R.string.ob_new_account))
        }
        TextButton(onSkip) { Text(stringResource(R.string.ob_skip), color = HoneTextDim) }
        Spacer(Modifier.height(12.dp))
        TextButton(onImportKey) { Text(stringResource(R.string.ob_import_key), style = MaterialTheme.typography.labelSmall, color = HoneTextFaint) }
    }
}

@Composable
fun VerifyForm(onDone: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var code by remember { mutableStateOf("") }
    var err by remember { mutableStateOf<String?>(null) }
    var info by remember { mutableStateOf<String?>(null) }
    Page {
        Title(stringResource(R.string.ob_verify_title))
        Body(stringResource(R.string.ob_verify_body, AccountStore.email(ctx) ?: ""), dim = true)
        OutlinedTextField(code, { code = it.filter(Char::isDigit).take(6) }, label = { Text(stringResource(R.string.ob_verify_code)) },
            singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.fillMaxWidth())
        err?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        info?.let { Text(it, color = HoneGreen) }
        BigButton(stringResource(R.string.ob_verify_button), enabled = code.length == 6) {
            scope.launch {
                withContext(Dispatchers.IO) { runCatching { AccountStore.verify(ctx, code) } }
                    .onSuccess { onDone() }.onFailure { err = it.message }
            }
        }
        val sent = stringResource(R.string.ob_verify_body, AccountStore.email(ctx) ?: "")
        TextButton({ scope.launch { withContext(Dispatchers.IO) { runCatching { AccountStore.resend(ctx) } }.onSuccess { info = sent }.onFailure { err = it.message } } }) {
            Text(stringResource(R.string.ob_verify_resend))
        }
        TextButton(onDone) { Text(stringResource(R.string.ob_verify_later), color = HoneTextDim) }
    }
}

/** The phone's own IPv4 on Wi-Fi, for the /24 scan. */
fun wifiIp(ctx: Context): String? {
    val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return null
    return cm.allNetworks.mapNotNull { n ->
        val caps = cm.getNetworkCapabilities(n) ?: return@mapNotNull null
        if (!caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI)) return@mapNotNull null
        cm.getLinkProperties(n)?.linkAddresses?.firstOrNull { it.address is java.net.Inet4Address }?.address?.hostAddress
    }.firstOrNull()
}

private fun scanLan(ctx: Context): List<FoundCamera> {
    val wm = ctx.applicationContext.getSystemService(WifiManager::class.java)
    val lock = wm?.createMulticastLock("hone:discovery")?.apply { setReferenceCounted(false); runCatching { acquire() } }
    try { return Discovery.discover(wifiIp(ctx)) } finally { runCatching { lock?.release() } }
}

/** Last known location -> the public cell for a fixed station (home rule applies). */
fun stationCell(ctx: Context): String? {
    network.hone.app.sightings.SightingsEngine.lastFix()?.let { return network.hone.app.birds.BirdsSync.publicCell(ctx, it.latitude, it.longitude, it.accuracy) }
    if (ctx.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) != PackageManager.PERMISSION_GRANTED) return null
    val lm = ctx.getSystemService(android.location.LocationManager::class.java) ?: return null
    @Suppress("MissingPermission")
    val l = lm.getProviders(true).mapNotNull { runCatching { lm.getLastKnownLocation(it) }.getOrNull() }.maxByOrNull { it.time } ?: return null
    return network.hone.app.birds.BirdsSync.publicCell(ctx, l.latitude, l.longitude, l.accuracy)
}

@Composable
fun AddCameraFlow(onDone: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var step by remember { mutableStateOf("scan") }
    var cams by remember { mutableStateOf<List<FoundCamera>>(emptyList()) }
    var pick by remember { mutableStateOf<FoundCamera?>(null) }
    var user by remember { mutableStateOf("admin") }
    var pass by remember { mutableStateOf("") }
    var probe by remember { mutableStateOf<ProbeResult?>(null) }
    var err by remember { mutableStateOf<String?>(null) }
    var tick by remember { mutableStateOf(false) }
    var mic by remember { mutableStateOf(false) }
    val errLogin = stringResource(R.string.fr_bad_login)
    val errAudio = stringResource(R.string.fr_no_audio)
    val errReach = stringResource(R.string.fr_cant_reach)
    val perms = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { _ -> }

    fun scan() { step = "scanning"; scope.launch { cams = withContext(Dispatchers.IO) { runCatching { scanLan(ctx) }.getOrDefault(emptyList()) }; step = "found" } }
    LaunchedEffect(Unit) { if (step == "scan") scan() }

    when (step) {
        "scanning" -> { Body(stringResource(R.string.fr_scanning)); LinearProgressIndicator(Modifier.fillMaxWidth()) }
        "found" -> {
            when {
                wifiIp(ctx) == null -> Body(stringResource(R.string.fr_needs_wifi))
                cams.isEmpty() -> Body(stringResource(R.string.fr_found_none))
                cams.size == 1 -> Body(stringResource(R.string.fr_found_one, cams[0].name ?: cams[0].host))
                else -> Body(stringResource(R.string.fr_found_many, cams.size))
            }
            cams.forEach { c ->
                Card(Modifier.fillMaxWidth()) {
                    Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(c.name ?: "Camera", style = MaterialTheme.typography.titleMedium)
                            Text(c.host, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                        }
                        Button({ pick = c; err = null; step = "login" }) { Text(stringResource(R.string.fr_use_this)) }
                    }
                }
            }
            OutlinedButton(::scan, Modifier.fillMaxWidth()) { Text(stringResource(R.string.fr_scan_again)) }
            TextButton({ mic = true; step = "privacy" }) { Text(stringResource(R.string.fr_use_mic)) }
        }
        "login" -> {
            Title(stringResource(R.string.fr_cam_login))
            Body(stringResource(R.string.fr_cam_login_hint), dim = true)
            OutlinedTextField(user, { user = it }, label = { Text(stringResource(R.string.fr_cam_user)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(pass, { pass = it }, label = { Text(stringResource(R.string.fr_cam_pass)) }, singleLine = true,
                visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth())
            err?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            BigButton(stringResource(R.string.fr_connect)) {
                val cam = pick ?: return@BigButton
                step = "testing"; err = null
                scope.launch {
                    val r = withContext(Dispatchers.IO) { runCatching { StationSetup.connect(cam, user, pass, aac = { AacDecoder(it) }, arpMac = StationStore::arpMac) } }
                    r.onSuccess { probe = it; step = "tier" }.onFailure { e ->
                        err = when ((e as? RtspException)?.code) { 401 -> errLogin; 415 -> errAudio; else -> errReach }
                        step = "login"
                    }
                }
            }
            TextButton({ step = "found" }) { Text(stringResource(R.string.fr_scan_again)) }
        }
        "testing" -> { Body(stringResource(R.string.fr_testing)); LinearProgressIndicator(Modifier.fillMaxWidth()) }
        "tier" -> {
            val p = probe!!
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(when (p.tier) { "A" -> stringResource(R.string.fr_tier_a); "B" -> stringResource(R.string.fr_tier_b); else -> stringResource(R.string.fr_tier_c) },
                        style = MaterialTheme.typography.titleMedium)
                    Text("Tier ${p.tier} · ${p.rate / 1000} kHz ${p.codec} · ${p.kbps} kbps" + (p.noiseDb?.let { " · background $it dB" } ?: ""),
                        style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                }
            }
            BigButton(stringResource(R.string.fr_continue)) { step = "privacy" }
        }
        "privacy" -> {
            Body(if (mic) network.hone.app.sound.SoundSensing.NOTICE else stringResource(R.string.fr_privacy))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(tick, { tick = it }); Text(stringResource(R.string.fr_rules_tick), style = MaterialTheme.typography.bodyMedium)
            }
            BigButton(stringResource(R.string.fr_start), enabled = tick) {
                val want = buildList {
                    add(Manifest.permission.ACCESS_COARSE_LOCATION)
                    if (mic) add(Manifest.permission.RECORD_AUDIO)
                    if (android.os.Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
                }.filter { ctx.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
                if (want.isNotEmpty()) perms.launch(want.toTypedArray())
                if (mic) {
                    network.hone.app.sound.SoundSensing.acceptNoticeAndEnable(ctx)
                    network.hone.app.service.NodeService.start(ctx)
                } else {
                    val p = probe!!
                    val seat = network.hone.app.telemetry.Telemetry.seatPub(ctx)
                    val st = Station(StationSetup.stationId(p.idSource, p.idValue, seat), p.model ?: "Camera", p.host, p.port, p.path,
                        pick?.onvif, p.codec, p.rate, p.tier, stationCell(ctx), p.idSource, true, System.currentTimeMillis())
                    StationStore.saveCreds(ctx, st.id, user, pass); pass = ""
                    StationStore.save(ctx, st)
                    StationService.start(ctx)
                }
                network.hone.app.engage.Engagement.scheduleDigest(ctx)
                step = "listening"
            }
        }
        "listening" -> {
            Title(stringResource(R.string.fr_listening))
            if (!mic) Body(stringResource(R.string.fr_runs_when), dim = true)
            BigButton(stringResource(R.string.fr_done), onClick = onDone)
        }
    }
}
