package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

import androidx.core.app.NotificationCompat;

import org.json.JSONObject;

import java.io.IOException;
import java.util.UUID;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;
import okhttp3.WebSocket;
import okhttp3.WebSocketListener;
import okio.ByteString;

public class NativeClockService extends Service {

    static final String DEFAULT_RELAY_URL = "wss://btcpc-relay.shindevlin.workers.dev/ws";

    private static final String TAG = "BTCPCClock";
    private static final String CHANNEL_ID = "btcpc_clock";
    private static final int NOTIFICATION_ID = 9430;
    private static final String PREFS = "btcpc_native_state";
    private static final long EPOCH_ZERO_MS = 1776236400000L;

    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    private final OkHttpClient client = new OkHttpClient.Builder().retryOnConnectionFailure(true).build();

    private volatile WebSocket socket;
    private volatile boolean running;
    private volatile boolean heartbeatStarted;
    private volatile String nodeId;
    private volatile String account;
    private volatile String relayUrl;
    private volatile String apiBase;
    private volatile String jwt;
    private volatile long lastHeartbeatMs;
    private static final MediaType JSON_MEDIA = MediaType.get("application/json; charset=utf-8");

    @Override
    public void onCreate() {
        super.onCreate();
        createNotificationChannel();
        loadSettings();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        loadSettings();
        running = true;

        Notification notification = buildNotification("Clock peer: connecting");
        startForeground(NOTIFICATION_ID, notification);

        connect();
        if (!heartbeatStarted) {
            heartbeatStarted = true;
            scheduler.scheduleAtFixedRate(this::sendHeartbeatSafe, 5, 30, TimeUnit.SECONDS);
        }
        persistState("Clock peer: running");
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        running = false;
        heartbeatStarted = false;
        scheduler.shutdownNow();
        if (socket != null) {
            socket.close(1000, "service stopped");
            socket = null;
        }
        persistState("Clock peer: stopped");
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private void loadSettings() {
        SharedPreferences prefs = getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        account  = prefs.getString("account", "btcpc-phone");
        relayUrl = prefs.getString("relayUrl", DEFAULT_RELAY_URL);
        apiBase  = prefs.getString("apiUrl", "https://btcpc.net");
        jwt      = prefs.getString("jwt", "");
        nodeId = prefs.getString("clockNodeId", null);
        if (nodeId == null || nodeId.isEmpty()) {
            nodeId = UUID.randomUUID().toString().replace("-", "");
            prefs.edit().putString("clockNodeId", nodeId).apply();
        }
    }

    private void connect() {
        try {
            if (socket != null) {
                socket.close(1000, "reconnect");
                socket = null;
            }
            Request request = new Request.Builder().url(relayUrl).build();
            socket = client.newWebSocket(request, new WebSocketListener() {
                @Override
                public void onOpen(WebSocket webSocket, Response response) {
                    persistState("Clock peer: connected");
                    sendHandshake(webSocket);
                }

                @Override
                public void onMessage(WebSocket webSocket, String text) {
                    handleIncoming(text);
                }

                @Override
                public void onMessage(WebSocket webSocket, ByteString bytes) {
                    handleIncoming(bytes.utf8());
                }

                @Override
                public void onFailure(WebSocket webSocket, Throwable t, Response response) {
                    Log.w(TAG, "WebSocket failure: " + t.getMessage());
                    persistState("Clock peer: reconnecting");
                    if (running) {
                        scheduler.schedule(NativeClockService.this::connect, 10, TimeUnit.SECONDS);
                    }
                }

                @Override
                public void onClosed(WebSocket webSocket, int code, String reason) {
                    persistState("Clock peer: disconnected");
                }
            });
        } catch (Exception e) {
            Log.e(TAG, "Connect failed: " + e.getMessage());
            persistState("Clock peer: connect failed");
            if (running) {
                scheduler.schedule(this::connect, 10, TimeUnit.SECONDS);
            }
        }
    }

    private void sendHandshake(WebSocket webSocket) {
        try {
            JSONObject payload = new JSONObject();
            payload.put("chainHeight", getCurrentEpoch());
            payload.put("version", "native-android-1.0");
            payload.put("peerCount", 1);
            payload.put("public_address", null);

            JSONObject msg = buildMessage("HANDSHAKE", payload);
            webSocket.send(msg.toString());
            persistState("Clock peer: handshake sent");
            sendHeartbeat(webSocket);
        } catch (Exception e) {
            Log.e(TAG, "Handshake failed: " + e.getMessage());
        }
    }

    private void sendHeartbeatSafe() {
        try {
            WebSocket current = socket;
            if (current != null) {
                sendHeartbeat(current);
            } else if (running) {
                connect();
            }
        } catch (Exception e) {
            Log.w(TAG, "Heartbeat error: " + e.getMessage());
        }
    }

    private void sendHeartbeat(WebSocket webSocket) {
        try {
            JSONObject payload = new JSONObject();
            payload.put("account", account);
            payload.put("epoch_number", getCurrentEpoch());
            payload.put("timestamp", System.currentTimeMillis());
            payload.put("source", "android");
            JSONObject msg = buildMessage("CLOCK_HEARTBEAT", payload);
            if (webSocket.send(msg.toString())) {
                lastHeartbeatMs = System.currentTimeMillis();
                persistState("Clock peer: heartbeat " + account);
            }
        } catch (Exception e) {
            Log.w(TAG, "Heartbeat send failed: " + e.getMessage());
        }
        // Also POST to chain REST endpoint so the node records credit even if
        // the relay doesn't forward the WebSocket message to the P2P network.
        postChainHeartbeat();
    }

    private void postChainHeartbeat() {
        if (account == null || account.isEmpty() || "btcpc-phone".equals(account)) return;
        try {
            JSONObject body = new JSONObject();
            body.put("account", account);
            body.put("client_id", nodeId);
            Request.Builder rb = new Request.Builder()
                    .url(apiBase + "/public/clock-heartbeat")
                    .post(RequestBody.create(body.toString(), JSON_MEDIA));
            if (jwt != null && !jwt.isEmpty()) {
                rb.addHeader("Authorization", "Bearer " + jwt);
            }
            try (Response resp = client.newCall(rb.build()).execute()) {
                if (resp.isSuccessful()) {
                    persistState("Clock peer: on-chain ✓ epoch " + getCurrentEpoch());
                }
            }
        } catch (Exception e) {
            Log.w(TAG, "Chain heartbeat failed: " + e.getMessage());
        }
    }

    private void handleIncoming(String raw) {
        try {
            JSONObject msg = new JSONObject(raw);
            String type = msg.optString("type", "");
            JSONObject data = msg.optJSONObject("data");
            if ("EPOCH_START".equals(type) && data != null) {
                persistState("Epoch start " + data.optLong("epoch_number", -1));
            } else if ("EPOCH_END".equals(type) && data != null) {
                persistState("Epoch end " + data.optLong("epoch_number", -1));
            } else if ("BLOCK".equals(type) && data != null) {
                persistState("Block " + data.optLong("epoch_number", -1));
            }
        } catch (Exception e) {
            Log.w(TAG, "Message parse failed: " + e.getMessage());
        }
    }

    private JSONObject buildMessage(String type, JSONObject data) throws Exception {
        JSONObject msg = new JSONObject();
        msg.put("id", nodeId + "-" + System.currentTimeMillis());
        msg.put("type", type);
        msg.put("data", data);
        msg.put("timestamp", System.currentTimeMillis());
        msg.put("nodeId", nodeId);
        return msg;
    }

    private long getCurrentEpoch() {
        long epoch = (System.currentTimeMillis() - EPOCH_ZERO_MS) / 30000L;
        return Math.max(epoch, 0L);
    }

    private Notification buildNotification(String text) {
        Intent intent = new Intent(this, MainActivity.class);
        PendingIntent pendingIntent = PendingIntent.getActivity(
                this,
                0,
                intent,
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.M
                        ? PendingIntent.FLAG_IMMUTABLE
                        : 0
        );

        return new NotificationCompat.Builder(this, CHANNEL_ID)
                .setContentTitle("BTCPC Clock Peer")
                .setContentText(text)
                .setSmallIcon(android.R.drawable.ic_menu_recent_history)
                .setContentIntent(pendingIntent)
                .setOngoing(true)
                .build();
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "BTCPC Clock Peer",
                    NotificationManager.IMPORTANCE_LOW
            );
            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }

    private void persistState(String state) {
        SharedPreferences prefs = getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        prefs.edit()
                .putString("clock_state", state)
                .putString("last_event", state)
                .putLong("clock_last_heartbeat_ms", lastHeartbeatMs)
                .apply();
    }
}
