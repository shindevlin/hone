package network.hone.app.ui.screens

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Map
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import network.hone.app.sightings.LiveCoverage
import network.hone.app.sightings.SightingsEngine
import network.hone.app.ui.theme.HoneGreen
import network.hone.app.ui.theme.HoneTextDim
import network.hone.app.ui.theme.HoneTextFaint

/** Live summary of the current 15-min window: what this phone would report for its cell. */
@Composable
fun CoverageCard(c: LiveCoverage, onFindMy: () -> Unit) {
    val ctx = LocalContext.current
    var routerCode by remember { mutableStateOf<String?>(null) }
    val bgLoc = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
    Column(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(16.dp))
            .padding(16.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Map, contentDescription = null,
                tint = if (c.running) HoneGreen else HoneTextFaint, modifier = Modifier.size(24.dp))
            Spacer(Modifier.width(14.dp))
            Text("Coverage · this window", style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            if (c.running) {
                val left = ((c.windowEndsMs - System.currentTimeMillis()) / 60_000).coerceAtLeast(0)
                Text("${left}m left", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
            }
        }
        if (!c.running) {
            Text("Starts with the sensor node.", style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
            return@Column
        }
        Spacer(Modifier.height(6.dp))
        val line = MaterialTheme.typography.labelSmall
        Text("Cell ${c.cell?.let { "$it (r${c.cellRes})" } ?: "unknown (no fix)"}" +
            (c.locAccM?.let { " · ±${it.toInt()} m" } ?: "") + (c.locAgeS?.let { " · ${it}s old" } ?: "") +
            (if (c.locSrc.isNotEmpty()) " · ${c.locSrc}" else "") +
            (if (c.moveState.isNotEmpty()) " · ${c.moveState}" else "") +
            (if (c.fingerprintCells > 0) " · ${c.fingerprintCells} fp cells" else ""),
            style = line, color = if (c.cell != null) HoneGreen else HoneTextFaint)
        val bands = listOf("2g" to "2.4", "5g" to "5", "6g" to "6").joinToString(" · ") { (k, n) ->
            if (k in c.bandsSupported) "$n GHz ${c.apByBand[k] ?: 0}" else "$n GHz n/a"
        }
        Text("Wi-Fi APs ${c.apCount} ($bands)", style = line, color = HoneTextDim)
        Text("BLE crowd ${c.bleCrowd} · HONE devices ${c.honeDevices}" +
            (if (c.mobileAps > 0) " · mobile APs ${c.mobileAps}" else ""), style = line, color = HoneTextDim)
        Text("Wi-Fi scans ${c.wifiScansOk} ok / ${c.wifiScansRefused} throttled · throttling " +
            when (c.wifiThrottled) { true -> "on"; false -> "off"; null -> "?" }, style = line, color = HoneTextFaint)
        Text("Advertising key ${c.lidPrefix}… ${if (c.advertising) "on (${c.advertMode})" else "off"} · " +
            "${if (c.seedDerived) "from hide key" else "provisional"} · summaries ${c.summariesStored} (${c.summariesQueued} queued)",
            style = line, color = HoneTextFaint)
        c.issues.forEach { Text(it, style = line, color = MaterialTheme.colorScheme.error) }
        (routerCode ?: c.routerPending)?.let {
            Text("Router check: put \"$it\" in your router's network name for a few minutes.", style = line, color = HoneGreen)
        }
        Row {
            TextButton(onClick = onFindMy) { Text("Find my things") }
            TextButton(onClick = { routerCode = SightingsEngine.beginRouterRegistration() }) { Text("Add my router") }
        }
        if (c.issues.any { it.startsWith("location only") } && Build.VERSION.SDK_INT >= 29) {
            TextButton(onClick = {
                if (Build.VERSION.SDK_INT >= 30) {
                    // Android 11+: "all the time" can only be chosen in Settings.
                    ctx.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.fromParts("package", ctx.packageName, null)))
                } else bgLoc.launch(Manifest.permission.ACCESS_BACKGROUND_LOCATION)
            }) { Text("Allow location all the time") }
        }
    }
}
