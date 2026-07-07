package network.hone.app.data

import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Wallet operations, kept SEPARATE from node ops so the wallet screen has a
 * focused seam. Screens depend on this interface, not the bridge.
 *
 * SECURITY CONTRACT (enforced by the real bridge impl in Phase 2/0b):
 *   - The private key NEVER leaves the Rust keystore (Argon2id+AES-GCM).
 *   - `send` is a SIGN-REQUEST: biometric-gated, human-confirmed, NEVER
 *     auto-signed. Founder-wallet transfers route as sign-requests per the
 *     standing rule. This mock mirrors the shape, not the crypto.
 */
interface WalletRepository {
    val identity: StateFlow<WalletIdentity>
    val history: StateFlow<List<Tx>>
    /** Validate a recipient handle/address without sending. */
    fun isValidRecipient(recipient: String): Boolean
    /** Attempt a send. Caller must have already passed biometric auth. */
    suspend fun send(recipient: String, amountHone: Double, memo: String?): SendResult
}

class MockWalletRepository : WalletRepository {

    private val _identity = MutableStateFlow(
        WalletIdentity(
            name = "@bullship",
            address = "hh1qeven0j4kz8xk2m9y7wq0r3s5t7v9x1c3e5g7i9",
            balanceHunits = 1_250_000_000UL,      // 0.125 HONE
            delegatedHunits = 500_000_000_000UL,  // 50 HONE delegated
        )
    )
    override val identity: StateFlow<WalletIdentity> = _identity.asStateFlow()

    private val _history = MutableStateFlow(
        listOf(
            Tx("t1", TxDirection.IN, "clock-reward", 4_200_000UL, 2962UL, TxStatus.CONFIRMED, "epoch seal"),
            Tx("t2", TxDirection.OUT, "@natoshisakamoto", 100_000_000UL, 2951UL, TxStatus.CONFIRMED, "split"),
            Tx("t3", TxDirection.IN, "mine-reward", 4_200_000UL, 2948UL, TxStatus.CONFIRMED),
            Tx("t4", TxDirection.OUT, "@josh", 50_000_000UL, 2940UL, TxStatus.CONFIRMED, "gas"),
        )
    )
    override val history: StateFlow<List<Tx>> = _history.asStateFlow()

    override fun isValidRecipient(recipient: String): Boolean {
        val r = recipient.trim()
        // A name (@handle) or a typed bech32 address (hh1…). Loose mock check.
        return (r.startsWith("@") && r.length in 2..64) ||
               (r.startsWith("hh1") && r.length in 10..90)
    }

    override suspend fun send(recipient: String, amountHone: Double, memo: String?): SendResult {
        val r = recipient.trim()
        if (!isValidRecipient(r)) return SendResult.Rejected("Invalid recipient")
        val hunits = (amountHone * 10_000_000_000.0).toULong()
        if (hunits == 0UL) return SendResult.Rejected("Amount must be greater than zero")
        if (hunits > _identity.value.balanceHunits) return SendResult.Rejected("Insufficient balance")

        delay(1_200) // simulate network round-trip / sealing

        val tx = Tx(
            id = "t${System.currentTimeMillis()}",
            direction = TxDirection.OUT,
            counterparty = r,
            amountHunits = hunits,
            epoch = 2964UL,
            status = TxStatus.PENDING,
            memo = memo?.takeIf { it.isNotBlank() },
        )
        _history.value = listOf(tx) + _history.value
        _identity.value = _identity.value.copy(
            balanceHunits = _identity.value.balanceHunits - hunits
        )
        return SendResult.Submitted(tx)
    }
}
