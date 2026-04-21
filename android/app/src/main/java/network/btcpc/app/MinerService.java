package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;

import androidx.annotation.Nullable;
import androidx.core.app.NotificationCompat;

import java.io.File;

/**
 * MinerService — on-device inference miner backed by a Rust/candle engine.
 *
 * The Rust native library (libbtcpc_miner.so) handles model download, inference,
 * and proof submission. This service is the Android lifecycle wrapper.
 *
 * Build: cargo-ndk must have produced the .so before assembling the APK.
 * If the .so is absent at class-load time, the service logs the error and
 * marks itself as "unavailable" without crashing.
 */
public class MinerService extends Service {

    private static final String CHANNEL_ID   = "btcpc_miner";
    private static final int    NOTIF_ID     = 4;
    private static final long   STATUS_INTERVAL_MS = 3000;

    private static boolean nativeAvailable = false;

    static {
        try {
            System.loadLibrary("btcpc_miner");
            nativeAvailable = true;
        } catch (UnsatisfiedLinkError e) {
            android.util.Log.w("MinerService", "Rust miner library not found: " + e.getMessage());
        }
    }

    // ---------- JNI bridge (implemented in Rust) ----------
    private static native void nativeStart(String account, String jwt, String apiBase, String modelDir);
    private static native void nativeStop();
    private static native String nativeGetStatus();
    private static native boolean nativeIsRunning();

    // ---------- Android service lifecycle ----------

    private AppPrefs prefs;
    private final Handler handler = new Handler(Looper.getMainLooper());

    private final Runnable statusPoller = new Runnable() {
        @Override
        public void run() {
            if (prefs == null) return;
            String status = nativeAvailable ? nativeGetStatus() : "Native library not available";
            if (status == null) status = "Running";
            prefs.setMinerState(status);
            updateNotification(status);
            handler.postDelayed(this, STATUS_INTERVAL_MS);
        }
    };

    @Override
    public void onCreate() {
        super.onCreate();
        prefs = new AppPrefs(this);
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        startForeground(NOTIF_ID, buildNotification("Starting…"));

        if (!nativeAvailable) {
            prefs.setMinerState("Native library not available — rebuild APK with cargo-ndk");
            updateNotification("Library missing");
            return START_NOT_STICKY;
        }

        String account  = prefs.getAccount();
        String jwt      = prefs.getJwt();
        String apiBase  = prefs.getApiUrl();
        String modelDir = getFilesDir().getAbsolutePath() + "/miner-model";

        new File(modelDir).mkdirs();

        if (account.isEmpty() || jwt.isEmpty()) {
            prefs.setMinerState("Not signed in — set account in Settings");
            updateNotification("Not signed in");
            return START_NOT_STICKY;
        }

        nativeStart(account, jwt, apiBase, modelDir);
        handler.post(statusPoller);
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
        handler.removeCallbacks(statusPoller);
        if (nativeAvailable) nativeStop();
        prefs.setMinerState("Stopped");
    }

    @Nullable
    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    // ---------- notification ----------

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel ch = new NotificationChannel(
                    CHANNEL_ID, "BTCPC Miner", NotificationManager.IMPORTANCE_LOW);
            ch.setDescription("On-device inference mining");
            ((NotificationManager) getSystemService(NOTIFICATION_SERVICE)).createNotificationChannel(ch);
        }
    }

    private Notification buildNotification(String status) {
        return new NotificationCompat.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_menu_upload)
                .setContentTitle("BTCPC Miner")
                .setContentText(status)
                .setOngoing(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .build();
    }

    private void updateNotification(String status) {
        NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (nm != null) nm.notify(NOTIF_ID, buildNotification(status));
    }
}
