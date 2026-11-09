package network.hone.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import network.hone.app.R
import network.hone.app.ui.AppMode
import network.hone.app.ui.theme.*

/** The one Ornigo link to HONE (dawn colour, one per screen). */
@Composable
fun HoneLink(onClick: () -> Unit) {
    Text(stringResource(R.string.hone_get_paid), style = MaterialTheme.typography.titleSmall, color = OrnDawn,
        modifier = Modifier.clickable(onClick = onClick).padding(vertical = 8.dp))
}

/** Ornigo -> HONE hand-off: what stays, what's new, then switch this phone to HONE mode. */
@Composable
fun HandoffScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    OrnPage {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onBack) { Text("←", color = OrnInkDim, style = MaterialTheme.typography.titleLarge) }
        }
        Text("ORNIGO → HONE", style = MaterialTheme.typography.labelMedium, color = OrnDawn)
        Text(stringResource(R.string.hone_handoff_title), style = MaterialTheme.typography.headlineMedium)
        Text(stringResource(R.string.hone_handoff_intro), style = MaterialTheme.typography.bodyMedium, color = OrnInkDim)
        listOf(R.string.hone_handoff_same, R.string.hone_handoff_new, R.string.hone_handoff_rewards).forEach {
            Text(stringResource(it), style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.fillMaxWidth().background(OrnCanopy, RoundedCornerShape(16.dp)).padding(14.dp))
        }
        Button({ AppMode.setHone(ctx, true); AppMode.version.intValue++ }, Modifier.fillMaxWidth().height(56.dp),
            shape = RoundedCornerShape(28.dp), colors = ButtonDefaults.buttonColors(containerColor = OrnDawn, contentColor = OrnNight)) {
            Text(stringResource(R.string.hone_handoff_go), style = MaterialTheme.typography.titleMedium)
        }
        Text(stringResource(R.string.hone_handoff_optional), style = MaterialTheme.typography.labelSmall, color = OrnInkFaint)
    }
}
