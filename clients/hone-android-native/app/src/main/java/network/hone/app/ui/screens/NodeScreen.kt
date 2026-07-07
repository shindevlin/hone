package network.hone.app.ui.screens

import androidx.compose.animation.core.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import network.hone.app.data.NodeSnapshot
import network.hone.app.ui.components.StatCard
import network.hone.app.ui.theme.HoneGreen
import network.hone.app.ui.theme.HoneMono
import network.hone.app.ui.theme.HoneRed
import network.hone.app.ui.theme.HoneTextDim

/**
 * The Node dashboard — the home screen. Shows live node state (balance, epoch,
 * peers, roles) and the primary mine/clock toggle. Everything animates so the
 * chain feels alive; the toggle gives haptic feedback (the #1 "native feel"
 * signal the old webview lacked).
 */
@Composable
fun NodeScreen(vm: NodeViewModel = viewModel()) {
    val state by vm.state.collectAsStateWithLifecycle()
    Column(
        Modifier
            .fillMaxSize()
            .padding(horizontal = 20.dp)
            .padding(top = 24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Header(state)
        BalanceHero(state)
        StatGrid(state)
        Spacer(Modifier.weight(1f))
        MineToggle(state, onToggle = { vm.toggleNode(state) })
        Spacer(Modifier.height(12.dp))
    }
}

@Composable
private fun Header(s: NodeSnapshot) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text("HONE", style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold, color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.weight(1f))
        StatusPill(running = s.running, text = s.statusText)
    }
}

@Composable
private fun StatusPill(running: Boolean, text: String) {
    val dot = if (running) HoneGreen else HoneRed
    // Gentle pulse on the live dot.
    val infinite = rememberInfiniteTransition(label = "pulse")
    val alpha by infinite.animateFloat(
        initialValue = if (running) 0.4f else 1f, targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(900), RepeatMode.Reverse), label = "dotAlpha",
    )
    Row(
        Modifier
            .background(MaterialTheme.colorScheme.surface, CircleShape)
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .semantics { contentDescription = "Node status: $text" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(Modifier.size(8.dp).background(dot.copy(alpha = alpha), CircleShape))
        Text(text, style = MaterialTheme.typography.labelLarge, color = HoneTextDim)
    }
}

@Composable
private fun BalanceHero(s: NodeSnapshot) {
    // Smoothly animate the balance so rewards visibly count up.
    val animated by animateFloatAsState(
        targetValue = s.balanceHone.toFloat(),
        animationSpec = tween(800, easing = FastOutSlowInEasing), label = "balance",
    )
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
        Text("BALANCE", style = MaterialTheme.typography.labelSmall, color = HoneTextDim)
        Spacer(Modifier.height(4.dp))
        Row(verticalAlignment = Alignment.Bottom) {
            Text(
                "%.6f".format(animated),
                style = MaterialTheme.typography.displayLarge.copy(fontFamily = HoneMono),
                color = MaterialTheme.colorScheme.onBackground,
            )
            Spacer(Modifier.width(8.dp))
            Text("HONE", style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(bottom = 8.dp))
        }
    }
}

@Composable
private fun StatGrid(s: NodeSnapshot) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            StatCard("EPOCH", s.epoch.toString(), Icons.Filled.Schedule,
                Modifier.weight(1f), accent = s.running)
            StatCard("PEERS", s.peers.toString(), Icons.Filled.Hub, Modifier.weight(1f))
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            StatCard("MINER", if (s.isMiner) "ON" else "OFF", Icons.Filled.Memory,
                Modifier.weight(1f), valueMono = false, accent = s.isMiner && s.running)
            StatCard("CLOCK", if (s.isClock) "ON" else "OFF", Icons.Filled.Timer,
                Modifier.weight(1f), valueMono = false, accent = s.isClock && s.running)
        }
    }
}

@Composable
private fun MineToggle(s: NodeSnapshot, onToggle: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    var pressed by remember { mutableStateOf(false) }
    val scale by animateFloatAsState(if (pressed) 0.97f else 1f, label = "press")
    val bg = if (s.running) HoneRed.copy(alpha = 0.15f) else MaterialTheme.colorScheme.primary
    val fg = if (s.running) HoneRed else Color.Black
    val label = if (s.running) "Stop node" else "Start mining"

    Button(
        onClick = {
            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
            pressed = true
            onToggle()
        },
        modifier = Modifier
            .fillMaxWidth()
            .height(64.dp)
            .scale(scale)
            .semantics { contentDescription = label },
        shape = RoundedCornerShape(18.dp),
        colors = ButtonDefaults.buttonColors(containerColor = bg, contentColor = fg),
    ) {
        Icon(if (s.running) Icons.Filled.Stop else Icons.Filled.Bolt, contentDescription = null)
        Spacer(Modifier.width(10.dp))
        Text(label, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
    }
    LaunchedEffect(pressed) { if (pressed) { kotlinx.coroutines.delay(120); pressed = false } }
}
