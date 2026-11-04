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
        enableEdgeToEdge()
        setContent {
            HoneTheme {
                HoneApp()
            }
        }
    }

    override fun onResume() {
        super.onResume()
        if (network.hone.app.sound.SoundSensing.isEnabled(this)) network.hone.app.service.NodeService.start(this)
    }
}
