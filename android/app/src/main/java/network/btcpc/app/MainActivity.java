package network.btcpc.app;

import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;

import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;

import com.getcapacitor.BridgeActivity;

public class MainActivity extends BridgeActivity {

    private static final String TAG = "BTCPCMain";
    private static final int REQ_BLE_PERMS = 101;

    @Override
    public void onCreate(Bundle savedInstanceState) {
        registerPlugin(BTCPCSensorsPlugin.class);
        super.onCreate(savedInstanceState);
        requestBlePermissionsIfNeeded();
    }

    private void requestBlePermissionsIfNeeded() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            String[] needed = {
                android.Manifest.permission.BLUETOOTH_SCAN,
                android.Manifest.permission.BLUETOOTH_CONNECT
            };
            boolean anyMissing = false;
            for (String p : needed) {
                if (ContextCompat.checkSelfPermission(this, p) != PackageManager.PERMISSION_GRANTED) {
                    anyMissing = true;
                    break;
                }
            }
            if (anyMissing) {
                ActivityCompat.requestPermissions(this, needed, REQ_BLE_PERMS);
            }
        }
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
