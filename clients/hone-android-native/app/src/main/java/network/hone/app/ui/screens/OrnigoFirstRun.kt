package network.hone.app.ui.screens

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.R
import network.hone.app.account.AccountStore
import network.hone.app.sightings.SightingsEngine
import network.hone.app.sound.SoundSensing
import network.hone.app.station.FoundCamera
import network.hone.app.ui.theme.*

/**
 * Ornigo first run: sample demo, disclaimer card, sources (all or pick) with a 5 km home chip,
 * then the first bird. No account up front; "Save your birds" asks for email after the first bird.
 */
@Composable
fun OrnigoFirstRun(onDone: () -> Unit, onSignIn: () -> Unit) {
    var step by remember { mutableStateOf("sample") }
    when (step) {
        "sample" -> SampleStep(onStart = { step = "disclaimer" }, onSignIn = onSignIn)
        "disclaimer" -> DisclaimerStep { step = "sources" }
        "sources" -> SourcesStep { step = "first" }
        else -> FirstBirdStep(onDone)
    }
}

@Composable
fun OrnPage(content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().background(OrnNight).verticalScroll(rememberScrollState())
        .padding(horizontal = 20.dp, vertical = 24.dp), verticalArrangement = Arrangement.spacedBy(14.dp), content = content)
}

@Composable
fun PrimaryButton(text: String, enabled: Boolean = true, onClick: () -> Unit) {
    Button(onClick, Modifier.fillMaxWidth().height(56.dp), enabled = enabled, shape = RoundedCornerShape(28.dp),
        colors = ButtonDefaults.buttonColors(containerColor = OrnChorus, contentColor = OrnNight)) {
        Text(text, style = MaterialTheme.typography.titleMedium)
    }
}

/** Sample demo: what a morning looks like, before anything is turned on. */
@Composable
private fun SampleStep(onStart: () -> Unit, onSignIn: () -> Unit) {
    val ctx = LocalContext.current
    val sample = listOf("Pitangus sulphuratus", "Turdus grayi", "Columbina talpacoti")
    var shown by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) { while (shown < sample.size) { delay(1400); shown++ } }
    OrnPage {
        Text(stringResource(R.string.orn_brand), style = MaterialTheme.typography.labelLarge, color = OrnChorus)
        Text(stringResource(R.string.orn_fr_headline), style = MaterialTheme.typography.displayLarge)
        Text(stringResource(R.string.orn_fr_sub), style = MaterialTheme.typography.bodyLarge, color = OrnInkDim)
        Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp)).border(1.dp, OrnTwig, RoundedCornerShape(22.dp))
            .padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(stringResource(R.string.orn_fr_sample_label), style = MaterialTheme.typography.labelMedium, color = OrnSky)
            sample.forEachIndexed { i, latin ->
                val a by animateFloatAsState(if (i < shown) 1f else 0.15f, tween(300), label = "a")
                Row(Modifier.alpha(a).fillMaxWidth().border(1.dp, if (i == shown - 1) OrnChorus else OrnTwig, RoundedCornerShape(16.dp))
                    .padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(network.hone.app.birds.Birds.english(ctx, latin) ?: latin, style = MaterialTheme.typography.titleMedium)
                        Text(stringResource(if (i == 0) R.string.orn_regular_here else R.string.orn_heard_recently),
                            style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
                    }
                    if (i == shown - 1) Text(stringResource(R.string.orn_singing_now), style = MaterialTheme.typography.labelMedium, color = OrnChorus)
                }
            }
        }
        PrimaryButton(stringResource(R.string.orn_fr_start), onClick = onStart)
        TextButton(onSignIn, Modifier.fillMaxWidth()) { Text(stringResource(R.string.ob_have_account), color = OrnInkDim) }
    }
}

/** The disclaimer as a card with one button, then the mic (and location for the home area). */
@Composable
private fun DisclaimerStep(onNext: () -> Unit) {
    val ctx = LocalContext.current
    val perms = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { r ->
        if (r[Manifest.permission.RECORD_AUDIO] == true) {
            SoundSensing.acceptNoticeAndEnable(ctx)
            network.hone.app.service.NodeService.start(ctx)
        }
        onNext()
    }
    OrnPage {
        Text(stringResource(R.string.orn_fr_privacy_title), style = MaterialTheme.typography.headlineMedium)
        DisclaimerCard()
        Text(stringResource(R.string.orn_fr_privacy_more), style = MaterialTheme.typography.bodyMedium, color = OrnInkDim)
        PrimaryButton(stringResource(R.string.orn_fr_understand)) {
            perms.launch(buildList {
                add(Manifest.permission.RECORD_AUDIO)
                add(Manifest.permission.ACCESS_COARSE_LOCATION); add(Manifest.permission.ACCESS_FINE_LOCATION)
                if (android.os.Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            }.toTypedArray())
        }
        TextButton(onNext, Modifier.fillMaxWidth()) { Text(stringResource(R.string.orn_fr_not_now), color = OrnInkDim) }
    }
}

/** Sources: this phone plus cameras found on Wi-Fi. "Listen with all" (default) or "Let me pick". */
@Composable
private fun SourcesStep(onNext: () -> Unit) {
    val ctx = LocalContext.current
    var all by remember { mutableStateOf(true) }
    var cams by remember { mutableStateOf<List<FoundCamera>?>(null) }
    var picked by remember { mutableStateOf(setOf<String>()) }
    var phone by remember { mutableStateOf(true) }
    var adding by remember { mutableStateOf<List<FoundCamera>>(emptyList()) }
    var homeSet by remember { mutableStateOf(ctx.getSharedPreferences("hone_birds", 0).contains("home_lat")) }
    LaunchedEffect(Unit) { cams = withContext(Dispatchers.IO) { runCatching { scanLanPublic(ctx) }.getOrDefault(emptyList()) } }
    if (adding.isNotEmpty()) {
        // Each chosen camera goes through the existing camera setup (password sealed on the phone).
        Box(Modifier.fillMaxSize().background(OrnNight).padding(20.dp)) {
            key(adding.size) { AddCameraFlow(onDone = { adding = adding.drop(1); if (adding.isEmpty()) onNext() }) }
        }
        return
    }
    OrnPage {
        Text(stringResource(R.string.orn_fr_sources_title), style = MaterialTheme.typography.headlineMedium)
        Row(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(28.dp)).padding(4.dp)) {
            listOf(true to R.string.orn_fr_all, false to R.string.orn_fr_pick).forEach { (v, s) ->
                Text(stringResource(s), Modifier.weight(1f).background(if (all == v) OrnChorus else OrnCanopy, RoundedCornerShape(24.dp))
                    .clickable { all = v }.padding(vertical = 12.dp),
                    color = if (all == v) OrnNight else OrnInk, style = MaterialTheme.typography.labelLarge,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            }
        }
        SourceRow(stringResource(R.string.orn_this_phone), stringResource(R.string.orn_q_good), if (all) true else phone, !all) { phone = !phone }
        when (val c = cams) {
            null -> Text(stringResource(R.string.orn_fr_looking), color = OrnInkFaint, style = MaterialTheme.typography.bodySmall)
            else -> {
                if (c.isEmpty()) Text(stringResource(R.string.orn_fr_no_cameras), color = OrnInkFaint, style = MaterialTheme.typography.bodySmall)
                c.forEach { cam ->
                    val on = all || cam.host in picked
                    SourceRow(cam.name ?: stringResource(R.string.orn_camera_at, cam.host), stringResource(R.string.orn_q_needs_password), on, !all) {
                        picked = if (cam.host in picked) picked - cam.host else picked + cam.host
                    }
                }
            }
        }
        // Home area chip: shared as a ~5 km area, never as a point.
        Text(stringResource(if (homeSet) R.string.orn_fr_home_set else R.string.orn_fr_home_chip),
            Modifier.background(OrnBough, RoundedCornerShape(50)).border(1.dp, if (homeSet) OrnChorus else OrnTwig, RoundedCornerShape(50))
                .clickable {
                    SightingsEngine.lastFix()?.let {
                        ctx.getSharedPreferences("hone_birds", 0).edit().putFloat("home_lat", it.latitude.toFloat())
                            .putFloat("home_lon", it.longitude.toFloat()).apply(); homeSet = true
                    }
                }.padding(horizontal = 14.dp, vertical = 8.dp),
            style = MaterialTheme.typography.labelLarge, color = if (homeSet) OrnChorus else OrnInk)
        Text(stringResource(R.string.orn_fr_home_note), style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
        PrimaryButton(stringResource(R.string.orn_fr_continue)) {
            if (!phone && !all) SoundSensing.disable(ctx)
            val chosen = (cams ?: emptyList()).filter { all || it.host in picked }
            if (chosen.isEmpty()) onNext() else adding = chosen
        }
    }
}

@Composable
private fun SourceRow(title: String, quality: String, on: Boolean, togglable: Boolean, onToggle: () -> Unit) {
    Row(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(16.dp)).border(1.dp, if (on) OrnSky else OrnTwig, RoundedCornerShape(16.dp))
        .padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(quality, style = MaterialTheme.typography.labelSmall, color = OrnSky)
        }
        if (togglable) Switch(on, { onToggle() }) else Text("✓", color = OrnSky, style = MaterialTheme.typography.titleMedium)
    }
}

/** Waits for the first bird, then hands over to the map. */
@Composable
private fun FirstBirdStep(onDone: () -> Unit) {
    val ctx = LocalContext.current
    val live by SoundSensing.live.collectAsStateWithLifecycle()
    val first = live.recent.firstOrNull()
    OrnPage {
        Text(stringResource(if (first != null) R.string.orn_fr_first_got else R.string.orn_fr_first_wait), style = MaterialTheme.typography.headlineMedium)
        if (first != null) {
            Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp)).border(2.dp, OrnChorus, RoundedCornerShape(22.dp)).padding(18.dp)) {
                Text(stringResource(R.string.orn_singing_now), style = MaterialTheme.typography.labelMedium, color = OrnChorus)
                Text(network.hone.app.birds.Birds.english(ctx, first.name) ?: first.name, style = MaterialTheme.typography.headlineSmall)
                Text(confidenceWord(first.conf * 100.0, network.hone.app.birds.Birds.unlikelyHere(ctx, first.name)), style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
            }
        } else {
            val p by rememberInfiniteTransition(label = "p").animateFloat(0.3f, 1f, infiniteRepeatable(tween(1200), RepeatMode.Reverse), label = "p")
            Text(stringResource(if (live.enabled) R.string.orn_fr_first_listening else R.string.orn_fr_first_off),
                Modifier.alpha(if (live.enabled) p else 1f), style = MaterialTheme.typography.bodyLarge, color = OrnInkDim)
        }
        PrimaryButton(stringResource(R.string.orn_fr_to_map)) { AccountStore.setOnboarded(ctx); onDone() }
    }
}

// ── "Save your birds" (email later, after the first bird) ──────────────────────

@Composable
fun SaveYourBirdsCard(onSave: () -> Unit) {
    val ctx = LocalContext.current
    if (AccountStore.signedIn(ctx)) return
    val live by SoundSensing.live.collectAsStateWithLifecycle()
    var any by remember { mutableStateOf(false) }
    LaunchedEffect(live.recent.size) { any = withContext(Dispatchers.IO) { network.hone.app.birds.Birds.heard(ctx).isNotEmpty() } }
    if (!any) return
    Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp)).border(1.dp, OrnGold, RoundedCornerShape(22.dp))
        .padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.orn_save_title), style = MaterialTheme.typography.titleLarge)
        Text(stringResource(R.string.orn_save_body), style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
        PrimaryButton(stringResource(R.string.orn_save_button), onClick = onSave)
    }
}
