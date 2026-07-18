package network.hone.app;

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

/**
 * MinerService — full hone micronode backed by libhone_miner.so (Rust).
 *
 * The Rust library runs a complete standalone node:
 *   • libp2p P2P networking (TCP, gossipsub, Kademlia DHT)
 *   • Clock consensus (epoch seals, quorum tracking)
 *   • Block mining (SHA-256 work)
 *   • On-device inference (candle, qwen2.5-0.5b downloaded on first run)
 *   • Sled embedded store — no remote API dependency
 */
public class MinerService extends Service {

    private static final String CHANNEL_ID         = "hone_miner";
    private static final int    NOTIF_ID           = 4;
    private static final long   STATUS_INTERVAL_MS = 3_000;
    private static final long   MAINNET_GENESIS_TS = 1777633200000L;
    private static final int    DEFAULT_P2P_PORT   = 6942;

    private static boolean nativeAvailable = false;

    static {
        try {
            System.loadLibrary("hone_miner");
            nativeAvailable = true;
        } catch (UnsatisfiedLinkError e) {
            android.util.Log.w("HONEMiner", "Rust library not available: " + e.getMessage());
        }
    }

    // ---------- JNI bridge ----------

    private static native void nativeStart(
            String  account,
            String  postingKey,
            String  chainId,
            long    genesisTs,
            String  dataDir,
            String  modelId,
            String  modelDir,
            String  bootstrapPeers,
            long    p2pPort,
            boolean isMiner,
            boolean isClock
    );
    private static native void    nativeStop();
    private static native String  nativeGetStatus();
    private static native boolean nativeIsRunning();
    static         native long    nativeGetBalance(String account);
    static         native long    nativeGetEpoch();

    // ---------- Android service lifecycle ----------

    private AppPrefs prefs;
    private final Handler handler = new Handler(Looper.getMainLooper());

    private final Runnable statusPoller = new Runnable() {
        @Override public void run() {
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
        try {
            startForeground(NOTIF_ID, buildNotification("Starting…"));
        } catch (Exception e) {
            stopSelf();
            return START_NOT_STICKY;
        }

        if (!nativeAvailable) {
            updateNotification("Native library missing — rebuild APK with cargo-ndk");
            return START_NOT_STICKY;
        }

        String account    = prefs.getAccount();
        String postingKey = prefs.getPostingKey();
        String chainId    = prefs.getChainId();
        long   genesisTs  = prefs.getGenesisTs(MAINNET_GENESIS_TS);
        String modelId    = prefs.getMinerModel();

        if (account.isEmpty()) {
            updateNotification("Not signed in — configure account in Settings");
            return START_NOT_STICKY;
        }

        // Model storage — external survives reinstalls; fallback to internal.
        java.io.File extBase = getExternalFilesDir("miner-models");
        if (extBase == null) extBase = new java.io.File(getFilesDir(), "miner-models");
        String modelDir = new java.io.File(extBase, modelId).getAbsolutePath();
        new java.io.File(modelDir).mkdirs();

        // Node data dir (sled DB + P2P identity key).
        String dataDir = new java.io.File(getFilesDir(), "hone-node").getAbsolutePath();
        new java.io.File(dataDir).mkdirs();

        // Bootstrap peers from prefs or hardcoded fallback.
        // Includes genesis node LAN + Tailscale IPs as well as DNS seeds.
        String bootstrapPeers = prefs.getBootstrapPeers(
                "/dns4/p2p.hone.net/tcp/443/wss/p2p/12D3KooWLLim8VHdYP1Ev5kiAi1ewC2Sj6GKcV3qWDu4E9sNTy5N");

        nativeStart(
                account, postingKey, chainId, genesisTs,
                dataDir, modelId, modelDir, bootstrapPeers,
                DEFAULT_P2P_PORT,
                prefs.isMinerEnabled(),
                prefs.isClockEnabled()
        );

        handler.post(statusPoller);
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
        handler.removeCallbacks(statusPoller);
        if (nativeAvailable) nativeStop();
        if (prefs != null) prefs.setMinerState("Stopped");
    }

    @Nullable @Override
    public IBinder onBind(Intent intent) { return null; }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel ch = new NotificationChannel(
                    CHANNEL_ID, "HONE Node", NotificationManager.IMPORTANCE_LOW);
            ch.setDescription("Full hone micronode — mining, clock, inference");
            ((NotificationManager) getSystemService(NOTIFICATION_SERVICE))
                    .createNotificationChannel(ch);
        }
    }

    private Notification buildNotification(String status) {
        return new NotificationCompat.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_menu_upload)
                .setContentTitle("HONE Node")
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
