package network.hone.app.ui.screens

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import network.hone.app.data.MockNodeRepository
import network.hone.app.data.NodeRepository
import network.hone.app.data.NodeSnapshot

/**
 * Holds node state for the UI. Depends on [NodeRepository] (currently the mock;
 * swaps to the bridge-backed repo in Phase 0b via constructor injection — no UI
 * change). The `state` StateFlow is what Compose collects.
 */
class NodeViewModel(
    private val repo: NodeRepository = MockNodeRepository(),
) : ViewModel() {

    val state = repo.observe().stateIn(
        scope = viewModelScope,
        started = SharingStarted.WhileSubscribed(5_000),
        initialValue = NodeSnapshot(
            running = false, statusText = "Idle", epoch = 0UL,
            balanceHunits = 0UL, peers = 0, isMiner = true, isClock = true,
        ),
    )

    fun toggleNode(current: NodeSnapshot) = viewModelScope.launch {
        if (current.running) repo.stop()
        else repo.start(isMiner = current.isMiner, isClock = current.isClock)
    }
}
