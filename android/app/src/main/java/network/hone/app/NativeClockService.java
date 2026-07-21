package network.hone.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;

import androidx.core.app.NotificationCompat;

/**
 * NativeClockService — libp2p-based clock consensus via the Rust hone_miner library.
 *
 * Replaces the old WebSocket-relay approach with direct P2P epoch seals
 * submitted to the connected hone node API.
 */
public class NativeClockService extends Service {

    private static final String CHANNEL_ID   = "hone_clock";
    private static final int    NOTIF_ID     = 9430;
    private static final long   STATUS_INTERVAL_MS = 5000;
    private static final long   MAINNET_GENESIS_TS = 1777633200000L;

    private static boolean nativeAvailable = false;

    static {
        try {
            System.loadLibrary("hone_miner");
            nativeAvailable = true;
        } catch (UnsatisfiedLinkError e) {
            android.util.Log.w("HONEClock", "Rust library not found: " + e.getMessage());
        }
    }

    private static final String DEFAULT_BOOTSTRAP_PEERS =
            "/dns4/p2p.hone.net/tcp/443/wss/p2p/12D3KooWLLim8VHdYP1Ev5kiAi1ewC2Sj6GKcV3qWDu4E9sNTy5N";

    // ---------- JNI bridge (Rust) ----------
    private static native void nativeStart(String account, String apiBase, String chainId,
                                           long genesisTs, String postingKey,
                                           String bootstrapPeers);
    private static native void nativeStop();
    private static native String nativeGetStatus();

    // ---------- Android lifecycle ----------

    private AppPrefs prefs;
    private final Handler handler = new Handler(Looper.getMainLooper());

    private final Runnable statusPoller = new Runnable() {
        @Override public void run() {
            if (prefs == null) return;
            String status = nativeAvailable ? nativeGetStatus() : "Native library not available";
            if (status == null) status = "Running";
            prefs.setClockState(status);
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
        try {
            startForeground(NOTIF_ID, buildNotification("Starting…"));
        } catch (Exception e) {
            stopSelf();
            return START_NOT_STICKY;
        }

        if (!nativeAvailable) {
            updateNotification("Library missing — rebuild APK with cargo-ndk");
            return START_NOT_STICKY;
        }

        String account    = prefs.getAccount();
        String apiBase    = prefs.getEffectiveApiUrl();
        String postingKey = prefs.getPostingKey();
        String chainId    = prefs.getChainId();
        long   genesisTs  = prefs.getGenesisTs(MAINNET_GENESIS_TS);

        if (account.isEmpty()) {
            updateNotification("Not signed in");
            return START_NOT_STICKY;
        }

        String bootstrapPeers = prefs.getBootstrapPeers(DEFAULT_BOOTSTRAP_PEERS);
        nativeStart(account, apiBase, chainId, genesisTs, postingKey, bootstrapPeers);
        handler.post(statusPoller);
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
        handler.removeCallbacks(statusPoller);
        if (nativeAvailable) nativeStop();
        if (prefs != null) prefs.setClockState("Stopped");
    }

    @Override public IBinder onBind(Intent intent) { return null; }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel ch = new NotificationChannel(
                    CHANNEL_ID, "HONE Clock Peer", NotificationManager.IMPORTANCE_LOW);
            ((NotificationManager) getSystemService(NOTIFICATION_SERVICE))
                    .createNotificationChannel(ch);
        }
    }

    private Notification buildNotification(String text) {
        Intent intent = new Intent(this, MainActivity.class);
        PendingIntent pi = PendingIntent.getActivity(this, 0, intent,
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.M
                        ? PendingIntent.FLAG_IMMUTABLE : 0);
        return new NotificationCompat.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_menu_recent_history)
                .setContentTitle("HONE Clock Peer")
                .setContentText(text)
                .setContentIntent(pi)
                .setOngoing(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .build();
    }

    private void updateNotification(String text) {
        NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (nm != null) nm.notify(NOTIF_ID, buildNotification(text));
    }
}
