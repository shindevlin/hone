package network.hone.app.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Restarts the sensor node after a reboot so a docked phone self-heals without the user
 * reopening the app. Handles both BOOT_COMPLETED and LOCKED_BOOT_COMPLETED (direct-boot).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_LOCKED_BOOT_COMPLETED -> NodeService.start(context.applicationContext)
        }
    }
}
