package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

import androidx.core.app.NotificationCompat;

import org.json.JSONException;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.Map;

import fi.iki.elonen.NanoHTTPD;

public class LocalRelayService extends Service {

    private static final String TAG = "BTCPCRelay";
    private static final int NOTIFICATION_ID = 9420;
    private static final String CHANNEL_ID = "btcpc_relay";
    private static final int RELAY_PORT = 6942;
    private static final String UPSTREAM_BASE = "https://btcpc.net/api";

    private RelayServer server;

    @Override
    public void onCreate() {
        super.onCreate();
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Notification notification = new NotificationCompat.Builder(this, CHANNEL_ID)
                .setContentTitle("BTCPC Relay")
                .setContentText("Accepting sensor data on port 6942")
                .setSmallIcon(android.R.drawable.ic_menu_upload)
                .setOngoing(true)
                .build();

        startForeground(NOTIFICATION_ID, notification);

        if (server == null) {
            server = new RelayServer(RELAY_PORT);
            try {
                server.start();
                Log.i(TAG, "Relay server started on port " + RELAY_PORT);
            } catch (IOException e) {
                Log.e(TAG, "Failed to start relay server: " + e.getMessage());
                stopSelf();
            }
        }

        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        if (server != null) {
            server.stop();
            server = null;
            Log.i(TAG, "Relay server stopped");
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "BTCPC Relay",
                    NotificationManager.IMPORTANCE_LOW
            );
            channel.setDescription("LAN relay for sensor data from Flipper Zero, browsers, and other devices");
            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }

    // -----------------------------------------------------------------------
    // NanoHTTPD inner server
    // -----------------------------------------------------------------------
    private static class RelayServer extends NanoHTTPD {

        RelayServer(int port) {
            super(port);
        }

        @Override
        public Response serve(IHTTPSession session) {
            String uri = session.getUri();
            Method method = session.getMethod();

            Log.d(TAG, method + " " + uri);

            // Only handle POST /sensors/:id/readings
            if (!Method.POST.equals(method)) {
                return newFixedLengthResponse(Response.Status.METHOD_NOT_ALLOWED,
                        "application/json", "{\"error\":\"Method not allowed\"}");
            }

            // Parse /sensors/:id/readings
            String sensorId = parseSensorId(uri);
            if (sensorId == null) {
                return newFixedLengthResponse(Response.Status.NOT_FOUND,
                        "application/json", "{\"error\":\"Not found. Use POST /sensors/:id/readings\"}");
            }

            // Read body
            String body;
            try {
                Map<String, String> files = new java.util.HashMap<>();
                session.parseBody(files);
                body = files.get("postData");
                if (body == null || body.isEmpty()) {
                    // Fallback: read directly from input stream
                    int contentLength = 0;
                    String cl = session.getHeaders().get("content-length");
                    if (cl != null) contentLength = Integer.parseInt(cl.trim());
                    if (contentLength > 0) {
                        byte[] buf = new byte[contentLength];
                        int read = session.getInputStream().read(buf, 0, contentLength);
                        body = new String(buf, 0, read, "UTF-8");
                    } else {
                        body = "{}";
                    }
                }
            } catch (Exception e) {
                Log.e(TAG, "Failed to read body: " + e.getMessage());
                return newFixedLengthResponse(Response.Status.BAD_REQUEST,
                        "application/json", "{\"error\":\"Failed to read request body\"}");
            }

            // Validate JSON
            JSONObject payload;
            try {
                payload = new JSONObject(body);
            } catch (JSONException e) {
                return newFixedLengthResponse(Response.Status.BAD_REQUEST,
                        "application/json", "{\"error\":\"Invalid JSON\"}");
            }

            // Validate required fields
            if (!payload.has("value")) {
                return newFixedLengthResponse(Response.Status.BAD_REQUEST,
                        "application/json", "{\"error\":\"'value' field is required\"}");
            }

            Object valueObj = payload.opt("value");
            if (!(valueObj instanceof Number)) {
                return newFixedLengthResponse(Response.Status.BAD_REQUEST,
                        "application/json", "{\"error\":\"'value' must be numeric\"}");
            }

            Log.i(TAG, "Relaying reading for sensor " + sensorId + ": " + body);

            // Forward to upstream
            String upstreamUrl = UPSTREAM_BASE + "/sensors/" + sensorId + "/readings";
            try {
                String response = postToUpstream(upstreamUrl, body);
                Log.i(TAG, "Upstream response for " + sensorId + ": " + response);
                return newFixedLengthResponse(Response.Status.OK, "application/json", response);
            } catch (IOException e) {
                Log.e(TAG, "Upstream request failed: " + e.getMessage());
                return newFixedLengthResponse(Response.Status.INTERNAL_ERROR,
                        "application/json", "{\"error\":\"Upstream unreachable\",\"detail\":\"" + e.getMessage() + "\"}");
            }
        }

        /**
         * Parse sensor ID from URI of the form /sensors/:id/readings
         */
        private String parseSensorId(String uri) {
            if (uri == null) return null;
            // Strip query string
            int q = uri.indexOf('?');
            if (q >= 0) uri = uri.substring(0, q);
            // Match /sensors/<id>/readings
            String[] parts = uri.split("/");
            // Expected: ["", "sensors", "<id>", "readings"]
            if (parts.length == 4
                    && "sensors".equals(parts[1])
                    && "readings".equals(parts[3])
                    && !parts[2].isEmpty()) {
                return parts[2];
            }
            return null;
        }

        /**
         * HTTP POST to upstream URL with JSON body. Returns response body as string.
         */
        private String postToUpstream(String urlStr, String jsonBody) throws IOException {
            URL url = new URL(urlStr);
            HttpURLConnection conn = (HttpURLConnection) url.openConnection();
            conn.setRequestMethod("POST");
            conn.setRequestProperty("Content-Type", "application/json");
            conn.setRequestProperty("Accept", "application/json");
            conn.setRequestProperty("User-Agent", "BTCPC-Android-Relay/1.0");
            conn.setConnectTimeout(10_000);
            conn.setReadTimeout(15_000);
            conn.setDoOutput(true);

            try (OutputStreamWriter writer = new OutputStreamWriter(conn.getOutputStream(), "UTF-8")) {
                writer.write(jsonBody);
                writer.flush();
            }

            int status = conn.getResponseCode();
            java.io.InputStream is = (status >= 200 && status < 300)
                    ? conn.getInputStream()
                    : conn.getErrorStream();

            StringBuilder sb = new StringBuilder();
            if (is != null) {
                try (BufferedReader reader = new BufferedReader(new InputStreamReader(is, "UTF-8"))) {
                    String line;
                    while ((line = reader.readLine()) != null) {
                        sb.append(line);
                    }
                }
            }

            return sb.length() > 0 ? sb.toString() : "{\"status\":" + status + "}";
        }
    }
}
