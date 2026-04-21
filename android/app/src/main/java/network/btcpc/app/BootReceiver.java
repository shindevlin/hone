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
        if (!Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction())) return;
        Log.i(TAG, "Boot completed — starting BTCPC services");
        Intent relay = new Intent(context, LocalRelayService.class);
        Intent clock = new Intent(context, NativeClockService.class);
        Intent sensors = new Intent(context, NativeSensorService.class);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            context.startForegroundService(relay);
            context.startForegroundService(clock);
            context.startForegroundService(sensors);
        } else {
            context.startService(relay);
            context.startService(clock);
            context.startService(sensors);
        }
    }
}
