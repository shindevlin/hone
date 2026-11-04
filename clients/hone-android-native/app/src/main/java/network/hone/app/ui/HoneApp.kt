package network.hone.app.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Dashboard
import androidx.compose.material.icons.filled.FlutterDash
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
}

@Composable
fun HoneApp() {
    val nav = rememberNavController()
    val backStack by nav.currentBackStackEntryAsState()
    val current = backStack?.destination

    Scaffold(
        topBar = { network.hone.app.update.UpdateBanner() },
        bottomBar = {
            NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
                Dest.entries.forEach { dest ->
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
            startDestination = Dest.Node.route,
            modifier = Modifier.padding(inner),
        ) {
            composable(Dest.Node.route) { NodeScreen() }
            composable(Dest.Wallet.route) { WalletScreen() }
            composable(Dest.Sensors.route) { SensorsScreen() }
            composable(Dest.Birds.route) { BirdsScreen() }
            composable(Dest.Settings.route) { SettingsScreen() }
        }
    }
}
