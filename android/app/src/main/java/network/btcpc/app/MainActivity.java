package network.btcpc.app;

import android.content.Intent;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;

import com.getcapacitor.BridgeActivity;

public class MainActivity extends BridgeActivity {

    private static final String TAG = "BTCPCMain";

    @Override
    public void onCreate(Bundle savedInstanceState) {
        registerPlugin(BTCPCSensorsPlugin.class);
        super.onCreate(savedInstanceState);
    }

    @Override
    public void onResume() {
        super.onResume();
        startRelayService();
    }

    @Override
    public void onPause() {
        super.onPause();
        stopRelayService();
    }

    private void startRelayService() {
        try {
            Intent intent = new Intent(this, LocalRelayService.class);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                startForegroundService(intent);
            } else {
                startService(intent);
            }
            Log.i(TAG, "LocalRelayService started");
        } catch (Exception e) {
            Log.e(TAG, "Failed to start LocalRelayService: " + e.getMessage());
        }
    }

    private void stopRelayService() {
        try {
            Intent intent = new Intent(this, LocalRelayService.class);
            stopService(intent);
            Log.i(TAG, "LocalRelayService stopped");
        } catch (Exception e) {
            Log.e(TAG, "Failed to stop LocalRelayService: " + e.getMessage());
        }
    }
}
