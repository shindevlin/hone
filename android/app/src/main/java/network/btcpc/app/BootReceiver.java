package network.btcpc.app;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.util.Log;

public class BootReceiver extends BroadcastReceiver {

    private static final String TAG = "BTCPCBoot";

    @Override
    public void onReceive(Context context, Intent intent) {
        String action = intent.getAction();
        if (!Intent.ACTION_BOOT_COMPLETED.equals(action)
                && !Intent.ACTION_MY_PACKAGE_REPLACED.equals(action)
                && !"android.intent.action.QUICKBOOT_POWERON".equals(action)) {
            return;
        }

        Log.i(TAG, "Startup trigger: " + action + " — starting BTCPC services");

        AppPrefs prefs = new AppPrefs(context);

        // Relay always starts — it is the core hardware bridge
        startService(context, LocalRelayService.class);

        if (prefs.isClockEnabled()) {
            startService(context, NativeClockService.class);
        }
        if (prefs.isSensorsEnabled()) {
            startService(context, NativeSensorService.class);
        }
        if (prefs.isMinerEnabled() && !prefs.getAccount().isEmpty()) {
            startService(context, MinerService.class);
        }
        if (prefs.isStorageEnabled() && !prefs.getAccount().isEmpty()) {
            startService(context, StorageService.class);
        }
    }

    private void startService(Context context, Class<?> serviceClass) {
        try {
            Intent intent = new Intent(context, serviceClass);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent);
            } else {
                context.startService(intent);
            }
        } catch (Exception e) {
            Log.w(TAG, "Could not start " + serviceClass.getSimpleName() + ": " + e.getMessage());
        }
    }
}
