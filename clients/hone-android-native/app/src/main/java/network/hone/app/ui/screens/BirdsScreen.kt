package network.hone.app.ui.screens

import android.content.Intent
import android.net.Uri
import android.text.format.DateUtils
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.FlutterDash
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.birds.Birds
import network.hone.app.birds.BirdsSync
import org.json.JSONArray
import org.json.JSONObject
import network.hone.app.birds.MySpecies
import network.hone.app.birds.SpeciesImage
import network.hone.app.sound.SoundSensing
import network.hone.app.ui.theme.*

/** Birds heard by this phone, the public feed and maps (the last two arrive with the indexer). */
@Composable
fun BirdsScreen() {
    val ctx = LocalContext.current
    val live by SoundSensing.live.collectAsStateWithLifecycle()
    var tab by remember { mutableIntStateOf(0) }
    var open by remember { mutableStateOf<String?>(null) }
    var mine by remember { mutableStateOf<List<MySpecies>>(emptyList()) }
    // Reload when a new detection arrives.
    LaunchedEffect(live.recent.firstOrNull()?.atMs, live.reportsStored) {
        mine = withContext(Dispatchers.IO) { Birds.mySpecies(ctx) }
    }

    LaunchedEffect(Unit) { withContext(Dispatchers.IO) { BirdsSync.syncNow(ctx.applicationContext) } }

    open?.let { latin ->
        BackHandler { open = null }
        val m = mine.firstOrNull { it.latin == latin }
        if (tab == 0 && m != null) SpeciesDetail(m) { open = null } else PublicDetail(latin) { open = null }
        return
    }

    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp)) {
        Text("Birds", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(4.dp))
        Text(
            if (mine.isEmpty()) "Nothing heard yet." else
                "${mine.size} species · ${mine.sumOf { it.count }} times heard",
            style = MaterialTheme.typography.bodyMedium, color = HoneTextDim,
        )
        Spacer(Modifier.height(12.dp))
        TabRow(selectedTabIndex = tab, containerColor = MaterialTheme.colorScheme.background) {
            listOf(androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_tab_mine),
                androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_tab_around),
                androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_tab_everyone),
                androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_tab_map)).forEachIndexed { i, t ->
                Tab(selected = tab == i, onClick = { tab = i }, text = { Text(t) })
            }
        }
        Spacer(Modifier.height(12.dp))
        when (tab) {
            0 -> MineList(mine, enabled = live.enabled) { open = it }
            1 -> AroundHere { open = it }
            2 -> EveryoneList { open = it }
            else -> BirdMap()
        }
    }
}

@Composable
private fun MineList(mine: List<MySpecies>, enabled: Boolean, onOpen: (String) -> Unit) {
    if (mine.isEmpty()) {
        Text(
            if (enabled) "Listening. Species show up here as soon as they're heard."
            else "Turn on listening (Settings > Sound sensing) to start hearing birds.",
            style = MaterialTheme.typography.bodyMedium, color = HoneTextFaint,
        )
        return
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        items(mine.distinctBy { it.latin }, key = { it.latin }) { s ->
            Row(
                Modifier.fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
                    .clickable { onOpen(s.latin) }
                    .padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                SpeciesThumb(s.latin, 64)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Names(s.latin, s.english)
                    Text((if (s.rare) "rare here · " else "") + "heard ${s.count}× · last ${ago(s.last)}",
                        style = MaterialTheme.typography.labelSmall, color = if (s.rare) HoneGreen else HoneTextDim)
                }
            }
        }
    }
}

/** Shared scores for this area: every phone and camera nearby adds to them. */
@Composable
private fun AroundHere(onOpen: (String) -> Unit) {
    val ctx = LocalContext.current
    var rows by remember { mutableStateOf<List<BirdsSync.Around>?>(null) }
    var loaded by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { rows = withContext(Dispatchers.IO) { BirdsSync.around(ctx.applicationContext) }; loaded = true }
    val r = rows
    if (!loaded) { Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_around_loading), color = HoneTextFaint); return }
    if (r == null) { Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_around_offline), color = HoneTextFaint); return }
    if (r.isEmpty()) { Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_around_empty), color = HoneTextFaint); return }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item {
            Text(androidx.compose.ui.res.stringResource(network.hone.app.R.string.bd_around_intro),
                style = MaterialTheme.typography.bodySmall, color = HoneTextDim)
        }
        items(r.distinctBy { it.latin }, key = { it.latin }) { s ->
            Row(
                Modifier.fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
                    .clickable { onOpen(s.latin) }
                    .padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                SpeciesThumb(s.latin, 48)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Names(s.latin, runCatching { Birds.english(ctx, s.latin) }.getOrNull())
                    Text(s.status, style = MaterialTheme.typography.labelMedium,
                        color = if (s.status == "Regular here" || s.status == "Heard recently") HoneGreen else HoneTextDim)
                }
            }
        }
    }
}

@Composable
private fun Names(latin: String, english: String?) {
    Text(english ?: latin, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
    if (english != null) Text(latin, style = MaterialTheme.typography.bodySmall, fontStyle = FontStyle.Italic, color = HoneTextDim)
}

@Composable
private fun SpeciesThumb(latin: String, size: Int) {
    val ctx = LocalContext.current
    val img by produceState<SpeciesImage?>(null, latin) { value = withContext(Dispatchers.IO) { Birds.image(ctx, latin) } }
    val bmp = remember(img) { img?.let { Birds.bitmap(it.file, size * 3) } }
    Box(Modifier.size(size.dp).clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.surfaceVariant),
        contentAlignment = Alignment.Center) {
        if (bmp != null) Image(bmp.asImageBitmap(), null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        else Icon(Icons.Filled.FlutterDash, null, tint = HoneTextFaint)
    }
}

@Composable
private fun SpeciesDetail(s: MySpecies, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val img by produceState<SpeciesImage?>(null, s.latin) { value = withContext(Dispatchers.IO) { Birds.image(ctx, s.latin) } }
    val bmp = remember(img) { img?.let { Birds.bitmap(it.file, 1080) } }
    var photos by remember { mutableStateOf(Birds.myPhotos(ctx, s.latin)) }
    var verdicts by remember { mutableStateOf(Birds.labels(ctx)) }
    var confirm by remember { mutableStateOf<Uri?>(null) }
    var camera by remember { mutableStateOf(false) }
    var shot by remember { mutableStateOf<Pair<android.graphics.Bitmap, JSONObject>?>(null) }
    if (camera) {
        BackHandler { camera = false }
        BirdCamera(onDone = { b, ev -> shot = b to ev; camera = false }, onCancel = { camera = false })
        return
    }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> confirm = uri }

    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item {
            Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) { Icon(Icons.Filled.ArrowBack, "Back") }
                Column { Names(s.latin, s.english) }
            }
        }
        item {
            if (bmp != null) {
                // Whole photo (no crop): box follows the photo's aspect, clamped so very tall
                // portraits stay on screen; Fit letterboxes anything outside the clamp.
                val ratio = (bmp.width.toFloat() / bmp.height.coerceAtLeast(1)).coerceIn(0.75f, 2.5f)
                Image(bmp.asImageBitmap(), null, contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxWidth().aspectRatio(ratio).clip(RoundedCornerShape(16.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant))
                img?.let {
                    Text("Photo: ${it.credit} · ${it.license}" + if (it.pageUrl.contains("wikimedia")) " · Wikimedia Commons" else "",
                        style = MaterialTheme.typography.labelSmall, color = HoneTextFaint,
                        modifier = Modifier.clickable {
                            if (it.pageUrl.isNotEmpty()) ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(it.pageUrl)))
                        })
                }
            }
        }
        item {
            Text("Heard ${s.count}× · first ${ago(s.first)} · last ${ago(s.last)}",
                style = MaterialTheme.typography.bodyMedium)
        }
        item { WhereHeard(s) }
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Your photos", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.weight(1f))
                OutlinedButton(onClick = {
                    picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                }) { Text("From gallery") }
                Spacer(Modifier.width(6.dp))
                Button(onClick = { camera = true }) { Text("Take photo") }
            }
        }
        if (photos.isNotEmpty()) item {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(photos, key = { it.file.path }) { p ->
                    val b = remember(p.file) { Birds.bitmap(p.file, 300) }
                    Column {
                        if (b != null) Image(b.asImageBitmap(), null, contentScale = ContentScale.Crop,
                            modifier = Modifier.size(110.dp).clip(RoundedCornerShape(10.dp)))
                        Text(p.status, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                    }
                }
            }
        }
        item {
            Text("History", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            val (yes, no) = remember(verdicts) { Birds.trainingCounts(ctx) }
            Text("Only answer if you're sure. Not sure? Leave it blank, or play its call to compare. " +
                "Answers train HONE's local model and keep wrong ones off the map. $yes yes · $no no so far.",
                style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }
        item { CallPlayer(s.latin) }
        items(s.history.distinctBy { it.t }.take(200), key = { "h-${it.t}" }) { h ->
            val v = verdicts["${h.t}|${h.name}"]
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("${DateUtils.formatDateTime(ctx, h.t, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_TIME)} · ${confidenceWord(h.conf)}" +
                    (if (h.rare) " · rare here" else ""),
                    style = MaterialTheme.typography.bodySmall, color = HoneTextDim, modifier = Modifier.weight(1f))
                FilterChip(selected = v == true, onClick = {
                    Birds.label(ctx, h, true); verdicts = Birds.labels(ctx)
                }, label = { Text("Yes, sure") }, modifier = Modifier.padding(end = 6.dp))
                FilterChip(selected = v == false, onClick = {
                    Birds.label(ctx, h, false); verdicts = Birds.labels(ctx)
                }, label = { Text("No, sure") })
            }
        }
        item { Spacer(Modifier.height(24.dp)) }
    }

    shot?.let { (bmp, ev) ->
        UflDialog(name = s.english ?: s.latin, onCancel = { shot = null }) { agreedMs ->
            Birds.addPhoto(ctx, s.latin, bmp, agreedMs, ev); photos = Birds.myPhotos(ctx, s.latin); shot = null
            Thread { BirdsSync.syncNow(ctx.applicationContext) }.start()
        }
    }
    confirm?.let { uri ->
        UflDialog(name = s.english ?: s.latin, onCancel = { confirm = null }) { agreedMs ->
            Birds.addPhoto(ctx, s.latin, uri, agreedMs); photos = Birds.myPhotos(ctx, s.latin); confirm = null
            Thread { BirdsSync.syncNow(ctx.applicationContext) }.start()
        }
    }
}

const val UFL_URL = "https://github.com/shindevlin/hone/blob/main/LICENSE"

/** Photo submission: the user must tick the UFL box, then press Agree. */
@Composable
private fun UflDialog(name: String, onCancel: () -> Unit, onAgree: (Long) -> Unit) {
    val ctx = LocalContext.current
    var ticked by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text("Share your photo") },
        text = {
            Column {
                Text("You confirm you took this photo of $name. It will be published on HONE under HONE's " +
                    "license (UFL). Location and camera data are removed before it leaves your phone.",
                    style = MaterialTheme.typography.bodySmall)
                Spacer(Modifier.height(6.dp))
                Text("Read the UFL", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.clickable { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(UFL_URL))) })
                Spacer(Modifier.height(10.dp))
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clickable { ticked = !ticked }) {
                    Checkbox(ticked, { ticked = it })
                    Text("I agree to publish this photo under the HONE license (UFL).", style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = { TextButton(enabled = ticked, onClick = { onAgree(System.currentTimeMillis()) }) { Text("Agree") } },
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } },
    )
}

// ── Everyone ─────────────────────────────────────────────────────────────────

private data class PublicSpecies(val latin: String, val sightings: Int, val cells: Int, val photos: Int, val lastWindow: Long, val rare: Int = 0)

@Composable
private fun EveryoneList(onOpen: (String) -> Unit) {
    val ctx = LocalContext.current
    val list by produceState<List<PublicSpecies>?>(null) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val a = JSONArray(BirdsSync.getJson("/species"))
                (0 until a.length()).map { a.getJSONObject(it) }.map {
                    PublicSpecies(it.getString("latin"), it.getInt("sightings"), it.getInt("cells"),
                        it.optInt("photos"), it.getLong("last_window"), it.optInt("rare"))
                }
            }.getOrDefault(emptyList())
        }
    }
    when {
        list == null -> Text("Loading…", color = HoneTextFaint)
        list!!.isEmpty() -> Text("No public sightings yet. Yours appear here after each 15-minute window.",
            style = MaterialTheme.typography.bodyMedium, color = HoneTextFaint)
        else -> LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            items(list!!.distinctBy { it.latin }, key = { it.latin }) { s ->
                Row(
                    Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
                        .clickable { onOpen(s.latin) }.padding(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    SpeciesThumb(s.latin, 64)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Names(s.latin, Birds.english(ctx, s.latin))
                        Text("${s.sightings} sightings · ${s.cells} places · ${s.photos} photos" +
                            (if (s.rare > 0) " · ${s.rare} rare" else "") + " · last ${ago(s.lastWindow * 900_000L)}",
                            style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
                    }
                }
            }
        }
    }
}

@Composable
private fun PublicDetail(latin: String, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val img by produceState<SpeciesImage?>(null, latin) { value = withContext(Dispatchers.IO) { Birds.image(ctx, latin) } }
    val bmp = remember(img) { img?.let { Birds.bitmap(it.file, 1080) } }
    val data by produceState<JSONObject?>(null, latin) {
        value = withContext(Dispatchers.IO) { runCatching { JSONObject(BirdsSync.getJson("/species/" + Uri.encode(latin))) }.getOrNull() }
    }
    LazyColumn(Modifier.fillMaxSize().padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item {
            Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) { Icon(Icons.Filled.ArrowBack, "Back") }
                Column { Names(latin, Birds.english(ctx, latin)) }
            }
        }
        item {
            if (bmp != null) {
                // Whole photo (no crop): box follows the photo's aspect, clamped so very tall
                // portraits stay on screen; Fit letterboxes anything outside the clamp.
                val ratio = (bmp.width.toFloat() / bmp.height.coerceAtLeast(1)).coerceIn(0.75f, 2.5f)
                Image(bmp.asImageBitmap(), null, contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxWidth().aspectRatio(ratio).clip(RoundedCornerShape(16.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant))
                img?.let { Text("Photo: ${it.credit} · ${it.license}" + if (it.pageUrl.contains("wikimedia")) " · Wikimedia Commons" else "",
                    style = MaterialTheme.typography.labelSmall, color = HoneTextFaint) }
            }
        }
        item { CallPlayer(latin) }
        val photos = data?.optJSONArray("photos")
        if (photos != null && photos.length() > 0) {
            item { Text("Community photos (UFL)", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold) }
            item {
                LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items((0 until photos.length()).map { photos.getJSONObject(it) }, key = { it.getString("id") }) { p ->
                        val b by produceState<android.graphics.Bitmap?>(null, p.getString("id")) {
                            value = withContext(Dispatchers.IO) {
                                runCatching { BirdsSync.cachedBytes(ctx, p.getString("url")).let { android.graphics.BitmapFactory.decodeByteArray(it, 0, it.size) } }.getOrNull()
                            }
                        }
                        Column {
                            b?.let { Image(it.asImageBitmap(), null, contentScale = ContentScale.Crop,
                                modifier = Modifier.size(140.dp).clip(RoundedCornerShape(10.dp))) }
                            Text(p.optString("account").ifEmpty { "anonymous" }, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                        }
                    }
                }
            }
        }
        val sightings = data?.optJSONArray("sightings")
        item { Text("Sightings: ${sightings?.length() ?: 0}", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold) }
        if (sightings != null) items((0 until minOf(sightings.length(), 100)).map { sightings.getJSONObject(it) }) { s ->
            Text("${DateUtils.formatDateTime(ctx, s.getLong("window") * 900_000L, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_TIME)} · " +
                "${(s.getInt("conf_q8") * 100 / 255)}% · ${s.optString("account").ifEmpty { "hidden name" }} · cell res ${s.getInt("res")}" +
                    when (s.optString("rare")) { "unconfirmed" -> " · rare, unconfirmed"; "confirmed" -> " · rare, confirmed"; else -> "" },
                style = MaterialTheme.typography.bodySmall, color = HoneTextDim)
        }
        item { Spacer(Modifier.height(24.dp)) }
    }
}

// ── Map ──────────────────────────────────────────────────────────────────────

@Composable
private fun BirdMap() {
    val ctx = LocalContext.current
    var scale by remember { mutableIntStateOf(0) }
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
    var info by remember { mutableStateOf("") }
    LaunchedEffect(scale) {
        val res = intArrayOf(9, 5, 3)[scale]
        val fc = withContext(Dispatchers.IO) {
            runCatching {
                val a = JSONArray(BirdsSync.getJson("/map?res=$res"))
                val feats = JSONArray()
                for (i in 0 until a.length()) {
                    val c = a.getJSONObject(i)
                    val ring = network.hone.app.sightings.Cells.boundary(c.getString("cell")) ?: continue
                    val coords = JSONArray().apply {
                        ring.forEach { (lat, lng) -> put(JSONArray().put(lng).put(lat)) }
                        put(JSONArray().put(ring[0].second).put(ring[0].first))
                    }
                    feats.put(JSONObject().put("type", "Feature")
                        .put("properties", JSONObject().put("n", c.getInt("sightings")).put("species", c.getInt("species")))
                        .put("geometry", JSONObject().put("type", "Polygon").put("coordinates", JSONArray().put(coords))))
                }
                info = "${a.length()} areas with sightings"
                JSONObject().put("type", "FeatureCollection").put("features", feats).toString()
            }.getOrElse { info = "map data unavailable"; """{"type":"FeatureCollection","features":[]}""" }
        }
        val fix = network.hone.app.sightings.SightingsEngine.lastFix()
        mapView.getMapAsync { map ->
            map.setStyle(org.maplibre.android.maps.Style.Builder().fromUri(MAP_STYLE)) { style ->
                style.getSourceAs<org.maplibre.android.style.sources.GeoJsonSource>("birds")?.setGeoJson(fc) ?: run {
                    style.addSource(org.maplibre.android.style.sources.GeoJsonSource("birds", fc))
                    style.addLayer(org.maplibre.android.style.layers.FillLayer("birds-fill", "birds").withProperties(
                        org.maplibre.android.style.layers.PropertyFactory.fillColor("#B8F06A"),
                        org.maplibre.android.style.layers.PropertyFactory.fillOpacity(0.45f),
                        org.maplibre.android.style.layers.PropertyFactory.fillOutlineColor("#B8F06A"),
                    ))
                }
                val zoom = doubleArrayOf(12.0, 7.0, 1.2)[scale]
                val target = if (fix != null && scale < 2) org.maplibre.android.geometry.LatLng(fix.latitude, fix.longitude)
                    else org.maplibre.android.geometry.LatLng(13.7, -89.2)
                map.moveCamera(org.maplibre.android.camera.CameraUpdateFactory.newLatLngZoom(target, zoom))
            }
        }
    }
    Column(Modifier.fillMaxSize()) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("Local", "Regional", "World").forEachIndexed { i, t ->
                FilterChip(selected = scale == i, onClick = { scale = i }, label = { Text(t) })
            }
        }
        Text(info, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        Spacer(Modifier.height(6.dp))
        androidx.compose.ui.viewinterop.AndroidView({ mapView },
            Modifier.fillMaxWidth().weight(1f).clip(RoundedCornerShape(16.dp)))
        Text("Map: OpenFreeMap · © OpenStreetMap contributors", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
    }
}

private const val MAP_STYLE = "https://tiles.openfreemap.org/styles/dark"

private fun ago(t: Long): String =
    DateUtils.getRelativeTimeSpanString(t, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString()

// ── Where I heard this bird (local log only, never sent) ───────────────────────

@Composable
private fun WhereHeard(s: MySpecies) {
    val ctx = LocalContext.current
    val hp by produceState<network.hone.app.birds.HeardPlaces?>(null, s.latin, s.count) {
        value = withContext(Dispatchers.IO) { network.hone.app.birds.Places.forSpecies(ctx, s.history) }
    }
    val r = hp ?: return
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(if (r.places.size == 1) "Heard in 1 place" else "Heard in ${r.places.size} places",
            style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        if (r.places.isNotEmpty()) PlacesMap(r.places)
        r.places.forEach { p ->
            Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface, RoundedCornerShape(12.dp)).padding(12.dp)) {
                Text(p.label, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                Text("${p.count}× · first ${ago(p.first)} · last ${ago(p.last)} · ${confidenceWord(p.bestConf)}",
                    style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            }
        }
        if (r.unlocated > 0) Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface, RoundedCornerShape(12.dp)).padding(12.dp)) {
            Text("Place not recorded", style = MaterialTheme.typography.bodyMedium, color = HoneTextDim)
            Text("${r.unlocated}× heard without a location fix", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }
        Text("From this phone's own log. Place names: GeoNames, offline. Stays on this phone.",
            style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
    }
}

@Composable
private fun PlacesMap(places: List<network.hone.app.birds.HeardPlace>) {
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
    LaunchedEffect(places) {
        val feats = JSONArray()
        val pins = JSONArray()
        places.forEach { p ->
            p.cells.forEach { c -> val ring = network.hone.app.sightings.Cells.boundary(c)
            if (ring != null) {
                val coords = JSONArray().apply {
                    ring.forEach { (lat, lng) -> put(JSONArray().put(lng).put(lat)) }
                    put(JSONArray().put(ring[0].second).put(ring[0].first))
                }
                feats.put(JSONObject().put("type", "Feature").put("properties", JSONObject().put("n", p.count))
                    .put("geometry", JSONObject().put("type", "Polygon").put("coordinates", JSONArray().put(coords))))
            } }
            pins.put(JSONObject().put("type", "Feature").put("properties", JSONObject().put("n", p.count))
                .put("geometry", JSONObject().put("type", "Point").put("coordinates", JSONArray().put(p.lon).put(p.lat))))
        }
        val hexes = JSONObject().put("type", "FeatureCollection").put("features", feats).toString()
        val points = JSONObject().put("type", "FeatureCollection").put("features", pins).toString()
        mapView.getMapAsync { map ->
            map.uiSettings.setAllGesturesEnabled(false)
            map.uiSettings.isAttributionEnabled = false
            map.uiSettings.isLogoEnabled = false
            map.setStyle(org.maplibre.android.maps.Style.Builder().fromUri(MAP_STYLE)) { style ->
                style.addSource(org.maplibre.android.style.sources.GeoJsonSource("heard-hex", hexes))
                style.addSource(org.maplibre.android.style.sources.GeoJsonSource("heard-pin", points))
                style.addLayer(org.maplibre.android.style.layers.FillLayer("heard-fill", "heard-hex").withProperties(
                    org.maplibre.android.style.layers.PropertyFactory.fillColor("#B8F06A"),
                    org.maplibre.android.style.layers.PropertyFactory.fillOpacity(0.40f),
                    org.maplibre.android.style.layers.PropertyFactory.fillOutlineColor("#B8F06A"),
                ))
                style.addLayer(org.maplibre.android.style.layers.CircleLayer("heard-dot", "heard-pin").withProperties(
                    org.maplibre.android.style.layers.PropertyFactory.circleColor("#B8F06A"),
                    org.maplibre.android.style.layers.PropertyFactory.circleRadius(5f),
                    org.maplibre.android.style.layers.PropertyFactory.circleStrokeColor("#FFFFFF"),
                    org.maplibre.android.style.layers.PropertyFactory.circleStrokeWidth(1.5f),
                ))
                val cam = if (places.size == 1) org.maplibre.android.camera.CameraUpdateFactory.newLatLngZoom(
                    org.maplibre.android.geometry.LatLng(places[0].lat, places[0].lon), 14.0)
                else org.maplibre.android.camera.CameraUpdateFactory.newLatLngBounds(
                    org.maplibre.android.geometry.LatLngBounds.Builder().apply {
                        places.forEach { include(org.maplibre.android.geometry.LatLng(it.lat, it.lon)) }
                    }.build(), 60)
                runCatching { map.moveCamera(cam) }.onFailure {
                    map.moveCamera(org.maplibre.android.camera.CameraUpdateFactory.newLatLngZoom(
                        org.maplibre.android.geometry.LatLng(places[0].lat, places[0].lon), 12.0))
                }
                if (map.cameraPosition.zoom > 15.0) map.moveCamera(org.maplibre.android.camera.CameraUpdateFactory.zoomTo(15.0))
            }
        }
    }
    androidx.compose.ui.viewinterop.AndroidView({ mapView },
        Modifier.fillMaxWidth().height(180.dp).clip(RoundedCornerShape(14.dp)))
    Text("Map: OpenFreeMap · © OpenStreetMap contributors", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
}
