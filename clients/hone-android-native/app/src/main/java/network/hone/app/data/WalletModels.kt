package network.hone.app.data

/** A wallet's public identity. `name` is the human handle (@bullship-style);
 *  `address` is the typed bech32 form (hh1…). Both are display-only here — the
 *  private key never leaves the Rust keystore. */
data class WalletIdentity(
    val name: String,
    val address: String,
    val balanceHunits: ULong,
    val delegatedHunits: ULong,
) {
    val balanceHone: Double get() = balanceHunits.toDouble() / 10_000_000_000.0
    val delegatedHone: Double get() = delegatedHunits.toDouble() / 10_000_000_000.0
}

enum class TxDirection { IN, OUT }
enum class TxStatus { PENDING, CONFIRMED, FAILED }

data class Tx(
    val id: String,
    val direction: TxDirection,
    val counterparty: String, // name or address of the other side
    val amountHunits: ULong,
    val epoch: ULong,
    val status: TxStatus,
    val memo: String? = null,
) {
    val amountHone: Double get() = amountHunits.toDouble() / 10_000_000_000.0
}

/** Result of a send attempt. In the real bridge, a send is a SIGN-REQUEST that
 *  is biometric-gated and human-confirmed — NEVER auto-signed. */
sealed interface SendResult {
    data class Submitted(val tx: Tx) : SendResult
    data class Rejected(val reason: String) : SendResult
}
