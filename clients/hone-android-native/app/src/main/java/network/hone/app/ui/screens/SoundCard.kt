package network.hone.app.ui.screens

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import network.hone.app.service.NodeService
import network.hone.app.sound.SoundSensing
import network.hone.app.ui.theme.*

/** Sound sensing: bird check + ambient noise. Off until the user accepts the notice. */
@Composable
fun SoundCard() {
    val ctx = LocalContext.current
    val live by SoundSensing.live.collectAsStateWithLifecycle()
    var enabled by remember { mutableStateOf(SoundSensing.isEnabled(ctx)) }
    var showNotice by remember { mutableStateOf(false) }

    val micPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { r ->
        if (r[Manifest.permission.RECORD_AUDIO] == true) {
            SoundSensing.acceptNoticeAndEnable(ctx)
            NodeService.start(ctx)   // re-enter foreground with the microphone type while visible
            enabled = true
        }
    }

    LaunchedEffect(enabled) { if (enabled) { SoundSensing.start(ctx); NodeService.start(ctx) } }

    Column(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
            .padding(16.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.GraphicEq, contentDescription = null,
                tint = if (enabled) HoneGreen else HoneTextFaint, modifier = Modifier.size(26.dp))
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text("Sound sensing", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(
                    if (enabled) "${live.status} · ${live.framesHeard} clips · ${live.framesHuman} with voices dropped"
                    else "bird check + ambient noise · off",
                    style = MaterialTheme.typography.labelSmall,
                    color = if (enabled) HoneGreen else HoneTextFaint,
                )
            }
            Switch(checked = enabled, onCheckedChange = { on ->
                if (on) showNotice = true else { SoundSensing.disable(ctx); enabled = false }
            })
        }
        if (enabled) {
            Spacer(Modifier.height(10.dp))
            live.lastLeq?.let {
                Text("Noise: ${"%.1f".format(it)} dBFS (uncalibrated)", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            }
            Text("This window: " + if (live.windowSpecies.isEmpty()) "no species yet" else "",
                style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            live.windowSpecies.take(6).forEach {
                Text("  ${it.name} · ${(it.conf * 100).toInt()}%", style = MaterialTheme.typography.bodySmall)
            }
            if (live.reportsStored > 0) {
                Text("${live.reportsStored} window reports kept", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            }
        }
    }

    if (showNotice) {
        var ticked by remember { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { showNotice = false },
            title = { Text("Sound sensing") },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    Text(SoundSensing.NOTICE, style = MaterialTheme.typography.bodySmall)
                    Spacer(Modifier.height(12.dp))
                    Row(verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.clickable { ticked = !ticked }) {
                        Checkbox(checked = ticked, onCheckedChange = { ticked = it })
                        Text("I have checked my local laws and want to turn on Sound sensing.",
                            style = MaterialTheme.typography.bodySmall)
                    }
                }
            },
            confirmButton = {
                TextButton(enabled = ticked, onClick = {
                    showNotice = false
                    micPerm.launch(buildList {
                        add(Manifest.permission.RECORD_AUDIO)
                        if (android.os.Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
                    }.toTypedArray())
                }) { Text("Turn on") }
            },
            dismissButton = { TextButton(onClick = { showNotice = false }) { Text("Cancel") } },
        )
    }
}
