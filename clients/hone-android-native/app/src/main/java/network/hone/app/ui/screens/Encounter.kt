package network.hone.app.ui.screens

import androidx.compose.animation.core.*
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.R
import network.hone.app.birds.Catches
import network.hone.app.sound.Detection
import network.hone.app.sound.SoundSensing
import network.hone.app.ui.theme.*

/**
 * Encounter: calls of one bird (30%+ each, within 2 minutes) add up as evidence against a prior
 * for this area (Catches.score). At least three calls and enough evidence fill the ring and catch
 * it. Replay check: two calls that are the same sound mean a played recording, no catch.
 */
@Composable
fun EncounterScreen(target: String?, onCollection: () -> Unit, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val live by SoundSensing.live.collectAsStateWithLifecycle()
    var startedAt by remember { mutableLongStateOf(System.currentTimeMillis()) }
    var species by remember { mutableStateOf(target) }
    var calls by remember { mutableStateOf<List<Detection>>(emptyList()) }
    var score by remember { mutableStateOf<Catches.Score?>(null) }
    var prior by remember { mutableStateOf<Catches.Prior?>(null) }
    var state by remember { mutableStateOf(if (SoundSensing.isEnabled(ctx)) "listen" else "off") }
    var timesBefore by remember { mutableIntStateOf(0) }
    var seen by remember { mutableIntStateOf(0) }

    fun finish(outcome: String) {
        Catches.logOutcome(ctx, startedAt, target, species, outcome, seen, score, prior, calls.map { it.conf })
    }
    fun restart() { calls = emptyList(); score = null; seen = 0; startedAt = System.currentTimeMillis(); state = "listen" }

    // Focus: listen to every frame while the encounter is open.
    LaunchedEffect(state) {
        while (state == "listen") { SoundSensing.focusUntilMs = System.currentTimeMillis() + 120_000L; delay(30_000) }
    }
    DisposableEffect(Unit) {
        onDispose { SoundSensing.focusUntilMs = 0L; if (state == "listen") finish("left") }
    }
    LaunchedEffect(state, startedAt) {
        if (state == "listen") { delay(4 * 60_000L); if (state == "listen") { finish("timeout"); state = "timeout" } }
    }
    LaunchedEffect(species) {
        val sp = species ?: return@LaunchedEffect
        prior = withContext(Dispatchers.IO) { Catches.prior(ctx, sp) }
    }

    // Emulator only (screenshots and tests): play detections from files/encounter_demo.txt,
    // lines "latin|conf|delayMs[|same]" ("same" = identical sound, i.e. a played recording).
    var demo by remember { mutableStateOf<List<Detection>>(emptyList()) }
    LaunchedEffect(Unit) {
        val f = java.io.File(ctx.filesDir, "encounter_demo.txt")
        if (!isEmulator() || !f.exists()) return@LaunchedEffect
        val base = FloatArray(64) { kotlin.random.Random.nextFloat() - 0.5f }
        for (l in f.readLines()) {
            val p = l.trim().split("|"); if (p.size < 3) continue
            delay(p[2].toLong())
            val emb = if (p.getOrNull(3) == "same") base.copyOf() else FloatArray(64) { kotlin.random.Random.nextFloat() - 0.5f }
            demo = demo + Detection(p[0], p[1].toFloat(), System.currentTimeMillis(), emb)
        }
    }

    LaunchedEffect(live.focus, demo, prior) {
        if (state != "listen") return@LaunchedEffect
        val now = System.currentTimeMillis()
        val fresh = (live.focus + demo).filter { it.atMs >= startedAt && now - it.atMs <= Catches.WINDOW_MS && it.conf >= Catches.CALL_FLOOR }
        seen = fresh.distinctBy { it.atMs }.size
        val sp = species ?: fresh.groupBy { it.name }.maxByOrNull { (_, v) -> v.sumOf { it.conf.toDouble() } }?.key ?: return@LaunchedEffect
        species = sp
        val pr = prior ?: return@LaunchedEffect
        val mine = fresh.filter { it.name == sp }.distinctBy { it.atMs }.sortedBy { it.atMs }
        if (Catches.looksReplayed(mine)) { calls = emptyList(); score = null; finish("replay"); state = "replay"; return@LaunchedEffect }
        calls = mine
        val s = Catches.score(mine, pr)
        score = s
        if (s.caught) {
            timesBefore = Catches.count(ctx, sp)
            Catches.record(ctx, sp, mine.maxOf { it.conf }, pr.kind == "out")
            finish("caught")
            state = "caught"
        }
    }

    val name = species?.let { network.hone.app.birds.Birds.english(ctx, it) ?: it }
    val unlikely = prior?.kind == "out"
    OrnPage {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onBack) { Text("←", color = OrnInkDim, style = MaterialTheme.typography.titleLarge) }
            Spacer(Modifier.weight(1f))
        }
        val (kicker, title, body) = when (state) {
            "caught" -> Triple(stringResource(R.string.orn_enc_kicker_caught), name ?: "",
                when {
                    unlikely -> stringResource(R.string.orn_enc_caught_unlikely)
                    timesBefore == 0 -> stringResource(R.string.orn_enc_new)
                    else -> stringResource(R.string.orn_enc_again, timesBefore + 1)
                })
            "replay" -> Triple(stringResource(R.string.orn_enc_kicker_listen), name ?: "", stringResource(R.string.orn_enc_replay))
            "timeout" -> Triple(stringResource(R.string.orn_enc_kicker_listen), name ?: stringResource(R.string.orn_enc_title_listen), stringResource(R.string.orn_enc_timeout))
            "off" -> Triple(stringResource(R.string.orn_enc_kicker_listen), stringResource(R.string.orn_enc_title_listen), stringResource(R.string.orn_enc_off))
            else -> Triple(stringResource(R.string.orn_enc_kicker_listen), if (calls.isNotEmpty() && name != null) name else stringResource(R.string.orn_enc_title_listen),
                when {
                    calls.size >= Catches.MIN_CALLS -> stringResource(R.string.orn_enc_body_more)
                    unlikely && calls.isNotEmpty() -> stringResource(R.string.orn_enc_body_unlikely)
                    prior?.status == "Regular here" && calls.isNotEmpty() -> stringResource(R.string.orn_enc_body_regular)
                    else -> stringResource(R.string.orn_enc_body_listen)
                })
        }
        Text(kicker, style = MaterialTheme.typography.labelMedium, color = if (state == "caught") OrnGold else OrnChorus)
        Text(title, style = MaterialTheme.typography.headlineMedium)
        Text(body, style = MaterialTheme.typography.bodyMedium, color = if (state == "replay") OrnAlarm else OrnInkDim)
        Box(Modifier.fillMaxWidth().height(260.dp), contentAlignment = Alignment.Center) {
            CatchRing(score?.fill ?: 0f, state == "caught")
            Text(if (state == "caught") "✓" else pluralStringResource(R.plurals.orn_enc_calls_heard, calls.size, calls.size),
                style = if (state == "caught") MaterialTheme.typography.displayLarge else MaterialTheme.typography.titleMedium,
                color = if (state == "caught") OrnGold else OrnInk)
        }
        when (state) {
            "caught" -> {
                PrimaryButton(stringResource(R.string.orn_enc_see_collection), onClick = onCollection)
                TextButton({ species = target; restart() }, Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.orn_enc_keep), color = OrnInkDim)
                }
            }
            "replay", "timeout" -> PrimaryButton(stringResource(R.string.orn_enc_try_again)) { restart() }
            else -> {}
        }
        Text(stringResource(R.string.orn_enc_live_only), style = MaterialTheme.typography.labelSmall, color = OrnInkFaint)
    }
}

@Composable
private fun CatchRing(fill: Float, caught: Boolean) {
    val sweep by animateFloatAsState(360f * (if (caught) 1f else fill.coerceIn(0f, 1f)), tween(400), label = "sweep")
    val burst by animateFloatAsState(if (caught) 1.12f else 1f, spring(dampingRatio = 0.4f), label = "burst")
    val pulse by rememberInfiniteTransition(label = "pulse").animateFloat(0.92f, 1f, infiniteRepeatable(tween(1200), RepeatMode.Reverse), label = "p")
    Canvas(Modifier.size(220.dp).scale(if (caught) burst else pulse)) {
        val w = 14.dp.toPx()
        val inset = w / 2
        drawArc(OrnBough, 0f, 360f, false, topLeft = Offset(inset, inset), size = Size(size.width - w, size.height - w), style = Stroke(w))
        drawArc(if (caught) OrnGold else OrnChorus, -90f, sweep, false, topLeft = Offset(inset, inset),
            size = Size(size.width - w, size.height - w), style = Stroke(w, cap = StrokeCap.Round))
    }
}

private fun isEmulator() = android.os.Build.FINGERPRINT.contains("generic") || android.os.Build.MODEL.startsWith("sdk_gphone")
