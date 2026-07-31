package network.hone.app.ui.screens

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import network.hone.app.data.SensorCapture
import network.hone.app.data.SensorChannel
import network.hone.app.service.NodeService
import network.hone.app.ui.theme.*

/**
 * The phone's sensor-node role, driven by REAL hardware. Enumerates the device's actual
 * sensors and shows live values. The master switch requests location (for GNSS) and starts
 * the foreground NodeService, which signs and submits readings continuously. A device that
 * lacks a sensor shows it greyed out — no mock values.
 */
@Composable
fun SensorsScreen() {
    val ctx = LocalContext.current
    val capture = remember { SensorCapture(ctx) }
    val channels by capture.channels.collectAsStateWithLifecycle()
    var running by remember { mutableStateOf(false) }

    val permLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted -> if (granted) { capture.start(); NodeService.start(ctx); running = true } }

    // Live preview while the screen is open (the service keeps capturing when it's closed).
    DisposableEffect(Unit) {
        capture.start()
        onDispose { if (!running) capture.stop() }
    }

    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp)) {
        Text("Sensors", style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.height(8.dp))
        Text("Contribute verified readings to earn.", style = MaterialTheme.typography.bodyMedium,
            color = HoneTextDim)
        Spacer(Modifier.height(16.dp))

        // Master control: start/stop the background sensor node.
        Row(
            Modifier.fillMaxWidth()
                .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Filled.Sensors, contentDescription = null,
                tint = if (running) HoneGreen else HoneTextFaint, modifier = Modifier.size(26.dp))
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text("Sensor node", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(
                    when {
                        running -> "running · read-only capture"
                        else -> "${capture.availableCount()} sensors ready"
                    },
                    style = MaterialTheme.typography.labelSmall,
                    color = if (running) HoneGreen else HoneTextFaint,
                )
            }
            Switch(checked = running, onCheckedChange = { on ->
                if (on) {
                    if (capture.hasLocationPermission()) {
                        capture.start(); NodeService.start(ctx); running = true
                    } else {
                        permLauncher.launch(Manifest.permission.ACCESS_FINE_LOCATION)
                    }
                } else {
                    NodeService.stop(ctx); running = false
                }
            })
        }
        Spacer(Modifier.height(16.dp))

        LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(channels, key = { it.id }) { SensorCard(it) }
        }
    }
}

private fun iconFor(id: String): ImageVector = when (id) {
    "gnss" -> Icons.Filled.LocationOn
    "accel", "gyro" -> Icons.Filled.Vibration
    "mag" -> Icons.Filled.Explore
    "baro" -> Icons.Filled.Speed
    "light" -> Icons.Filled.LightMode
    "proximity" -> Icons.Filled.SocialDistance
    "steps" -> Icons.Filled.DirectionsWalk
    else -> Icons.Filled.Sensors
}

@Composable
private fun SensorCard(ch: SensorChannel) {
    Row(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(iconFor(ch.id), contentDescription = null,
            tint = if (ch.available) MaterialTheme.colorScheme.primary else HoneTextFaint,
            modifier = Modifier.size(24.dp))
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(ch.label, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Medium)
            Text(
                when {
                    !ch.available -> "not on this device"
                    ch.detail.isNotEmpty() -> ch.detail
                    else -> ch.unit
                },
                style = MaterialTheme.typography.labelSmall,
                color = if (ch.available && ch.detail.isNotEmpty()) HoneGreen else HoneTextFaint,
            )
        }
        if (ch.available && ch.samples > 0) {
            Text("${ch.samples}", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
        }
    }
}
