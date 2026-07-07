package network.hone.app.ui.screens

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import network.hone.app.data.MockWalletRepository
import network.hone.app.data.SendResult
import network.hone.app.data.WalletRepository

/** Transient UI state for the send flow. */
data class SendUiState(
    val recipient: String = "",
    val amount: String = "",
    val memo: String = "",
    val sending: Boolean = false,
    val error: String? = null,
    val lastResult: SendResult? = null,
) {
    val amountValid: Boolean get() = amount.toDoubleOrNull()?.let { it > 0 } == true
}

class WalletViewModel(
    val repo: WalletRepository = MockWalletRepository(),
) : ViewModel() {

    val identity = repo.identity
    val history = repo.history

    private val _send = MutableStateFlow(SendUiState())
    val send = _send.asStateFlow()

    fun setRecipient(v: String) { _send.value = _send.value.copy(recipient = v, error = null) }
    fun setAmount(v: String) { _send.value = _send.value.copy(amount = v, error = null) }
    fun setMemo(v: String) { _send.value = _send.value.copy(memo = v) }

    fun canSend(): Boolean {
        val s = _send.value
        return !s.sending && s.amountValid && repo.isValidRecipient(s.recipient)
    }

    /** Call ONLY after biometric auth has succeeded. */
    fun confirmSend() {
        val s = _send.value
        if (!canSend()) return
        _send.value = s.copy(sending = true, error = null)
        viewModelScope.launch {
            val result = repo.send(s.recipient, s.amount.toDouble(), s.memo)
            _send.value = when (result) {
                is SendResult.Submitted -> SendUiState(lastResult = result) // reset form on success
                is SendResult.Rejected -> _send.value.copy(sending = false, error = result.reason)
            }
        }
    }

    fun clearResult() { _send.value = _send.value.copy(lastResult = null) }
}
