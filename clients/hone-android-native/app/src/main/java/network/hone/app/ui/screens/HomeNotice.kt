package network.hone.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import network.hone.app.birds.Places
import network.hone.app.home.HomeState
import network.hone.app.ui.theme.*

/** One-time notice after home was set automatically. Shown once; Settings can always change it. */
@Composable
fun HomeNotice(onChange: () -> Unit) {
    val ctx = LocalContext.current
    var show by remember { mutableStateOf(false) }
    var place by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        while (true) {
            if (!show && HomeState.noticePending(ctx)) {
                place = withContext(Dispatchers.IO) {
                    HomeState.home(ctx)?.let { runCatching { Places.nearestName(ctx, it.first, it.second) }.getOrNull() }
                }
                show = true
            }
            delay(30_000)
        }
    }
    if (!show) return
    val nights = HomeState.nights(ctx)
    AlertDialog(
        onDismissRequest = {},
        containerColor = HoneSurface,
        title = { Text("Home", fontWeight = FontWeight.Bold, color = HoneText) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("We think this is home. It's kept private and never shared.", color = HoneText)
                val where = listOfNotNull(place?.let { "Near $it" }, if (nights > 0) "based on $nights nights here" else null)
                    .joinToString(", ").replaceFirstChar { it.uppercase() }
                if (where.isNotEmpty()) Text("$where.", color = HoneTextDim)
                Text("Near home, what this phone shares is only shown as a rough area, and nothing from home goes on chain.",
                    style = MaterialTheme.typography.bodySmall, color = HoneTextFaint)
            }
        },
        confirmButton = {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                TextButton({ HomeState.decline(ctx); show = false }) { Text("Not home", color = HoneTextDim) }
                TextButton({ HomeState.noticeSeen(ctx); show = false; onChange() }) { Text("Change", color = HoneOrange) }
                TextButton({ HomeState.noticeSeen(ctx); show = false }, Modifier) { Text("OK", fontWeight = FontWeight.Bold, color = HoneOrange) }
            }
        },
    )
}
