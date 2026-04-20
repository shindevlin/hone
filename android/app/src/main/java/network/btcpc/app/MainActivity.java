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
    private static final int REQ_SENSOR_PERMS = 102;

    @Override
    public void onCreate(Bundle savedInstanceState) {
        registerPlugin(BTCPCSensorsPlugin.class);
        super.onCreate(savedInstanceState);
        // Start the relay service once — it's a foreground service and persists independently
        startRelayService();
        // Request BLE permissions after service is running so the permission dialog
        // doesn't fire onPause before startForeground() can be called
        requestBlePermissionsIfNeeded();
        requestSensorPermissionsIfNeeded();
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

    private void requestSensorPermissionsIfNeeded() {
        String[] needed = { android.Manifest.permission.RECORD_AUDIO };
        boolean anyMissing = false;
        for (String p : needed) {
            if (ContextCompat.checkSelfPermission(this, p) != PackageManager.PERMISSION_GRANTED) {
                anyMissing = true;
                break;
            }
        }
        if (anyMissing) {
            ActivityCompat.requestPermissions(this, needed, REQ_SENSOR_PERMS);
        }
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
}
