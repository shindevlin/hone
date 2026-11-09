package network.hone.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import network.hone.app.data.HoneChain
import network.hone.app.ui.theme.*

/** Bird cards: claim, send, history. Driven by the chain's BIRD_CARDS gate (/chain/api/bird/:id active_from_epoch). */
@Composable
fun CardsScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    var gate by remember { mutableStateOf(ChainGate.Unknown) }
    var kinds by remember { mutableStateOf<Int?>(null) }
    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            gate = when (HoneChain.birdCardsOn()) { true -> ChainGate.Live; false -> ChainGate.Off; null -> ChainGate.Unknown }
            kinds = runCatching { network.hone.app.birds.Catches.all(ctx).map { it.name }.distinct().size }.getOrNull()
        }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 20.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HoneTitle("Bird cards", Modifier.weight(1f))
            TextButton(onBack) { Text("Back", color = HoneOrange) }
        }
        ComingSoon("Claim, send and keep cards",
            "A card is made for a bird that was found first, or named, with proof (a voiceprint, a second listener or a photo). " +
            "Claim it to your wallet, send it to someone, and every card keeps its story. \"Found by\" never changes.", gate)
        if (gate == ChainGate.Live) Dim("Claiming and sending from the phone arrive in a coming app update.")
        HoneBox {
            Text("Your birds so far", fontWeight = FontWeight.Bold, color = HoneText)
            Dim(kinds?.let { if (it == 0) "No catches on this phone yet. Catch birds in Listen." else "$it ${if (it == 1) "kind" else "kinds"} caught on this phone. Firsts and named birds can become cards." }
                ?: "…")
        }
        Dim("Cards are earned, never sold.", color = HoneTextFaint)
        Tech("BIRD_CARD_CLAIM · BIRD_CARD_SEND · /api/bird/:bird_id · /api/bird/card/:card")
    }
}
