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
import android.util.Log;

import androidx.annotation.Nullable;
import androidx.core.app.NotificationCompat;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.HashMap;
import java.util.Map;

import fi.iki.elonen.NanoHTTPD;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;

/**
 * StorageService — hosts BTCPC-FS blobs from the phone.
 *
 * On start:
 *  1. Fetches the phone's assigned blob list from /api/storage/phone/assignments
 *  2. Downloads any blobs not already cached in getFilesDir()/btcpc-fs/
 *  3. Serves them via NanoHTTPD on port 7878
 *  4. Periodically POSTs delivery proofs to /api/storage/phone/heartbeat
 *
 * Peers that need a blob can request it from this phone's local IP.
 * The chain rewards delivery based on heartbeat + verified fetch counts.
 */
public class StorageService extends Service {

    private static final String TAG          = "StorageService";
    private static final String CHANNEL_ID   = "btcpc_storage";
    private static final int    NOTIF_ID     = 5;
    private static final int    HTTP_PORT    = 7878;
    private static final long   HEARTBEAT_MS = 60_000;

    private AppPrefs prefs;
    private BlobServer blobServer;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private File blobDir;
    private final OkHttpClient httpClient = new OkHttpClient();
    private int servedCount = 0;
    private long quotaBytes = 1024L * 1024 * 1024;

    // ---------- service lifecycle ----------

    @Override
    public void onCreate() {
        super.onCreate();
        prefs = new AppPrefs(this);
        blobDir = new File(getFilesDir(), "btcpc-fs");
        blobDir.mkdirs();
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        startForeground(NOTIF_ID, buildNotification("Starting…"));

        String account = prefs.getAccount();
        String jwt     = prefs.getJwt();
        quotaBytes = (long) prefs.getStorageQuotaMb() * 1024 * 1024;
        if (account.isEmpty() || jwt.isEmpty()) {
            prefs.setStorageState("Not signed in — set account in Settings");
            updateNotification("Not signed in");
            return START_NOT_STICKY;
        }

        // Start HTTP server
        try {
            blobServer = new BlobServer(HTTP_PORT, blobDir);
            blobServer.start();
            prefs.setStorageState("Serving on port " + HTTP_PORT);
            updateNotification("Serving blobs");
        } catch (Exception e) {
            Log.e(TAG, "Failed to start blob server: " + e.getMessage());
            prefs.setStorageState("Error: " + e.getMessage());
            return START_NOT_STICKY;
        }

        // Fetch initial blob assignments and schedule heartbeat
        fetchAssignments(account, jwt);
        handler.postDelayed(heartbeatRunnable, HEARTBEAT_MS);

        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
        handler.removeCallbacks(heartbeatRunnable);
        if (blobServer != null) blobServer.stop();
        prefs.setStorageState("Stopped");
    }

    @Nullable
    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    // ---------- blob management ----------

    private void fetchAssignments(String account, String jwt) {
        new Thread(() -> {
            try {
                String url = prefs.getApiUrl() + "/api/storage/phone/assignments";
                Request req = new Request.Builder()
                        .url(url)
                        .addHeader("Authorization", "Bearer " + jwt)
                        .get()
                        .build();
                try (Response resp = httpClient.newCall(req).execute()) {
                    if (!resp.isSuccessful() || resp.body() == null) return;
                    JSONObject json = new JSONObject(resp.body().string());
                    JSONArray blobs = json.optJSONArray("blobs");
                    if (blobs == null) {
                        handler.post(() -> {
                            prefs.setStorageState("Hosting (0 blobs assigned)");
                            updateNotification("0 blobs");
                        });
                        return;
                    }
                    int downloaded = 0;
                    for (int i = 0; i < blobs.length(); i++) {
                        JSONObject blob = blobs.getJSONObject(i);
                        String blobId = blob.optString("blob_id");
                        String blobUrl = blob.optString("source_url");
                        if (blobId.isEmpty()) continue;
                        File dest = new File(blobDir, blobId);
                        if (!dest.exists() && !blobUrl.isEmpty()) {
                            // Enforce quota
                            long usedBytes = 0;
                            File[] files = blobDir.listFiles();
                            if (files != null) for (File f : files) usedBytes += f.length();
                            if (usedBytes < quotaBytes) {
                                if (downloadBlob(blobUrl, dest)) downloaded++;
                            }
                        }
                    }
                    final int count = blobs.length();
                    final int dl = downloaded;
                    handler.post(() -> {
                        String s = "Hosting " + count + " blob(s)";
                        if (dl > 0) s += " (+" + dl + " new)";
                        prefs.setStorageState(s);
                        updateNotification(s);
                    });
                }
            } catch (Exception e) {
                Log.w(TAG, "Assignments fetch failed: " + e.getMessage());
                handler.post(() -> prefs.setStorageState("Hosting (assignments pending)"));
            }
        }).start();
    }

    private boolean downloadBlob(String url, File dest) {
        try {
            Request req = new Request.Builder().url(url).get().build();
            try (Response resp = httpClient.newCall(req).execute()) {
                if (!resp.isSuccessful() || resp.body() == null) return false;
                try (InputStream in = resp.body().byteStream();
                     FileOutputStream out = new FileOutputStream(dest)) {
                    byte[] buf = new byte[8192];
                    int n;
                    while ((n = in.read(buf)) != -1) out.write(buf, 0, n);
                }
                return true;
            }
        } catch (Exception e) {
            Log.w(TAG, "Blob download failed: " + e.getMessage());
            return false;
        }
    }

    // ---------- heartbeat ----------

    private final Runnable heartbeatRunnable = new Runnable() {
        @Override
        public void run() {
            if (prefs == null) return;
            sendHeartbeat();
            handler.postDelayed(this, HEARTBEAT_MS);
        }
    };

    private void sendHeartbeat() {
        new Thread(() -> {
            try {
                String url = prefs.getApiUrl() + "/api/storage/phone/heartbeat";
                JSONObject body = new JSONObject();
                body.put("account", prefs.getAccount());
                body.put("port", HTTP_PORT);
                body.put("blob_count", blobDir.listFiles() != null ? blobDir.listFiles().length : 0);
                body.put("served_count", servedCount);
                body.put("quota_mb", quotaBytes / (1024 * 1024));

                okhttp3.RequestBody rb = okhttp3.RequestBody.create(
                        body.toString(),
                        okhttp3.MediaType.get("application/json")
                );
                Request req = new Request.Builder()
                        .url(url)
                        .addHeader("Authorization", "Bearer " + prefs.getJwt())
                        .post(rb)
                        .build();
                try (Response resp = httpClient.newCall(req).execute()) {
                    if (resp.isSuccessful()) {
                        Log.d(TAG, "Heartbeat OK");
                    }
                }
            } catch (Exception e) {
                Log.w(TAG, "Heartbeat failed: " + e.getMessage());
            }
        }).start();
    }

    // ---------- notification ----------

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel ch = new NotificationChannel(
                    CHANNEL_ID, "BTCPC Storage", NotificationManager.IMPORTANCE_LOW);
            ((NotificationManager) getSystemService(NOTIFICATION_SERVICE)).createNotificationChannel(ch);
        }
    }

    private Notification buildNotification(String status) {
        return new NotificationCompat.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_menu_save)
                .setContentTitle("BTCPC Storage")
                .setContentText(status)
                .setOngoing(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .build();
    }

    private void updateNotification(String status) {
        NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (nm != null) nm.notify(NOTIF_ID, buildNotification(status));
    }

    // ---------- NanoHTTPD blob server ----------

    private class BlobServer extends NanoHTTPD {
        private final File root;

        BlobServer(int port, File root) {
            super(port);
            this.root = root;
        }

        @Override
        public Response serve(IHTTPSession session) {
            String uri = session.getUri(); // e.g. /blob/<id>
            if (!uri.startsWith("/blob/")) {
                return newFixedLengthResponse(Response.Status.NOT_FOUND, "text/plain", "Not found");
            }
            String blobId = uri.substring(6).replaceAll("[^a-zA-Z0-9._\\-]", "");
            if (blobId.isEmpty()) {
                return newFixedLengthResponse(Response.Status.BAD_REQUEST, "text/plain", "Bad request");
            }
            File f = new File(root, blobId);
            if (!f.exists() || !f.isFile()) {
                return newFixedLengthResponse(Response.Status.NOT_FOUND, "text/plain", "Blob not found");
            }
            try {
                servedCount++;
                InputStream is = new FileInputStream(f);
                return newChunkedResponse(Response.Status.OK, "application/octet-stream", is);
            } catch (Exception e) {
                return newFixedLengthResponse(Response.Status.INTERNAL_ERROR, "text/plain", e.getMessage());
            }
        }
    }
}
