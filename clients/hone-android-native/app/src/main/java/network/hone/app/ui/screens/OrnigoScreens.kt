package network.hone.app.ui.screens

import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.birds.Birds
import network.hone.app.birds.BirdsSync
import network.hone.app.birds.Places
import network.hone.app.birds.Today
import network.hone.app.birds.TodayStats
import network.hone.app.sightings.SightingsEngine
import network.hone.app.sound.SoundSensing
import network.hone.app.ui.theme.*
import org.json.JSONArray
import org.json.JSONObject

/** The required disclaimer, verbatim. Never edit. */
const val DISCLAIMER = "Human conversation is automatically deleted. The listener only keeps a count of voices it removed. " +
    "No audio is ever saved or sent; only a short bird report every 15 minutes."

@Composable
fun DisclaimerCard(modifier: Modifier = Modifier) {
    Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_disclaimer), modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(18.dp))
        .border(1.dp, OrnTwig, RoundedCornerShape(18.dp)).padding(16.dp),
        style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
}

/** Confidence for people: a word, never a percent. */
fun confidenceWord(pct: Double, unlikelyHere: Boolean = false): String = when {
    unlikelyHere -> "unlikely here"
    pct >= 85 -> "very likely"
    pct >= 70 -> "likely"
    else -> "possibly"
}

// ── Map (Ornigo home) ────────────────────────────────────────────────────────

private data class Pin(val latin: String, val name: String, val lat: Double, val lon: Double, val last: Long, val rare: Boolean, val n: Int)

@Composable
fun OrnigoMapScreen(onListen: (String?) -> Unit, onSpots: () -> Unit = {}) {
    val ctx = LocalContext.current
    val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
    val mapView = remember {
        org.maplibre.android.MapLibre.getInstance(ctx)
        org.maplibre.android.maps.MapView(ctx).apply { onCreate(null) }
    }
    DisposableEffect(lifecycle) {
        val obs = androidx.lifecycle.LifecycleEventObserver { _, e ->
            when (e) {
                androidx.lifecycle.Lifecycle.Event.ON_START -> mapView.onStart()
                androidx.lifecycle.Lifecycle.Event.ON_RESUME -> mapView.onResume()
                androidx.lifecycle.Lifecycle.Event.ON_PAUSE -> mapView.onPause()
                androidx.lifecycle.Lifecycle.Event.ON_STOP -> mapView.onStop()
                else -> {}
            }
        }
        lifecycle.addObserver(obs); mapView.onStart(); mapView.onResume()
        onDispose { lifecycle.removeObserver(obs); mapView.onPause(); mapView.onStop(); mapView.onDestroy() }
    }
    var pins by remember { mutableStateOf<List<Pin>>(emptyList()) }
    var sel by remember { mutableStateOf<Pin?>(null) }
    var town by remember { mutableStateOf<String?>(null) }
    var energy by remember { mutableIntStateOf(0) }

    LaunchedEffect(Unit) {
        val since = System.currentTimeMillis() - 24 * 3600_000L
        val (p, t, e) = withContext(Dispatchers.IO) {
            val heard = Birds.heard(ctx).filter { it.t >= since && it.lat != null && it.lon != null && Birds.counts(ctx, it) }
            val ps = heard.groupBy { (network.hone.app.sightings.Cells.cell(it.lat!!, it.lon!!, 9) ?: "%.3f,%.3f".format(it.lat, it.lon)) + "|" + it.name }
                .map { (_, hs) ->
                    // Pin sits on the block (res-9 cell centre) where the bird was heard, not on a person.
                    val c = network.hone.app.sightings.Cells.cell(hs[0].lat!!, hs[0].lon!!, 9)?.let { network.hone.app.sightings.Cells.center(it) }
                    Pin(hs[0].name, Birds.english(ctx, hs[0].name) ?: hs[0].name, c?.first ?: hs.map { it.lat!! }.average(),
                        c?.second ?: hs.map { it.lon!! }.average(), hs.maxOf { it.t }, hs.any { it.rare }, hs.size)
                }.sortedByDescending { it.last }
            val fix = SightingsEngine.lastFix()
            Triple(ps, fix?.let { Places.nearestName(ctx, it.latitude, it.longitude) }, runCatching { network.hone.app.birds.Energy.available(ctx) }.getOrDefault(0))
        }
        pins = p; town = t; energy = e
        val fix = SightingsEngine.lastFix()
        val community = withContext(Dispatchers.IO) { runCatching { JSONArray(BirdsSync.getJson("/map?res=9")) }.getOrNull() }
        mapView.getMapAsync { map ->
            map.uiSettings.isAttributionEnabled = true
            map.uiSettings.isLogoEnabled = false
            map.uiSettings.isRotateGesturesEnabled = false
            map.setStyle(org.maplibre.android.maps.Style.Builder().fromUri(DARK_STYLE)) { style ->
                val hexes = JSONArray()
                if (community != null) for (i in 0 until community.length()) {
                    val ring = network.hone.app.sightings.Cells.boundary(community.getJSONObject(i).getString("cell")) ?: continue
                    val coords = JSONArray().apply { ring.forEach { (la, lo) -> put(JSONArray().put(lo).put(la)) }; put(JSONArray().put(ring[0].second).put(ring[0].first)) }
                    hexes.put(JSONObject().put("type", "Feature").put("properties", JSONObject())
                        .put("geometry", JSONObject().put("type", "Polygon").put("coordinates", JSONArray().put(coords))))
                }
                style.addSource(org.maplibre.android.style.sources.GeoJsonSource("spots",
                    JSONObject().put("type", "FeatureCollection").put("features", hexes).toString()))
                style.addLayer(org.maplibre.android.style.layers.FillLayer("spots-fill", "spots").withProperties(
                    org.maplibre.android.style.layers.PropertyFactory.fillColor("#B8F06A"),
                    org.maplibre.android.style.layers.PropertyFactory.fillOpacity(0.10f),
                    org.maplibre.android.style.layers.PropertyFactory.fillOutlineColor("#2A3A32")))
                val feats = JSONArray()
                p.forEachIndexed { i, pin ->
                    feats.put(JSONObject().put("type", "Feature")
                        .put("properties", JSONObject().put("i", i).put("name", pin.name).put("rare", pin.rare))
                        .put("geometry", JSONObject().put("type", "Point").put("coordinates", JSONArray().put(pin.lon).put(pin.lat))))
                }
                style.addSource(org.maplibre.android.style.sources.GeoJsonSource("birds",
                    JSONObject().put("type", "FeatureCollection").put("features", feats).toString()))
                val rareExpr = org.maplibre.android.style.expressions.Expression.get("rare")
                style.addLayer(org.maplibre.android.style.layers.CircleLayer("bird-ring", "birds").withProperties(
                    org.maplibre.android.style.layers.PropertyFactory.circleRadius(11f),
                    org.maplibre.android.style.layers.PropertyFactory.circleColor("#141F1A"),
                    org.maplibre.android.style.layers.PropertyFactory.circleStrokeWidth(3f),
                    org.maplibre.android.style.layers.PropertyFactory.circleStrokeColor(
                        org.maplibre.android.style.expressions.Expression.switchCase(rareExpr,
                            org.maplibre.android.style.expressions.Expression.literal("#FFD36B"),
                            org.maplibre.android.style.expressions.Expression.literal("#B8F06A")))))
                style.addLayer(org.maplibre.android.style.layers.SymbolLayer("bird-name", "birds").withProperties(
                    org.maplibre.android.style.layers.PropertyFactory.textField(org.maplibre.android.style.expressions.Expression.get("name")),
                    org.maplibre.android.style.layers.PropertyFactory.textSize(12f),
                    org.maplibre.android.style.layers.PropertyFactory.textColor("#F2F6EE"),
                    org.maplibre.android.style.layers.PropertyFactory.textHaloColor("#0B1310"),
                    org.maplibre.android.style.layers.PropertyFactory.textHaloWidth(1.5f),
                    org.maplibre.android.style.layers.PropertyFactory.textOffset(arrayOf(0f, 1.6f)),
                    org.maplibre.android.style.layers.PropertyFactory.textAllowOverlap(false)))
                if (fix != null) {
                    style.addSource(org.maplibre.android.style.sources.GeoJsonSource("me",
                        JSONObject().put("type", "Point").put("coordinates", JSONArray().put(fix.longitude).put(fix.latitude)).toString()))
                    style.addLayer(org.maplibre.android.style.layers.CircleLayer("me-halo", "me").withProperties(
                        org.maplibre.android.style.layers.PropertyFactory.circleRadius(16f),
                        org.maplibre.android.style.layers.PropertyFactory.circleColor("#7CC4FF"),
                        org.maplibre.android.style.layers.PropertyFactory.circleOpacity(0.18f)))
                    style.addLayer(org.maplibre.android.style.layers.CircleLayer("me-dot", "me").withProperties(
                        org.maplibre.android.style.layers.PropertyFactory.circleRadius(6f),
                        org.maplibre.android.style.layers.PropertyFactory.circleColor("#7CC4FF"),
                        org.maplibre.android.style.layers.PropertyFactory.circleStrokeColor("#F2F6EE"),
                        org.maplibre.android.style.layers.PropertyFactory.circleStrokeWidth(2f)))
                }
                val target = fix?.let { org.maplibre.android.geometry.LatLng(it.latitude, it.longitude) }
                    ?: p.firstOrNull()?.let { org.maplibre.android.geometry.LatLng(it.lat, it.lon) }
                    ?: org.maplibre.android.geometry.LatLng(13.7, -89.2)
                map.moveCamera(org.maplibre.android.camera.CameraUpdateFactory.newLatLngZoom(target, if (fix != null || p.isNotEmpty()) 14.5 else 7.0))
                map.addOnMapClickListener { ll ->
                    val pt = map.projection.toScreenLocation(ll)
                    val f = map.queryRenderedFeatures(android.graphics.RectF(pt.x - 30, pt.y - 30, pt.x + 30, pt.y + 30), "bird-ring", "bird-name").firstOrNull()
                    sel = f?.getNumberProperty("i")?.toInt()?.let { p.getOrNull(it) }
                    true
                }
            }
        }
    }

    Box(Modifier.fillMaxSize()) {
        androidx.compose.ui.viewinterop.AndroidView({ mapView }, Modifier.fillMaxSize())
        Row(Modifier.fillMaxWidth().padding(12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Chip(town?.let { androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_near, it) } ?: androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_your_area))
            Spacer(Modifier.weight(1f))
            Box(Modifier.clickable(onClick = onSpots)) { Chip(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_spots), OrnGold) }
            Chip(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_energy, energy), OrnChorus)
        }
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(12.dp)) {
            val s = sel
            Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp))
                .border(1.dp, if (s?.rare == true) OrnGold else OrnTwig, RoundedCornerShape(22.dp)).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)) {
                if (s != null) {
                    Text(if (s.rare) androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_rare) else androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_nearby), style = MaterialTheme.typography.labelMedium,
                        color = if (s.rare) OrnGold else OrnChorus)
                    Text(s.name, style = MaterialTheme.typography.headlineSmall)
                    Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_heard_ago, DateUtils.getRelativeTimeSpanString(s.last, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString()) +
                        (if (s.n > 1) " " + androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_times_today, s.n) else ""), style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
                } else {
                    Text(if (pins.isEmpty()) androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_empty_title) else pins.map { it.latin }.toSet().size.let { n -> androidx.compose.ui.res.pluralStringResource(network.hone.app.R.plurals.orn_map_kinds, n, n) },
                        style = MaterialTheme.typography.headlineSmall)
                    Text(if (pins.isEmpty()) androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_empty_body) else androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_tap),
                        style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
                }
                Button({ onListen(sel?.latin) }, Modifier.fillMaxWidth().height(56.dp), shape = RoundedCornerShape(28.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = OrnChorus, contentColor = OrnNight)) {
                    Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_go_listen), style = MaterialTheme.typography.titleMedium)
                }
            }
            Spacer(Modifier.height(6.dp))
            Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_map_only_you),
                Modifier.fillMaxWidth().background(OrnNight.copy(alpha = 0.7f), RoundedCornerShape(12.dp)).padding(6.dp),
                style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
        }
    }
}

private const val DARK_STYLE = "https://tiles.openfreemap.org/styles/dark"

@Composable
private fun Chip(text: String, color: androidx.compose.ui.graphics.Color = OrnInk) {
    Text(text, Modifier.background(OrnCanopy.copy(alpha = 0.92f), RoundedCornerShape(50)).border(1.dp, OrnTwig, RoundedCornerShape(50))
        .padding(horizontal = 14.dp, vertical = 8.dp), style = MaterialTheme.typography.labelLarge, color = color)
}

// ── Today ────────────────────────────────────────────────────────────────────

@Composable
fun TodayScreen(onSettings: () -> Unit, onSave: () -> Unit = {}) {
    val ctx = LocalContext.current
    val live by SoundSensing.live.collectAsStateWithLifecycle()
    val st by produceState<TodayStats?>(null, live.reportsStored, live.recent.firstOrNull()?.atMs) {
        value = withContext(Dispatchers.IO) { runCatching { Today.compute(ctx) }.getOrNull() }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_today), style = MaterialTheme.typography.headlineMedium, modifier = Modifier.weight(1f))
            TextButton(onSettings) { Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_settings), color = OrnInkDim) }
        }
        SaveYourBirdsCard(onSave)
        val s = st ?: run { Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_counting), color = OrnInkFaint); return@Column }
        // Streak
        Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp)).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.Bottom) {
                Text("${s.streak}", style = MaterialTheme.typography.displayLarge, color = OrnGold)
                Spacer(Modifier.width(8.dp))
                Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_day_streak), style = MaterialTheme.typography.titleMedium,
                    color = OrnInkDim, modifier = Modifier.padding(bottom = 8.dp))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                s.week.forEach { c ->
                    val (bg, fg) = when (c) { 'y' -> OrnGold to OrnNight; 'f' -> OrnSky to OrnNight; 't' -> OrnBough to OrnInk; else -> OrnBough to OrnInkFaint }
                    Box(Modifier.size(34.dp).clip(CircleShape).background(bg)
                        .then(if (c == 't') Modifier.border(2.dp, OrnGold, CircleShape) else Modifier), contentAlignment = Alignment.Center) {
                        Text(when (c) { 'y' -> "✓"; 'f' -> "❄"; 't' -> "·"; else -> "" }, color = fg, style = MaterialTheme.typography.labelLarge)
                    }
                }
            }
            Text(if (s.freezeLeftThisWeek) androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_streak_note_free) else androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_streak_note_used),
                style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
        }
        // Numbers
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Stat("${s.birdsToday}", androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_kinds_today), Modifier.weight(1f))
            Stat("${s.newForYou}", androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_new_for_you), Modifier.weight(1f))
            Stat("${s.cameBack}", androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_came_back), Modifier.weight(1f))
        }
        // Goals
        Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_daily_goals), style = MaterialTheme.typography.titleLarge)
        s.goals.forEach { g ->
            Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(16.dp))
                .border(1.dp, if (g.done) OrnChorus else OrnTwig, RoundedCornerShape(16.dp)).padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text((if (g.done) "✓ " else "") + androidx.compose.ui.res.stringResource(g.titleRes), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                    Text("+${g.energy}", style = MaterialTheme.typography.labelLarge, color = OrnChorus, fontFamily = HoneMono)
                }
                LinearProgressIndicator(progress = { g.progress.toFloat() / g.target }, Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)),
                    color = OrnChorus, trackColor = OrnBough)
                Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_of, g.progress, g.target), style = MaterialTheme.typography.labelSmall, color = OrnInkDim, fontFamily = HoneMono)
            }
        }
        // Energy (earned minus spent)
        var spentTick by remember { mutableIntStateOf(0) }
        val avail = remember(s, spentTick) { (s.energy - network.hone.app.birds.Energy.spent(ctx)).coerceAtLeast(0) }
        Column(Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(22.dp)).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_energy_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                Text("$avail", style = MaterialTheme.typography.headlineMedium, color = OrnChorus, fontFamily = HoneMono)
            }
            Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.orn_energy_body), style = MaterialTheme.typography.bodySmall, color = OrnInkDim)
            EnergySpends(avail) { spentTick++ }
        }
        DawnChorusCard()
        DisclaimerCard()
    }
}

@Composable
private fun Stat(n: String, label: String, modifier: Modifier) {
    Column(modifier.background(OrnCanopy, RoundedCornerShape(18.dp)).padding(14.dp)) {
        Text(n, style = MaterialTheme.typography.headlineMedium, color = OrnChorus, fontFamily = HoneMono)
        Text(label, style = MaterialTheme.typography.labelSmall, color = OrnInkDim)
    }
}
