package network.hone.app.ui.screens

import android.content.Intent
import android.net.Uri
import androidx.annotation.OptIn
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.birds.BirdCalls
import network.hone.app.birds.CallResult
import network.hone.app.sound.SoundSensing
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

/**
 * In-app reference call player (xeno-canto, streamed unmodified from xeno-canto's file URL).
 * Sound sensing is paused while it plays so the phone does not detect its own speaker.
 * Listening only: nothing here feeds rewards, scoring, training, anti-gaming or marketing.
 */
@OptIn(UnstableApi::class)
@Composable
fun CallPlayer(latin: String) {
    val ctx = LocalContext.current
    val result by produceState<CallResult?>(null, latin) { value = withContext(Dispatchers.IO) { BirdCalls.fetch(latin) } }
    val recs = (result as? CallResult.Ok)?.recordings.orEmpty()
    var idx by remember(latin) { mutableIntStateOf(0) }
    var playing by remember { mutableStateOf(false) }
    var buffering by remember { mutableStateOf(false) }
    var pos by remember { mutableLongStateOf(0L) }
    var dur by remember { mutableLongStateOf(0L) }
    var err by remember { mutableStateOf<String?>(null) }
    var prepared by remember(latin) { mutableIntStateOf(-1) }
    val sensing by SoundSensing.live.collectAsState()

    val player = remember {
        ExoPlayer.Builder(ctx).build().apply {
            addListener(object : Player.Listener {
                override fun onIsPlayingChanged(isPlaying: Boolean) {
                    playing = isPlaying
                    if (isPlaying) SoundSensing.pauseForPlayback() else if (!playWhenReady) SoundSensing.resumeAfterPlayback()
                }
                override fun onPlaybackStateChanged(state: Int) {
                    buffering = state == Player.STATE_BUFFERING
                    if (state == Player.STATE_READY) dur = duration.coerceAtLeast(0)
                    if (state == Player.STATE_ENDED) { playWhenReady = false; SoundSensing.resumeAfterPlayback() }
                }
                override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                    err = "could not play this recording"; playWhenReady = false; SoundSensing.resumeAfterPlayback()
                }
            })
        }
    }
    DisposableEffect(Unit) { onDispose { player.release(); SoundSensing.resumeAfterPlayback() } }
    LaunchedEffect(playing) { while (playing) { pos = player.currentPosition; delay(250) }; pos = player.currentPosition }

    fun prepare(i: Int) {
        val r = recs[i]
        val item = MediaItem.Builder().setUri(Uri.parse(r.fileUrl)).setCustomCacheKey(BirdCalls.cacheKey(r)).build()
        player.setMediaSource(ProgressiveMediaSource.Factory(BirdCalls.dataSource(ctx)).createMediaSource(item))
        player.prepare(); prepared = i; pos = 0; dur = 0; err = null
    }

    Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Text("Its call", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
        when (val r = result) {
            null -> Text("loading reference calls…", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            is CallResult.Unavailable -> Text(r.reason, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            is CallResult.Ok -> {
                val cur = recs[idx.coerceIn(0, recs.lastIndex)]
                Row(verticalAlignment = Alignment.CenterVertically) {
                    FilledIconButton(onClick = {
                        if (player.isPlaying) { player.playWhenReady = false; SoundSensing.resumeAfterPlayback() }
                        else {
                            SoundSensing.pauseForPlayback()
                            if (prepared != idx) prepare(idx)
                            if (player.playbackState == Player.STATE_ENDED) player.seekTo(0)
                            player.playWhenReady = true
                        }
                    }, modifier = Modifier.semantics { contentDescription = if (playing) "Pause call" else "Play call" }) {
                        if (buffering && player.playWhenReady) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        else Icon(if (playing) Icons.Filled.Pause else Icons.Filled.PlayArrow, null)
                    }
                    Slider(value = if (dur > 0) (pos.toFloat() / dur).coerceIn(0f, 1f) else 0f,
                        onValueChange = { f -> if (dur > 0) { pos = (f * dur).toLong(); player.seekTo(pos) } },
                        enabled = dur > 0, modifier = Modifier.weight(1f).padding(horizontal = 6.dp))
                    Text("${fmt(pos)}/${if (dur > 0) fmt(dur) else cur.length}", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
                    IconButton(enabled = recs.size > 1, onClick = {
                        val wasPlaying = player.playWhenReady
                        idx = (idx + 1) % recs.size; prepare(idx)
                        if (wasPlaying) { SoundSensing.pauseForPlayback(); player.playWhenReady = true }
                    }, modifier = Modifier.semantics { contentDescription = "Next recording" }) { Icon(Icons.Filled.SkipNext, null) }
                }
                Text("${idx + 1} of ${recs.size}" + (if (cur.type.isNotEmpty()) " · ${cur.type}" else "") +
                    (if (cur.quality.isNotEmpty()) " · quality ${cur.quality}" else ""),
                    style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
                Text("${cur.credit} · unmodified", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
                Row {
                    Text("License", style = MaterialTheme.typography.labelSmall, textDecoration = TextDecoration.Underline,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.clickable { open(ctx, cur.licenseUrl) }.padding(end = 12.dp, top = 2.dp, bottom = 2.dp))
                    Text("Recording page (xeno-canto)", style = MaterialTheme.typography.labelSmall, textDecoration = TextDecoration.Underline,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.clickable { open(ctx, cur.pageUrl) }.padding(vertical = 2.dp))
                }
                err?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.error) }
                if (sensing.enabled) Text("Sound sensing pauses while a call plays.",
                    style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            }
        }
    }
}

private fun open(ctx: android.content.Context, url: String) {
    if (url.startsWith("https://")) runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}

private fun fmt(ms: Long): String { val s = ms / 1000; return "%d:%02d".format(s / 60, s % 60) }