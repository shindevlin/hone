package network.hone.app.ui.screens

import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringArrayResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import network.hone.app.R
import network.hone.app.fw.FirmwareInstaller
import network.hone.app.fw.FirmwarePackage
import network.hone.app.ui.theme.*

private const val OFFICIAL_URL = "https://honemesh.net/app/firmware/official.json"

@Composable
fun FirmwareScreen(onBack: () -> Unit, onHowTo: () -> Unit = {}) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val inst = remember { FirmwareInstaller(ctx.applicationContext) }
    val appCode = remember { runCatching { androidx.core.content.pm.PackageInfoCompat.getLongVersionCode(ctx.packageManager.getPackageInfo(ctx.packageName, 0)).toInt() }.getOrDefault(0) }
    val st by inst.state.collectAsState()
    var link by remember { mutableStateOf("none") }
    var rel by remember { mutableStateOf<FirmwarePackage.Release?>(null) }
    var relErr by remember { mutableStateOf<String?>(null) }
    var agree by remember { mutableStateOf(false) }
    var official by remember { mutableStateOf(false) }
    val url = if (official) OFFICIAL_URL else FirmwarePackage.MANIFEST_URL
    LaunchedEffect(Unit) { while (true) { link = inst.link(); delay(2_000) } }
    LaunchedEffect(url) {
        rel = null; relErr = null
        runCatching { inst.fetchRelease(url, appCode) }.onSuccess { rel = it }.onFailure { relErr = it.message }
    }
    val busy = st is FirmwareInstaller.State.Working
    val noKey = FirmwarePackage.RELEASE_KEYS.isEmpty()

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle(stringResource(R.string.fw_title), Modifier.weight(1f))
            TextButton(onBack, enabled = !busy) { Text(stringResource(R.string.fw_back), color = HoneOrange) }
        }
        Dim(stringResource(R.string.fw_intro))
        Text(stringResource(when (link) { "USB" -> R.string.fw_link_usb; "Bluetooth" -> R.string.fw_link_ble; else -> R.string.fw_link_none }),
            fontWeight = FontWeight.SemiBold)

        if (link == "none") {
            Card { Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(stringResource(R.string.fw_cable_title), fontWeight = FontWeight.SemiBold)
                Dim(stringResource(R.string.fw_cable_steps))
                Spacer(Modifier.height(6.dp))
                Text(stringResource(R.string.fw_pair_title), fontWeight = FontWeight.SemiBold)
                Dim(stringResource(R.string.fw_pair_steps))
                OutlinedButton({ runCatching { ctx.startActivity(Intent(Settings.ACTION_BLUETOOTH_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } }) {
                    Text(stringResource(R.string.fw_pair_open))
                }
            } }
        }

        if (noKey) Dim(stringResource(R.string.fw_no_key))
        when {
            rel != null -> Text(stringResource(R.string.fw_latest, rel!!.version))
            relErr != null -> Dim(relErr!!)
            else -> Dim(stringResource(R.string.fw_checking))
        }

        if (!official) {
            Card { Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(stringResource(R.string.fw_stock_title), fontWeight = FontWeight.SemiBold)
                Dim(stringResource(R.string.fw_stock_body))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(agree, { agree = it }, enabled = !busy); Text(stringResource(R.string.fw_stock_agree))
                }
            } }
        }

        when (val s = st) {
            is FirmwareInstaller.State.Working -> {
                Text(stepName(s.step), fontWeight = FontWeight.SemiBold)
                if (s.total > 0) LinearProgressIndicator(progress = { (s.done.toFloat() / s.total).coerceIn(0f, 1f) }, Modifier.fillMaxWidth())
                else LinearProgressIndicator(Modifier.fillMaxWidth())
                if (s.detail.isNotEmpty()) Dim(s.detail)
                Dim(stringResource(R.string.fw_keep_close))
            }
            is FirmwareInstaller.State.Done -> Text(stringResource(R.string.fw_done, s.version, s.via), color = HoneOrange, fontWeight = FontWeight.SemiBold)
            is FirmwareInstaller.State.Failed -> { Text(stringResource(R.string.fw_failed, stepName(s.step), s.reason)); Dim(stringResource(R.string.fw_safe)) }
            else -> Dim(stringResource(R.string.fw_safe))
        }

        val r = rel
        val canStart = r != null && !busy && !noKey && link != "none" && (official || agree)
        Button({ scope.launch { inst.install(appCode, url) } }, enabled = canStart, modifier = Modifier.fillMaxWidth()) {
            Text(if (st is FirmwareInstaller.State.Failed && (st as FirmwareInstaller.State.Failed).retry) stringResource(R.string.fw_retry)
                 else stringResource(R.string.fw_install, r?.version ?: "…"))
        }

        HorizontalDivider()
        Text(stringResource(R.string.fw_switch_back), fontWeight = FontWeight.SemiBold)
        Dim(stringResource(R.string.fw_switch_back_body))
        Row {
            TextButton({ official = !official }, enabled = !busy) {
                Text(if (official) stringResource(R.string.fw_title) else stringResource(R.string.fw_switch_back), color = HoneOrange)
            }
            TextButton(onHowTo) { Text(stringResource(R.string.howto_open), color = HoneOrange) }
        }
    }
}

@Composable
private fun stepName(s: FirmwareInstaller.Step): String = stringResource(when (s) {
    FirmwareInstaller.Step.MANIFEST -> R.string.fw_step_MANIFEST
    FirmwareInstaller.Step.DOWNLOAD -> R.string.fw_step_DOWNLOAD
    FirmwareInstaller.Step.CONNECT -> R.string.fw_step_CONNECT
    FirmwareInstaller.Step.CHECK -> R.string.fw_step_CHECK
    FirmwareInstaller.Step.UPLOAD -> R.string.fw_step_UPLOAD
    FirmwareInstaller.Step.VERIFY -> R.string.fw_step_VERIFY
    FirmwareInstaller.Step.START -> R.string.fw_step_START
    FirmwareInstaller.Step.REBOOT -> R.string.fw_step_REBOOT
    FirmwareInstaller.Step.CONFIRM -> R.string.fw_step_CONFIRM
})

/** In-app "How to": the same four guides as honemesh.net/app/guides, in the app's language. */
@Composable
fun HowToScreen(onBack: () -> Unit) {
    val guides = listOf(R.string.howto_apk to R.array.howto_apk_steps, R.string.howto_usb to R.array.howto_usb_steps,
        R.string.howto_ble to R.array.howto_ble_steps, R.string.howto_stock to R.array.howto_stock_steps)
    var open by remember { mutableIntStateOf(1) }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle(stringResource(R.string.howto_title), Modifier.weight(1f))
            TextButton(onBack) { Text(stringResource(R.string.fw_back), color = HoneOrange) }
        }
        guides.forEachIndexed { i, (title, steps) ->
            Card(onClick = { open = i }) { Column(Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(stringResource(title), fontWeight = FontWeight.SemiBold)
                if (open == i) stringArrayResource(steps).forEachIndexed { n, s -> Text("${n + 1}. $s") }
            } }
        }
    }
}
