package network.hone.app

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.fragment.app.FragmentActivity
import network.hone.app.ui.HoneApp
import network.hone.app.ui.theme.HoneTheme

// FragmentActivity (not plain ComponentActivity) so BiometricPrompt can attach
// to it for the wallet's sign-request gate. Compose works the same either way.
class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        network.hone.app.telemetry.Telemetry.installCrashHandler(this)
        // Signed status every 15 min even when the sensor service is not running.
        runCatching { network.hone.app.telemetry.StatusWorker.schedule(this) }
        handleJoin(intent)
        network.hone.app.engage.Engagement.scheduleDigest(this)
        enableEdgeToEdge()
        setContent {
            HoneTheme {
                HoneApp()
            }
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent); handleJoin(intent)
    }

    /** hone://join?ref=josh (from https://honemesh.net/join) carries the invite code. */
    private fun handleJoin(i: android.content.Intent?) {
        val u = i?.data ?: return
        if (u.scheme == "hone" && u.host == "join") u.getQueryParameter("ref")?.let { network.hone.app.account.AccountStore.setPendingRef(this, it) }
    }

    override fun onResume() {
        super.onResume()
        network.hone.app.station.StationService.start(this)
        if (network.hone.app.sound.SoundSensing.isEnabled(this)) network.hone.app.service.NodeService.start(this)
    }
}
