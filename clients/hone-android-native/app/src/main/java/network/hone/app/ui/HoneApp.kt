package network.hone.app.ui

import android.content.Context
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Dashboard
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Paid
import androidx.compose.material.icons.filled.FlutterDash
import androidx.compose.material.icons.filled.Hearing
import androidx.compose.material.icons.filled.Map
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.WbSunny
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import network.hone.app.ui.screens.BirdsScreen
import network.hone.app.ui.screens.NodeScreen
import network.hone.app.ui.screens.SensorsScreen
import network.hone.app.ui.screens.SettingsScreen
import network.hone.app.ui.screens.WalletScreen
import network.hone.app.ui.theme.*
import network.hone.app.ui.theme.OrnChorus
import network.hone.app.ui.theme.OrnInkDim
import network.hone.app.ui.theme.OrnNight

/** Bottom-nav destinations. Material Symbols icons (Decision #3 — no emoji). */
private enum class Dest(val route: String, val label: String, val icon: ImageVector) {
    Map("map", "Map", Icons.Filled.Map),
    Listen("listen", "Listen", Icons.Filled.Hearing),
    Birds("birds", "Birds", Icons.Filled.FlutterDash),
    Today("today", "Today", Icons.Filled.WbSunny),
    Node("node", "Node", Icons.Filled.Dashboard),
    Wallet("wallet", "Wallet", Icons.Filled.AccountBalanceWallet),
    Sensors("sensors", "Sensors", Icons.Filled.Sensors),
    Earnings("earnings", "Earnings", Icons.Filled.Paid),
    Devices("devices", "Devices", Icons.Filled.Devices),
    Settings("settings", "Settings", Icons.Filled.Settings),
}

/**
 * Ornigo mode (default for everyone without a wallet seat) hides wallet, Find My, node and keys.
 * HONE mode: a phone with a wallet seat, or switched on under Settings > Advanced > HONE mode.
 */
object AppMode {
    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_mode", Context.MODE_PRIVATE)
    fun hasSeat(ctx: Context) = java.io.File(ctx.filesDir, "hone-node/witness_account").exists()
    fun hone(ctx: Context) = prefs(ctx).getBoolean("hone", hasSeat(ctx))
    fun setHone(ctx: Context, on: Boolean) = prefs(ctx).edit().putBoolean("hone", on).apply()
    fun light(ctx: Context) = prefs(ctx).getBoolean("light", false)
    fun setLight(ctx: Context, on: Boolean) = prefs(ctx).edit().putBoolean("light", on).apply()
    val version = mutableIntStateOf(0)
}

@Composable
fun HoneApp() {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val modeVersion by AppMode.version
    val honeMode = remember(modeVersion) { AppMode.hone(ctx) }
    // HONE light colours are opt-in until they have been checked on a phone (Settings > Advanced).
    network.hone.app.ui.theme.ThemeMode.light = honeMode && AppMode.light(ctx)
    key(honeMode) { AppShell(honeMode) }
}

@Composable
private fun AppShell(honeMode: Boolean) {
    val nav = rememberNavController()
    val backStack by nav.currentBackStackEntryAsState()
    val current = backStack?.destination
    val ctx = androidx.compose.ui.platform.LocalContext.current
    // "simple" settings (Node and Wallet under Settings > Advanced) whenever Ornigo mode is on.
    val simple = !honeMode
    var firstRun by remember { mutableStateOf(simple && !network.hone.app.account.AccountStore.onboarded(ctx)) }
    var importKey by remember { mutableStateOf(false) }
    if (importKey) { network.hone.app.ui.screens.FindMyScreen(onBack = { importKey = false }); return }
    var account by remember { mutableStateOf(false) }
    if (account) {
        Scaffold(containerColor = HoneBg) { inner ->
            androidx.compose.foundation.layout.Box(Modifier.padding(inner)) {
                network.hone.app.ui.screens.AccountFlow(onSignedIn = { account = false }, onSkip = { account = false }, onImportKey = { importKey = true })
            }
        }
        return
    }
    if (firstRun && !honeMode) {
        Scaffold(topBar = { network.hone.app.update.UpdateBanner() }, containerColor = HoneBg) { inner ->
            androidx.compose.foundation.layout.Box(Modifier.padding(inner)) {
                network.hone.app.ui.screens.OrnigoFirstRun(onDone = { firstRun = false }, onSignIn = { account = true })
            }
        }
        return
    }
    if (firstRun) {
        Scaffold(topBar = { network.hone.app.update.UpdateBanner() }, containerColor = HoneBg) { inner ->
            androidx.compose.foundation.layout.Box(Modifier.padding(inner)) {
                network.hone.app.ui.screens.FirstRunScreen(onDone = { firstRun = false }, onImportKey = { importKey = true })
            }
        }
        return
    }
    val tabs = if (honeMode) listOf(Dest.Wallet, Dest.Earnings, Dest.Devices, Dest.Birds, Dest.Settings)
        else listOf(Dest.Map, Dest.Listen, Dest.Birds, Dest.Today)
    fun go(route: String) = nav.navigate(route) {
        popUpTo(nav.graph.findStartDestination().id) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }

    Scaffold(
        containerColor = HoneBg,
        topBar = { network.hone.app.update.UpdateBanner() },
        bottomBar = {
            NavigationBar(containerColor = HoneSurface) {
                tabs.forEach { dest ->
                    val selected = current?.hierarchy?.any { it.route == dest.route } == true
                    NavigationBarItem(
                        selected = selected,
                        onClick = { go(dest.route) },
                        icon = { Icon(dest.icon, contentDescription = dest.label) },
                        label = { FitText(dest.label) },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = if (honeMode) androidx.compose.ui.graphics.Color.White else OrnNight, indicatorColor = HoneOrange,
                            selectedTextColor = HoneOrange, unselectedIconColor = HoneTextDim, unselectedTextColor = HoneTextDim),
                    )
                }
            }
        },
    ) { inner ->
        NavHost(
            navController = nav,
            startDestination = if (honeMode) Dest.Wallet.route else Dest.Map.route,
            modifier = Modifier.padding(inner),
        ) {
            composable(Dest.Map.route) { network.hone.app.ui.screens.OrnigoMapScreen(onListen = { t ->
                nav.navigate("encounter" + (t?.let { "?t=" + android.net.Uri.encode(it) } ?: "")) }, onSpots = { nav.navigate("spots") }) }
            composable("spots") { network.hone.app.ui.screens.SpotsScreen(onBack = { nav.popBackStack() }) }
            composable("encounter?t={t}", arguments = listOf(androidx.navigation.navArgument("t") {
                type = androidx.navigation.NavType.StringType; nullable = true; defaultValue = null })) { e ->
                network.hone.app.ui.screens.EncounterScreen(target = e.arguments?.getString("t"),
                    onCollection = { go(Dest.Birds.route) }, onBack = { nav.popBackStack() })
            }
            composable(Dest.Today.route) { network.hone.app.ui.screens.TodayScreen(onSettings = { nav.navigate(Dest.Settings.route) }, onSave = { account = true }, onHone = { nav.navigate("handoff") }) }
            composable("handoff") { network.hone.app.ui.screens.HandoffScreen(onBack = { nav.popBackStack() }) }
            composable(Dest.Node.route) { NodeScreen() }
            composable(Dest.Wallet.route) { WalletScreen(onEarnings = { go(Dest.Earnings.route) }, onSponsor = { nav.navigate("sponsor") }) }
            composable(Dest.Earnings.route) { network.hone.app.ui.screens.EarningsScreen() }
            composable(Dest.Devices.route) { network.hone.app.ui.screens.DevicesScreen(onNode = { nav.navigate(Dest.Node.route) },
                onSensors = { nav.navigate(Dest.Sensors.route) }, onRoles = { nav.navigate("roles") }, onFindMy = { nav.navigate("findmy") }) }
            composable("sponsor") { network.hone.app.ui.screens.SponsorScreen(onBack = { nav.popBackStack() }) }
            composable("roles") { network.hone.app.ui.screens.RolesScreen(onBack = { nav.popBackStack() }) }
            composable("findmy") { network.hone.app.ui.screens.FindMyScreen(onBack = { nav.popBackStack() }) }
            composable(Dest.Sensors.route) { SensorsScreen() }
            composable(Dest.Birds.route) { BirdsScreen() }
            composable(Dest.Settings.route) { SettingsScreen(simple = simple, onOpen = { nav.navigate(it) }, onSignIn = { firstRun = true }) }
            composable(Dest.Listen.route) { network.hone.app.ui.screens.ListenScreen() }
        }
    }
}
