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
        enableEdgeToEdge()
        setContent {
            HoneTheme {
                HoneApp()
            }
        }
    }
}
