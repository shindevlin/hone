package network.hone.app.ui.screens

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import network.hone.app.service.NodeService
import network.hone.app.sightings.SightingsEngine
import network.hone.app.sound.Perch
import network.hone.app.sound.SoundSensing
import network.hone.app.ui.theme.*
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

private val CHANNELS = listOf(
    "accel" to "Accelerometer", "gyro" to "Gyroscope", "mag" to "Magnetometer", "baro" to "Barometer",
    "light" to "Light", "proximity" to "Proximity", "steps" to "Steps", "sound" to "Sound (bird + noise reports)",
)

/** Live settings: node connection, what this phone submits, sound, birds privacy, about. */
@Composable
fun SettingsScreen() {
    val ctx = LocalContext.current
    val file = remember { File(ctx.filesDir, "hone-node/sensor_settings.json") }
    val cur = remember { runCatching { JSONObject(file.readText()) }.getOrDefault(JSONObject()) }
    var nodeUrl by remember { mutableStateOf(cur.optString("node_url", "")) }
    var owner by remember { mutableStateOf(cur.optString("owner", "")) }
    var prefix by remember { mutableStateOf(cur.optString("sensor_id_prefix", android.os.Build.MODEL.lowercase().replace(' ', '-'))) }
    val chans = remember {
        val a = cur.optJSONArray("channels"); mutableStateListOf<String>().apply {
            if (a != null) for (i in 0 until a.length()) add(a.getString(i))
        }
    }
    var saved by remember { mutableStateOf("") }

    val prefs = remember { ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE) }
    var showName by remember { mutableStateOf(prefs.getBoolean("show_name", false)) }
    var homeRadius by remember { mutableFloatStateOf(prefs.getFloat("home_radius_m", 500f)) }
    var homeSet by remember { mutableStateOf(prefs.contains("home_lat")) }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 24.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text("Settings", style = MaterialTheme.typography.headlineMedium, color = MaterialTheme.colorScheme.primary)

        Section("Node") {
            OutlinedTextField(nodeUrl, { nodeUrl = it.trim() }, label = { Text("Node URL") }, singleLine = true,
                modifier = Modifier.fillMaxWidth())
            OutlinedTextField(owner, { owner = it.trim() }, label = { Text("Owner account") }, singleLine = true,
                modifier = Modifier.fillMaxWidth())
            OutlinedTextField(prefix, { prefix = it.trim() }, label = { Text("Sensor ID prefix") }, singleLine = true,
                modifier = Modifier.fillMaxWidth())
        }

        Section("Submit to chain") {
            Text("Each checked sensor commits signed readings every 30 s. Location is never submitted.",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            CHANNELS.forEach { (id, label) ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(id in chans, { on -> if (on) chans.add(id) else chans.remove(id) })
                    Text(label, style = MaterialTheme.typography.bodyMedium)
                }
            }
            Button(onClick = {
                saved = if (nodeUrl.isBlank() || owner.isBlank() || prefix.isBlank()) "Node URL, owner and prefix are required."
                else runCatching {
                    file.parentFile?.mkdirs()
                    file.writeText(JSONObject()
                        .put("chain_id", cur.optString("chain_id", "hone"))
                        .put("owner", owner).put("node_url", nodeUrl).put("sensor_id_prefix", prefix)
                        .put("sensor_type", cur.optString("sensor_type", "continuous"))
                        .put("channels", JSONArray(chans.toList())).toString(2))
                    NodeService.stop(ctx); NodeService.start(ctx)
                    "Saved. Node restarted."
                }.getOrElse { "Save failed: ${it.message}" }
            }) { Text("Save and restart node") }
            if (saved.isNotEmpty()) Text(saved, style = MaterialTheme.typography.labelSmall, color = HoneGreen)
            val keys = File(ctx.filesDir, "hone-node/sensor_pubkeys.json")
            if (keys.exists()) Text("Device keys to enroll: " + runCatching { JSONObject(keys.readText()).length() }.getOrDefault(0),
                style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
        }

        SoundCard()

        Section("Birds privacy") {
            Text(
                if (network.hone.app.birds.BirdsSync.canSign(ctx)) "Posting key imported: sightings and photos are signed by your account."
                else "Import your posting key (Sensors > Find my things > keys) to publish sightings and photos.",
                style = MaterialTheme.typography.labelSmall,
                color = if (network.hone.app.birds.BirdsSync.canSign(ctx)) HoneGreen else MaterialTheme.colorScheme.error,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Show my name on public sightings", style = MaterialTheme.typography.bodyMedium)
                    Text("Off: your name is hidden on the public map; HONE still verifies your posting key.",
                        style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                }
                Switch(showName, { showName = it; prefs.edit().putBoolean("show_name", it).apply() })
            }
            Text("Home area: sightings inside it are shown at about 5 km. Elsewhere, as precise as your GPS.",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            Text("Radius ${homeRadius.toInt()} m", style = MaterialTheme.typography.bodySmall)
            Slider(homeRadius, { homeRadius = it }, valueRange = 100f..5000f,
                onValueChangeFinished = { prefs.edit().putFloat("home_radius_m", homeRadius).apply() })
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = {
                    SightingsEngine.lastFix()?.let {
                        prefs.edit().putFloat("home_lat", it.latitude.toFloat()).putFloat("home_lon", it.longitude.toFloat()).apply()
                        homeSet = true
                    }
                }) { Text(if (homeSet) "Move home here" else "Set home here") }
                if (homeSet) TextButton(onClick = {
                    prefs.edit().remove("home_lat").remove("home_lon").apply(); homeSet = false
                }) { Text("Clear") }
            }
            if (!homeSet) Text("No home set. Needs a location fix (Sensor node on).",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }

        Section("About") {
            val pi = remember { ctx.packageManager.getPackageInfo(ctx.packageName, 0) }
            Text("HONE ${pi.versionName} · ${ctx.packageName}", style = MaterialTheme.typography.bodySmall)
            Text("Species model: ${Perch.MODEL_ID}", style = MaterialTheme.typography.bodySmall)
            Text("sha256 ${Perch.MODEL_SHA256.take(16)}…", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            Text("Perch v2 © Google, Apache-2.0. English names: Wikidata (CC0). Species photos: Wikimedia Commons, credited per photo.",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            Text("Source: ${SoundSensing.CODE_URL}", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }
    }
}

@Composable
private fun Section(title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        content()
    }
}
