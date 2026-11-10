package network.hone.app.ui.screens

import android.content.Context
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.birds.Places
import network.hone.app.data.FlipperLog
import network.hone.app.data.HoneAnnounceScanner
import network.hone.app.data.HoneDevice
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import network.hone.app.telemetry.FlipperPublisher
import network.hone.app.ui.theme.*
import org.json.JSONArray
import java.net.URL
import java.util.Calendar

/** One Flipper as the app knows it: live from the BLE scanner, else from its remembered announce. */
data class FlipperInfo(
    val id: String, val owner: String, val battery: Int, val extPower: Boolean, val paused: Boolean,
    val caps: List<String>, val lastSeenMs: Long, val rssi: Int, val relayed: Int, val queued: Int,
) {
    /** Heard by this phone in the last 2 minutes. */
    val connected: Boolean get() = System.currentTimeMillis() - lastSeenMs < 120_000
}

object Flippers {
    fun list(ctx: Context, live: Map<String, HoneDevice>): List<FlipperInfo> {
        val known = FlipperLog.known(ctx).associateBy { it.id }
        val ids = (known.keys + live.values.filter { it.signatureOk }.map { it.deviceId }).distinct()
        return ids.mapNotNull { id ->
            val d = live[id]; val k = known[id]
            when {
                d != null && d.signatureOk -> FlipperInfo(id, d.owner, d.batteryPct, d.extPower, d.paused, d.caps,
                    maxOf(d.lastSeenMs, k?.lastSeenMs ?: 0), d.rssi, d.relayed, d.queued)
                k != null -> FlipperInfo(id, k.owner, k.battery, k.extPower, k.paused, k.caps, k.lastSeenMs, k.rssi, 0, 0)
                else -> null
            }
        }.sortedByDescending { it.lastSeenMs }
    }
}

internal fun flAgo(ms: Long): String {
    if (ms <= 0) return "never"
    val s = ((System.currentTimeMillis() - ms) / 1000).coerceAtLeast(0)
    return when { s < 60 -> "$s s ago"; s < 3600 -> "${s / 60} min ago"; s < 48 * 3600 -> "${s / 3600} h ago"; else -> "${s / 86400} days ago" }
}

private fun FlipperInfo.batteryText() =
    if (battery in 0..100) "battery $battery%" + (if (extPower) ", charging" else "") else ""

private fun FlipperInfo.statusLine() =
    listOf("Seen ${flAgo(lastSeenMs)}", batteryText(), if (paused) "paused" else "").filter { it.isNotEmpty() }.joinToString(" · ")

/**
 * "Your devices": each Flipper with live status; tap one to see what it is seeing.
 * With [showEmpty] false the card is hidden until a Flipper has been heard (Ornigo Today).
 */
@Composable
fun FlipperDevicesCard(onOpen: () -> Unit, showEmpty: Boolean = true, title: String? = null) {
    val ctx = LocalContext.current
    val live by HoneAnnounceScanner.devices.collectAsStateWithLifecycle()
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) { while (true) { delay(15_000); tick++ } }
    val list by produceState(emptyList<FlipperInfo>(), live, tick) { value = withContext(Dispatchers.IO) { Flippers.list(ctx, live) } }
    if (list.isEmpty() && !showEmpty) return
    val me = remember { runCatching { network.hone.app.telemetry.Telemetry.account(ctx) }.getOrNull() }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        title?.let { Text(it.uppercase(), fontSize = 12.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.sp, color = HoneTextDim) }
        if (list.isEmpty()) HoneBox(onClick = onOpen) {
            Text("Flipper", fontWeight = FontWeight.Bold, fontSize = 15.sp, color = HoneText)
            Dim("No Flipper heard yet. Keep Bluetooth on and bring a HONE Flipper close; it shows up here by itself.")
        }
        list.forEach { f ->
            HoneBox(border = if (f.connected) HoneGreen else HoneOutline, onClick = onOpen) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Dot(if (f.connected) HoneGreen else HoneAmber); Spacer(Modifier.width(10.dp))
                    Text(if (f.owner.isNotEmpty() && f.owner != me) "Nearby Flipper (owner ${f.owner})" else "Flipper ${f.id.take(6)}", fontWeight = FontWeight.Bold, fontSize = 15.sp, color = HoneText, modifier = Modifier.weight(1f))
                    Text(if (f.connected) "Connected" else "Not nearby", fontSize = 12.sp, fontWeight = FontWeight.Bold,
                        color = if (f.connected) HoneGreen else HoneAmber)
                    Icon(Icons.Filled.ChevronRight, contentDescription = "Open", tint = HoneTextDim)
                }
                Dim(f.statusLine())
                Dim(if (f.connected) "This phone is relaying it" + (if (f.relayed > 0) " · ${f.relayed} batches sent" else "") + ". Tap to see what it sees."
                    else "Not heard by this phone lately. Another HONE phone may be carrying it.", color = HoneTextFaint)
            }
        }
    }
}

/** What the Flipper is seeing, in plain English. Counts and scrambled ids only. */
@Composable
fun FlipperScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val live by HoneAnnounceScanner.devices.collectAsStateWithLifecycle()
    val cov by SightingsEngine.live.collectAsStateWithLifecycle()
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) { while (true) { delay(15_000); tick++ } }
    var range by remember { mutableIntStateOf(1) }   // 0 = last hour, 1 = today
    val flippers by produceState(emptyList<FlipperInfo>(), live, tick) { value = withContext(Dispatchers.IO) { Flippers.list(ctx, live) } }
    val dayStart = remember(tick / 40) {
        Calendar.getInstance().apply { set(Calendar.HOUR_OF_DAY, 0); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0) }.timeInMillis
    }
    val week by produceState(emptyList<FlipperLog.Entry>(), tick) {
        value = withContext(Dispatchers.IO) { FlipperLog.read(ctx, System.currentTimeMillis() - 7 * 86_400_000L) }
    }
    val f = flippers.firstOrNull()
    val from = if (range == 0) System.currentTimeMillis() - 3_600_000L else dayStart
    val shown = week.filter { it.t >= from && (f == null || it.dev == f.id) }
    // Public index (counts only): what reached HONE today and how much was confirmed as this Flipper's.
    var index by remember { mutableStateOf<Triple<Int, Int, Long>?>(null) }   // batches, confirmed, newest t_min
    LaunchedEffect(f?.owner, tick / 8) {
        val fl = f ?: return@LaunchedEffect
        if (fl.owner.isEmpty()) return@LaunchedEffect
        index = withContext(Dispatchers.IO) {
            runCatching {
                val a = JSONArray(URL("https://honemesh.net/api/flipper/frames?owner=${fl.owner}&since=${dayStart / 60_000}&limit=2000").readText())
                val rows = (0 until a.length()).map { a.getJSONObject(it) }.filter { it.optString("sensor_id").contains(fl.id) }
                Triple(rows.size, rows.count { it.optBoolean("chain_ok", false) }, rows.maxOfOrNull { it.optLong("t_min") } ?: 0L)
            }.getOrNull()
        } ?: index
    }
    val pendingSend = remember(tick) { runCatching { FlipperPublisher.pending(ctx) }.getOrDefault(0) }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle(f?.let { "Flipper ${it.id.take(6)}" } ?: "Flipper", Modifier.weight(1f))
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        Dim("Your Flipper is a pocket radio sensor. It notices Wi-Fi, radio, infrared and tags around it and sends HONE " +
            "counts through your phone, never what it heard. Here is what it noticed and why that helps.")
        if (f == null) {
            HoneBox { Dim("No Flipper heard yet. Turn on Bluetooth and bring a HONE Flipper close to this phone.") }
            return@Column
        }
        // Status
        HoneBox(border = if (f.connected) HoneGreen else HoneOutline) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Dot(if (f.connected) HoneGreen else HoneAmber); Spacer(Modifier.width(10.dp))
                Text(if (f.connected) "Connected to this phone" else "Not near this phone", fontWeight = FontWeight.Bold, color = HoneText,
                    modifier = Modifier.weight(1f))
            }
            Dim(f.statusLine())
            val remoteMs = (index?.third ?: 0L) * 60_000L
            Dim(when {
                f.connected -> "Relayed by this phone." + if (f.queued > 0) " ${f.queued} waiting for a connection." else ""
                remoteMs > f.lastSeenMs + 120_000 -> "Another HONE phone relayed it ${flAgo(remoteMs)}."
                else -> "Nobody has relayed it since. It keeps its readings and sends them when a HONE phone is near."
            })
            Tech("id ${f.id} · owner ${f.owner.ifEmpty { "?" }} · ${f.caps.joinToString(" ")} · ${f.rssi} dBm")
        }
        Segmented(listOf("Last hour", "Today"), range) { range = it }
        // Timeline
        val buckets = if (range == 0) 12 else 24
        val span = if (range == 0) 300_000L else 3_600_000L
        val counts = IntArray(buckets).also { b ->
            shown.forEach { e -> val i = ((e.t - from) / span).toInt(); if (i in 0 until buckets) b[i] += e.n }
        }
        HoneBox {
            Text("Activity", fontWeight = FontWeight.Bold, color = HoneText)
            val total = shown.sumOf { it.n }
            Dim(if (total == 0) "Nothing yet in this time. It sends a batch every few minutes while it is near a phone."
                else "$total readings in ${shown.size} batches " + (if (range == 0) "in the last hour." else "today."))
            Bars(counts, if (range == 0) "Readings per 5 minutes, last hour" else "Readings per hour, today")
            Row { Tech(if (range == 0) "1 hour ago" else "midnight"); Spacer(Modifier.weight(1f)); Tech("now") }
        }
        // What it noticed
        fun of(g: String) = shown.filter { it.group == g }
        SenseCard(Icons.Filled.Wifi, "Wi-Fi networks", of("wifi"), buildString {
            if (cov.running && cov.apCount > 0) {
                append("${cov.apCount} network${if (cov.apCount == 1) "" else "s"} around you now")
                cov.apStrongest?.let { append(", strongest $it dBm") }
                val busy = cov.apChannels.entries.sortedByDescending { it.value }.take(3)
                if (busy.isNotEmpty()) append(". Busiest channels: " + busy.joinToString(", ") { "${it.key} (${it.value})" })
                append(".")
            }
        }, "Counts Wi-Fi networks to map signal coverage and spot outages when a network disappears. Names and addresses are never kept.")
        SenseCard(Icons.Filled.Groups, "Devices around", emptyList(),
            if (cov.running) "About ${cov.bleCrowd} Bluetooth devices nearby in the last few minutes" +
                (if (cov.mobileAps > 0) ", ${cov.mobileAps} phone hotspots" else "") + "."
            else "Start the helper to count devices around you.",
            "Counts how busy a place is without identifying anyone. Each device becomes a number that changes every day.")
        SenseCard(Icons.Filled.Radio, "Radio activity", of("subghz"), "",
            "Garage remotes, weather stations and meters talk on these bands. How busy they are helps map interference. Only the amount is shared, never the message.")
        SenseCard(Icons.Filled.SettingsRemote, "Infrared", of("ir"), "",
            "Remote controls for TVs and air conditioners. Shows a place is in use without seeing anyone.")
        val tags = of("tags")
        SenseCard(Icons.Filled.Nfc, "Tags and cards (NFC, RFID)", tags,
            tags.takeLast(3).reversed().joinToString(" · ") { "#" + it.id.take(6) }.let { if (it.isEmpty()) "" else "Scrambled ids: $it" },
            "Tags it touched or read. Kept only as counts and a scrambled id, never the card number.")
        // Places
        val places by produceState(emptyList<Pair<String, Int>>(), shown) { value = withContext(Dispatchers.IO) { placesOf(ctx, shown) } }
        HoneBox {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Filled.Place, null, tint = HoneOrange); Spacer(Modifier.width(8.dp))
                Text("Places it covered", fontWeight = FontWeight.Bold, color = HoneText, modifier = Modifier.weight(1f))
                Mono("${places.size}", 15)
            }
            if (places.isEmpty()) Dim("No places yet in this time (the phone needs a location fix).")
            places.take(5).forEach { (label, n) -> Row { Dim(label, Modifier.weight(1f)); Dim("$n batches") } }
            Dim("Only you see these places. On the public map others only see a rough area of a few kilometres.", color = HoneTextFaint)
        }
        // What it earns
        HoneBox {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Filled.Paid, null, tint = HoneOrange); Spacer(Modifier.width(8.dp))
                Text("What it earns", fontWeight = FontWeight.Bold, color = HoneText)
            }
            Dim("HONE pays for new or changed information, like a new place or a change at a place it knows. Repeats are not paid.")
            val idx = index
            if (idx == null) Dim("Asking HONE what arrived today…")
            else {
                SplitBar(0f, idx.first.toFloat(), label = "${idx.first} batches waiting to be checked")
                Dim("Today ${idx.first} batches reached HONE, ${idx.second} confirmed as from your Flipper." +
                    (if (pendingSend > 0) " $pendingSend more on this phone, waiting to send." else ""))
                Dim("Earned so far: nothing yet. Flipper rewards are not switched on, so its finds wait here and get checked for new information once they are.",
                    color = HoneTextFaint)
            }
        }
    }
}

@Composable
private fun SenseCard(icon: ImageVector, title: String, es: List<FlipperLog.Entry>, extra: String, why: String) {
    HoneBox {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(icon, null, tint = HoneOrange); Spacer(Modifier.width(8.dp))
            Text(title, fontWeight = FontWeight.Bold, color = HoneText, modifier = Modifier.weight(1f))
            if (es.isNotEmpty()) Mono("${es.sumOf { it.n }}", 15)
        }
        if (es.isNotEmpty()) Dim("Flipper: ${es.sumOf { it.n }} readings in ${es.size} batches, last ${flAgo(es.maxOf { it.t })}.")
        if (extra.isNotEmpty()) Dim(extra)
        if (es.isEmpty() && extra.isEmpty()) Dim("Nothing in this time.")
        Dim(why, color = HoneTextFaint)
    }
}

@Composable
private fun Bars(v: IntArray, label: String) {
    val max = (v.maxOrNull() ?: 0).coerceAtLeast(1)
    val bar = HoneOrange; val track = HoneSurfaceHi
    Canvas(Modifier.fillMaxWidth().height(56.dp).semantics { contentDescription = label }) {
        val w = size.width / v.size; val gap = w * 0.2f
        v.forEachIndexed { i, n ->
            drawRect(track, Offset(i * w + gap / 2, size.height - 2f), Size(w - gap, 2f))
            if (n > 0) { val h = size.height * n / max; drawRect(bar, Offset(i * w + gap / 2, size.height - h), Size(w - gap, h)) }
        }
    }
}

/** Distinct private res-9 cells, named on the phone (offline place list). */
private fun placesOf(ctx: Context, es: List<FlipperLog.Entry>): List<Pair<String, Int>> =
    es.filter { it.cell != null }.groupBy { it.cell!! }.map { (c, l) ->
        val ll = runCatching { Cells.center(c) }.getOrNull()
        (ll?.let { runCatching { Places.nearestName(ctx, it.first, it.second) }.getOrNull() } ?: "Place ${c.takeLast(10).take(4)}") to l.size
    }.groupBy({ it.first }, { it.second }).map { it.key to it.value.sum() }.sortedByDescending { it.second }
