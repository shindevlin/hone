package network.hone.app.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Dashboard
import androidx.compose.material.icons.filled.FlutterDash
import androidx.compose.material.icons.filled.Hearing
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.filled.Settings
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

/** Bottom-nav destinations. Material Symbols icons (Decision #3 — no emoji). */
private enum class Dest(val route: String, val label: String, val icon: ImageVector) {
    Node("node", "Node", Icons.Filled.Dashboard),
    Wallet("wallet", "Wallet", Icons.Filled.AccountBalanceWallet),
    Sensors("sensors", "Sensors", Icons.Filled.Sensors),
    Birds("birds", "Birds", Icons.Filled.FlutterDash),
    Settings("settings", "Settings", Icons.Filled.Settings),
    Listen("listen", "Listening", Icons.Filled.Hearing),
}

@Composable
fun HoneApp() {
    val nav = rememberNavController()
    val backStack by nav.currentBackStackEntryAsState()
    val current = backStack?.destination
    val ctx = androidx.compose.ui.platform.LocalContext.current
    // Simple mode: no wallet seat on this phone (a volunteer). Node and Wallet live under Settings > Advanced.
    val simple = remember { !java.io.File(ctx.filesDir, "hone-node/witness_account").exists() }
    var firstRun by remember { mutableStateOf(simple && !network.hone.app.account.AccountStore.onboarded(ctx)) }
    var importKey by remember { mutableStateOf(false) }
    if (importKey) { network.hone.app.ui.screens.FindMyScreen(onBack = { importKey = false }); return }
    if (firstRun) {
        Scaffold(topBar = { network.hone.app.update.UpdateBanner() }) { inner ->
            androidx.compose.foundation.layout.Box(Modifier.padding(inner)) {
                network.hone.app.ui.screens.FirstRunScreen(onDone = { firstRun = false }, onImportKey = { importKey = true })
            }
        }
        return
    }
    val tabs = if (simple) listOf(Dest.Birds, Dest.Listen, Dest.Settings) else Dest.entries.filter { it != Dest.Listen }

    Scaffold(
        topBar = { network.hone.app.update.UpdateBanner() },
        bottomBar = {
            NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
                tabs.forEach { dest ->
                    val selected = current?.hierarchy?.any { it.route == dest.route } == true
                    NavigationBarItem(
                        selected = selected,
                        onClick = {
                            nav.navigate(dest.route) {
                                popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                                launchSingleTop = true
                                restoreState = true
                            }
                        },
                        icon = { Icon(dest.icon, contentDescription = dest.label) },
                        label = { Text(dest.label) },
                    )
                }
            }
        },
    ) { inner ->
        NavHost(
            navController = nav,
            startDestination = if (simple) Dest.Birds.route else Dest.Node.route,
            modifier = Modifier.padding(inner),
        ) {
            composable(Dest.Node.route) { NodeScreen() }
            composable(Dest.Wallet.route) { WalletScreen() }
            composable(Dest.Sensors.route) { SensorsScreen() }
            composable(Dest.Birds.route) { BirdsScreen() }
            composable(Dest.Settings.route) { SettingsScreen(simple = simple, onOpen = { nav.navigate(it) }, onSignIn = { firstRun = true }) }
            composable(Dest.Listen.route) { network.hone.app.ui.screens.ListenScreen() }
        }
    }
}
