package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import network.hone.app.R
import network.hone.app.account.AccountStore
import network.hone.app.station.StationService
import network.hone.app.station.StationStore
import network.hone.app.station.Stations
import network.hone.app.ui.theme.HoneGreen
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

/** "Listening": camera stations and the phone mic, in plain words. */
@Composable
fun ListenScreen() {
    val ctx = LocalContext.current
    var adding by remember { mutableStateOf(false) }
    var version by remember { mutableIntStateOf(0) }
    if (adding) { Page { AddCameraFlow(onDone = { adding = false; version++ }) }; return }
    val live by Stations.live.collectAsStateWithLifecycle()
    val stations = remember(version) { StationStore.list(ctx) }
    val ready = StationService.conditionsMet(ctx)
    Page {
        Title(stringResource(R.string.st_title))
        if (AccountStore.signedIn(ctx) && !AccountStore.verified(ctx))
            Text(stringResource(R.string.ob_verify_nudge), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
        stations.forEach { s ->
            val l = live[s.id]
            Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp)).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(s.label, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(when (s.tier) { "A" -> stringResource(R.string.fr_tier_a); "B" -> stringResource(R.string.fr_tier_b); else -> stringResource(R.string.fr_tier_c) },
                    style = MaterialTheme.typography.bodyMedium, color = HoneTextDim)
                Text(when { !ready -> stringResource(R.string.st_paused); l?.reachable == false -> stringResource(R.string.st_unreachable)
                    else -> stringResource(R.string.st_running) },
                    style = MaterialTheme.typography.labelLarge, color = if (ready && l?.reachable != false) HoneGreen else HoneTextFaint)
                Text(stringResource(R.string.st_stats, (l?.birds ?: 0).toInt(), (l?.speechDropped ?: 0).toInt()) +
                    " · " + stringResource(R.string.st_checked, (l?.frames ?: 0).toInt()),
                    style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                TextButton({ StationStore.remove(ctx, s.id); version++; if (StationStore.list(ctx).isEmpty()) StationService.stop(ctx) }) {
                    Text(stringResource(R.string.st_remove))
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.st_mic), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
            Text(if (network.hone.app.sound.SoundSensing.isEnabled(ctx)) stringResource(R.string.st_running) else "",
                color = HoneGreen, style = MaterialTheme.typography.labelLarge)
        }
        BigButton(stringResource(R.string.st_add)) { adding = true }
    }
}
