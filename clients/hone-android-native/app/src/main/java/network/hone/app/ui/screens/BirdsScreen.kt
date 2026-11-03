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

    open?.let { latin ->
        BackHandler { open = null }
        mine.firstOrNull { it.latin == latin }?.let { SpeciesDetail(it) { open = null } }
        return
    }

    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp).padding(top = 24.dp)) {
        Text("Birds", style = MaterialTheme.typography.headlineMedium, color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.height(4.dp))
        Text(
            if (mine.isEmpty()) "Nothing heard yet." else
                "${mine.size} species · heard ${mine.sumOf { it.count }}×",
            style = MaterialTheme.typography.bodyMedium, color = HoneTextDim,
        )
        Spacer(Modifier.height(12.dp))
        TabRow(selectedTabIndex = tab, containerColor = MaterialTheme.colorScheme.background) {
            listOf("Mine", "Everyone", "Map").forEachIndexed { i, t ->
                Tab(selected = tab == i, onClick = { tab = i }, text = { Text(t) })
            }
        }
        Spacer(Modifier.height(12.dp))
        when (tab) {
            0 -> MineList(mine, enabled = live.enabled) { open = it }
            else -> ComingSoon(tab)
        }
    }
}

@Composable
private fun MineList(mine: List<MySpecies>, enabled: Boolean, onOpen: (String) -> Unit) {
    if (mine.isEmpty()) {
        Text(
            if (enabled) "Listening. Species show up here as soon as they're heard."
            else "Turn on Sound sensing in the Sensors tab to start hearing birds.",
            style = MaterialTheme.typography.bodyMedium, color = HoneTextFaint,
        )
        return
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        items(mine, key = { it.latin }) { s ->
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
                    Text("heard ${s.count}× · last ${ago(s.last)} · best ${s.bestConf.toInt()}%",
                        style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
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
    var confirm by remember { mutableStateOf<Uri?>(null) }
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
                Image(bmp.asImageBitmap(), null, contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxWidth().height(240.dp).clip(RoundedCornerShape(16.dp)))
                img?.let {
                    Text("Photo: ${it.credit} · ${it.license} · Wikimedia Commons",
                        style = MaterialTheme.typography.labelSmall, color = HoneTextFaint,
                        modifier = Modifier.clickable {
                            ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(it.pageUrl)))
                        })
                }
            }
        }
        item {
            Text("Heard ${s.count}× · first ${ago(s.first)} · last ${ago(s.last)} · best ${s.bestConf.toInt()}%",
                style = MaterialTheme.typography.bodyMedium)
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Your photos", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.weight(1f))
                OutlinedButton(onClick = {
                    picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                }) { Text("Add your photo") }
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
        item { Text("History", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold) }
        items(s.history.take(200)) { h ->
            Text("${DateUtils.formatDateTime(ctx, h.t, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_TIME)} · ${h.conf.toInt()}%",
                style = MaterialTheme.typography.bodySmall, color = HoneTextDim)
        }
        item { Spacer(Modifier.height(24.dp)) }
    }

    confirm?.let { uri ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text("Share your photo") },
            text = {
                Text("You confirm you took this photo of ${s.english ?: s.latin}. It will be published " +
                    "on HONE under HONE's license (UFL), credited to you. Location and camera data are removed " +
                    "from the photo before it leaves your phone.", style = MaterialTheme.typography.bodySmall)
            },
            confirmButton = {
                TextButton(onClick = {
                    Birds.addPhoto(ctx, s.latin, uri); photos = Birds.myPhotos(ctx, s.latin); confirm = null
                }) { Text("Share") }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun ComingSoon(tab: Int) {
    Text(
        if (tab == 1) "Everyone's sightings arrive with the HONE indexer: other people's birds, how often each " +
            "species is heard, anonymous unless they choose to show their name."
        else "Local, regional and worldwide maps arrive with the HONE indexer.",
        style = MaterialTheme.typography.bodyMedium, color = HoneTextFaint,
    )
}

private fun ago(t: Long): String =
    DateUtils.getRelativeTimeSpanString(t, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString()
