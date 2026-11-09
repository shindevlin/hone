package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import network.hone.app.R
import network.hone.app.birds.Birds
import network.hone.app.birds.BirdsSync
import network.hone.app.birds.Energy
import network.hone.app.birds.Places
import network.hone.app.birds.Spots
import network.hone.app.sightings.SightingsEngine
import network.hone.app.ui.theme.*

/** Spots near you and their Keepers; challenge one for 20 energy. */
@Composable
fun SpotsScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var spots by remember { mutableStateOf<List<Spots.Spot>?>(null) }
    var loaded by remember { mutableStateOf(false) }
    var energy by remember { mutableIntStateOf(0) }
    var open by remember { mutableStateOf<Spots.Spot?>(null) }
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(tick) {
        withContext(Dispatchers.IO) {
            val f = SightingsEngine.lastFix()
            spots = f?.let { Spots.near(ctx, it.latitude, it.longitude) }
            energy = Energy.available(ctx)
        }
        loaded = true
    }
    open?.let { s ->
        androidx.activity.compose.BackHandler { open = null }
        SpotDetail(s, energy, onChallenge = {
            scope.launch {
                withContext(Dispatchers.IO) { Spots.challenge(ctx, s.cell) }
                tick++
            }
        }, onBack = { open = null })
        return
    }
    OrnPage {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onBack) { Text("←", color = OrnInkDim, style = MaterialTheme.typography.titleLarge) }
            Text(stringResource(R.string.orn_spots), style = MaterialTheme.typography.headlineMedium, modifier = Modifier.weight(1f))
            Text(stringResource(R.string.orn_energy, energy), style = MaterialTheme.typography.labelLarge, color = OrnChorus, fontFamily = HoneMono)
        }
        Text(stringResource(R.string.orn_spot_rule), style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
        val list = spots
        when {
            !loaded -> Text(stringResource(R.string.bd_around_loading), color = OrnInkFaint)
            list == null -> Text(stringResource(R.string.orn_spots_offline), color = OrnInkFaint)
            list.isEmpty() -> Text(stringResource(R.string.orn_spots_empty), color = OrnInkFaint)
            else -> list.forEach { s -> SpotRow(s) { open = s } }
        }
        Text(stringResource(R.string.orn_event_private), style = MaterialTheme.typography.labelSmall, color = OrnInkFaint)
    }
}

@Composable
private fun SpotRow(s: Spots.Spot, onOpen: () -> Unit) {
    val ctx = LocalContext.current
    val name = remember(s.cell) { Places.nearestName(ctx, s.lat, s.lon) ?: "" }
    Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(18.dp)).clickable(onClick = onOpen).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(stringResource(R.string.orn_spot_near, name), style = MaterialTheme.typography.titleMedium)
        Text(when {
            s.keeperIsMe -> stringResource(R.string.orn_spot_you_keep)
            s.keeperMornings == 0 -> stringResource(R.string.orn_spot_no_keeper)
            else -> stringResource(R.string.orn_spot_keeper, s.keeper ?: stringResource(R.string.orn_spot_a_listener))
        }, style = MaterialTheme.typography.bodyMedium, color = if (s.keeperIsMe) OrnGold else OrnInk)
        Text(stringResource(R.string.orn_spot_kinds_here, s.kinds, s.listeners), style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
    }
}

@Composable
private fun SpotDetail(s: Spots.Spot, energy: Int, onChallenge: () -> Unit, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val name = remember(s.cell) { Places.nearestName(ctx, s.lat, s.lon) ?: "" }
    val mine by produceState(Spots.Mine(0, 0), s.cell) { value = withContext(Dispatchers.IO) { Spots.mine(ctx, s.cell) } }
    val ends = Spots.challengeEnds(ctx, s.cell)
    val daysLeft = ((ends - System.currentTimeMillis()) / 86_400_000L + 1).toInt()
    val here = remember(s.cell) {
        SightingsEngine.lastFix()?.let { f -> f.accuracy <= 50f && network.hone.app.sightings.Cells.cell(f.latitude, f.longitude, 9) == s.cell } ?: false
    }
    OrnPage {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onBack) { Text("←", color = OrnInkDim, style = MaterialTheme.typography.titleLarge) }
            Spacer(Modifier.weight(1f))
        }
        Text(stringResource(R.string.orn_spot_kicker), style = MaterialTheme.typography.labelMedium, color = OrnGold)
        Text(stringResource(R.string.orn_spot_near, name), style = MaterialTheme.typography.headlineMedium)
        Text(when {
            s.keeperIsMe -> stringResource(R.string.orn_spot_you_keep)
            s.keeperMornings == 0 -> stringResource(R.string.orn_spot_no_keeper)
            else -> stringResource(R.string.orn_spot_keeper, s.keeper ?: stringResource(R.string.orn_spot_a_listener))
        }, style = MaterialTheme.typography.titleMedium, color = OrnGold)
        if (s.keeperMornings > 0) Text(stringResource(R.string.orn_spot_mornings, s.keeperMornings, s.keeperKinds),
            style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
        Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(18.dp)).padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.orn_spot_you), style = MaterialTheme.typography.titleMedium)
            Progress(stringResource(R.string.orn_spot_mornings_label), mine.mornings, maxOf(s.keeperMornings, 1))
            Progress(stringResource(R.string.orn_spot_kinds_label), mine.kinds, maxOf(s.kinds, 1))
        }
        when {
            s.keeperIsMe -> {}
            ends > System.currentTimeMillis() -> Text(stringResource(R.string.orn_spot_challenging, daysLeft), color = OrnChorus, style = MaterialTheme.typography.titleMedium)
            energy < Energy.CHALLENGE -> Text(stringResource(R.string.orn_spot_need_energy), color = OrnInkDim)
            !here -> { Text(stringResource(R.string.orn_spot_need_here), color = OrnInkDim, style = MaterialTheme.typography.bodySmall)
                PrimaryButton(stringResource(R.string.orn_spot_challenge), enabled = false) {} }
            else -> PrimaryButton(stringResource(R.string.orn_spot_challenge), onClick = onChallenge)
        }
        Text(stringResource(R.string.orn_enc_live_only), style = MaterialTheme.typography.labelSmall, color = OrnInkFaint)
        Text(stringResource(R.string.orn_spot_privacy), style = MaterialTheme.typography.labelSmall, color = OrnInkFaint)
    }
}

@Composable
private fun Progress(label: String, n: Int, of: Int) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row { Text(label, style = MaterialTheme.typography.bodySmall, color = OrnInkDim, modifier = Modifier.weight(1f))
            Text("$n / $of", style = MaterialTheme.typography.labelLarge, fontFamily = HoneMono) }
        LinearProgressIndicator(progress = { (n.toFloat() / of).coerceIn(0f, 1f) }, modifier = Modifier.fillMaxWidth().height(6.dp),
            color = OrnChorus, trackColor = OrnTwig)
    }
}

/** Today: Focus mode and Who's nearby (energy spends) and the dawn chorus event. */
@Composable
fun EnergySpends(energy: Int, onChanged: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var nearby by remember { mutableStateOf<List<BirdsSync.Around>?>(null) }
    var msg by remember { mutableStateOf<String?>(null) }
    val focusUntil = Energy.focusUntil(ctx)
    val focusOn = System.currentTimeMillis() < focusUntil
    val offline = stringResource(R.string.orn_nearby_offline)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SpendRow(stringResource(R.string.orn_focus_title), if (focusOn) stringResource(R.string.orn_focus_on,
            ((focusUntil - System.currentTimeMillis()) / 60_000L).toInt() + 1) else stringResource(R.string.orn_focus_body),
            Energy.FOCUS, enabled = !focusOn && energy >= Energy.FOCUS) {
            scope.launch { withContext(Dispatchers.IO) { Energy.startFocus(ctx) }; onChanged() }
        }
        SpendRow(stringResource(R.string.orn_nearby_title), stringResource(R.string.orn_nearby_body), Energy.NEARBY,
            enabled = nearby == null && energy >= Energy.NEARBY) {
            scope.launch {
                val r = withContext(Dispatchers.IO) { Energy.nearby(ctx) }
                if (r == null) msg = offline else nearby = r
                onChanged()
            }
        }
        msg?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = OrnInkFaint) }
        nearby?.forEach { a ->
            Text("• " + (Birds.english(ctx, a.latin) ?: a.latin) + " · " + a.status, style = MaterialTheme.typography.bodyMedium)
        }
    }
}

@Composable
private fun SpendRow(title: String, body: String, cost: Int, enabled: Boolean, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().background(OrnBough, RoundedCornerShape(16.dp)).padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(body, style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
        }
        Button(onClick, enabled = enabled, shape = RoundedCornerShape(20.dp),
            colors = ButtonDefaults.buttonColors(containerColor = OrnChorus, contentColor = OrnNight)) {
            Text("$cost", fontFamily = HoneMono)
        }
    }
}

/** Dawn chorus: listening from 5 to 7 in the morning earns double energy. */
@Composable
fun DawnChorusCard() {
    val h = java.time.LocalTime.now().hour
    val live = h in 5..6
    Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(stringResource(if (live) R.string.orn_event_live else R.string.orn_event_next), style = MaterialTheme.typography.labelMedium, color = OrnGold)
        Text(stringResource(R.string.orn_event_dawn, "5–7"), style = MaterialTheme.typography.titleMedium)
        Text(stringResource(R.string.orn_event_private), style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
    }
}
