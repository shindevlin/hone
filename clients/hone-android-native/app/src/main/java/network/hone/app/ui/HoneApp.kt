package network.hone.app.ui

import android.content.Context
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Dashboard
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
import network.hone.app.ui.theme.OrnCanopy
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
    val version = mutableIntStateOf(0)
}

@Composable
fun HoneApp() {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val modeVersion by AppMode.version
    val honeMode = remember(modeVersion) { AppMode.hone(ctx) }
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
    if (firstRun) {
        Scaffold(topBar = { network.hone.app.update.UpdateBanner() }, containerColor = OrnNight) { inner ->
            androidx.compose.foundation.layout.Box(Modifier.padding(inner)) {
                network.hone.app.ui.screens.FirstRunScreen(onDone = { firstRun = false }, onImportKey = { importKey = true })
            }
        }
        return
    }
    val tabs = if (honeMode) listOf(Dest.Node, Dest.Wallet, Dest.Sensors, Dest.Birds, Dest.Settings)
        else listOf(Dest.Map, Dest.Listen, Dest.Birds, Dest.Today)
    fun go(route: String) = nav.navigate(route) {
        popUpTo(nav.graph.findStartDestination().id) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }

    Scaffold(
        containerColor = OrnNight,
        topBar = { network.hone.app.update.UpdateBanner() },
        bottomBar = {
            NavigationBar(containerColor = OrnCanopy) {
                tabs.forEach { dest ->
                    val selected = current?.hierarchy?.any { it.route == dest.route } == true
                    NavigationBarItem(
                        selected = selected,
                        onClick = { go(dest.route) },
                        icon = { Icon(dest.icon, contentDescription = dest.label) },
                        label = { Text(dest.label) },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = OrnNight, indicatorColor = OrnChorus,
                            selectedTextColor = OrnChorus, unselectedIconColor = OrnInkDim, unselectedTextColor = OrnInkDim),
                    )
                }
            }
        },
    ) { inner ->
        NavHost(
            navController = nav,
            startDestination = if (honeMode) Dest.Node.route else Dest.Map.route,
            modifier = Modifier.padding(inner),
        ) {
            composable(Dest.Map.route) { network.hone.app.ui.screens.OrnigoMapScreen(onListen = { go(Dest.Listen.route) }) }
            composable(Dest.Today.route) { network.hone.app.ui.screens.TodayScreen(onSettings = { nav.navigate(Dest.Settings.route) }) }
            composable(Dest.Node.route) { NodeScreen() }
            composable(Dest.Wallet.route) { WalletScreen() }
            composable(Dest.Sensors.route) { SensorsScreen() }
            composable(Dest.Birds.route) { BirdsScreen() }
            composable(Dest.Settings.route) { SettingsScreen(simple = simple, onOpen = { nav.navigate(it) }, onSignIn = { firstRun = true }) }
            composable(Dest.Listen.route) { network.hone.app.ui.screens.ListenScreen() }
        }
    }
}
