package network.hone.app.data

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * The one confirm screen every active-key signature goes through (shown by VaultPromptHost):
 * what is being signed, the amount in HONE, where it goes, then the password.
 */
object VaultPrompt {
    /** [amountHunits] null = no tokens move. 1 HONE = 10^10 hunits. */
    data class Confirm(
        val action: String,
        val amountHunits: Long?,
        val destination: String,
        val extra: List<Pair<String, String>> = emptyList(),
        val error: String? = null,
        val waitUntil: Long = 0L,
        /** Shown instead of [amountHunits] when a caller only has formatted text. */
        val amountLabel: String? = null,
    ) {
        val amountText: String get() = amountHunits?.let { WalletChain.fmt(it) } ?: amountLabel ?: "None (no HONE moves)"
    }

    class Request(val confirm: Confirm, val answer: CompletableDeferred<CharArray?>)

    var pending by mutableStateOf<Request?>(null)
        private set
    private val one = Mutex()

    /** Suspends until the user enters a password (returned, caller zeroes it) or cancels (null). */
    suspend fun ask(confirm: Confirm): CharArray? = one.withLock {
        val r = Request(confirm, CompletableDeferred())
        pending = r
        try { r.answer.await() } finally { if (pending === r) pending = null }
    }
}
