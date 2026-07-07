package network.hone.app.ui.screens

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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import network.hone.app.ui.theme.*

/**
 * Sensors screen — the phone's sensor-node role. Each sensor is a card the user
 * toggles ON to start contributing readings to the chain (button-driven; the
 * permission request is triggered by the toggle, not a separate copy-paste
 * step). Live values shown when active. Wired to the Rust sensor submit
 * (`submitSensorReading`) in Phase 0b; mock values today.
 */
@Composable
fun SensorsScreen() {
    // Mock sensor states — Phase 0b hooks real Capacitor-free native sensors +
    // runtime permission requests, then submitSensorReading() through the bridge.
    var gps by remember { mutableStateOf(false) }
    var motion by remember { mutableStateOf(false) }
    var baro by remember { mutableStateOf(false) }
    var light by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp)) {
        Text("Sensors", style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.height(8.dp))
        Text("Contribute verified readings to earn.", style = MaterialTheme.typography.bodyMedium,
            color = HoneTextDim)
        Spacer(Modifier.height(20.dp))

        LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(
                listOf(
                    SensorSpec("GPS", "location", Icons.Filled.LocationOn, gps, "53.34°N, 6.26°W") { gps = it },
                    SensorSpec("Motion", "accelerometer", Icons.Filled.Vibration, motion, "x 0.02 · y 0.01 · z 9.81") { motion = it },
                    SensorSpec("Barometer", "pressure", Icons.Filled.Speed, baro, "1013.2 hPa") { baro = it },
                    SensorSpec("Light", "illuminance", Icons.Filled.LightMode, light, "342 lux") { light = it },
                ),
                key = { it.title },
            ) { SensorCard(it) }
        }
    }
}

private data class SensorSpec(
    val title: String,
    val type: String,
    val icon: ImageVector,
    val active: Boolean,
    val reading: String,
    val onToggle: (Boolean) -> Unit,
)

@Composable
private fun SensorCard(spec: SensorSpec) {
    Row(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(spec.icon, contentDescription = null,
            tint = if (spec.active) MaterialTheme.colorScheme.primary else HoneTextFaint,
            modifier = Modifier.size(24.dp))
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(spec.title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Medium)
            Text(
                if (spec.active) spec.reading else spec.type,
                style = MaterialTheme.typography.labelSmall,
                color = if (spec.active) HoneGreen else HoneTextFaint,
            )
        }
        Switch(checked = spec.active, onCheckedChange = spec.onToggle)
    }
}
