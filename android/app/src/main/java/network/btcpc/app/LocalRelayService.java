package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothGatt;
import android.bluetooth.BluetoothGattCallback;
import android.bluetooth.BluetoothGattCharacteristic;
import android.bluetooth.BluetoothGattDescriptor;
import android.bluetooth.BluetoothGattService;
import android.bluetooth.BluetoothManager;
import android.bluetooth.BluetoothProfile;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanFilter;
import android.bluetooth.le.ScanResult;
import android.bluetooth.le.ScanSettings;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.hardware.usb.UsbConstants;
import android.hardware.usb.UsbDevice;
import android.hardware.usb.UsbDeviceConnection;
import android.hardware.usb.UsbEndpoint;
import android.hardware.usb.UsbInterface;
import android.hardware.usb.UsbManager;
import android.os.Build;
import android.os.IBinder;
import android.os.ParcelUuid;
import android.util.Log;

import androidx.core.app.NotificationCompat;

import org.json.JSONException;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

import fi.iki.elonen.NanoHTTPD;

/**
 * LocalRelayService — Flipper Zero sensor relay for BTCPC.
 *
 * All three connection methods run concurrently. Whichever delivers a reading first wins —
 * they all funnel into the same forwardReading() method.
 *
 *   1. WiFi/HTTP — NanoHTTPD server on port 6942.
 *                  Accepts POST /sensors/:id/readings (JSON body).
 *                  Works with Flipper WiFi dev board or any device on the same LAN.
 *   2. USB OTG   — Flipper Zero appears as CDC ACM serial (VID 0x0483, PID 0x5740).
 *                  Android USB Host API reads lines from bulk endpoint.
 *   3. BLE UART  — Flipper Zero UART BLE Bridge app advertises Nordic UART Service.
 *                  Android connects as GATT client, subscribes to TX characteristic.
 *
 * Each method parses JSON: {"id":"sensor_id","value":23.5,"unit":"C"}
 * and forwards to the BTCPC sensor readings API.
 *
 * API target: local node on localhost:4242 with fallback to btcpc.net.
 */
public class LocalRelayService extends Service {

    private static final String TAG = "BTCPCRelay";
    private static final int NOTIFICATION_ID = 9420;
    private static final String CHANNEL_ID = "btcpc_relay";

    // Flipper Zero USB identifiers (STM32 CDC ACM)
    private static final int FLIPPER_VID = 0x0483;
    private static final int FLIPPER_PID = 0x5740;

    // Nordic UART Service UUIDs (used by Flipper BLE UART Bridge app)
    private static final UUID NUS_SERVICE_UUID =
            UUID.fromString("6E400001-B5A3-F393-E0A9-E50E24DCCA9E");
    private static final UUID NUS_TX_CHAR_UUID =           // Flipper → phone (notify)
            UUID.fromString("6E400003-B5A3-F393-E0A9-E50E24DCCA9E");
    private static final UUID CLIENT_CHARACTERISTIC_CONFIG =
            UUID.fromString("00002902-0000-1000-8000-00805f9b34fb");

    // API endpoints — local node first, remote fallback
    private static final String LOCAL_API_BASE  = "http://localhost:4242/api";
    private static final String REMOTE_API_BASE = "https://btcpc.net/api";

    private static final String ACTION_USB_PERMISSION = "network.btcpc.app.USB_PERMISSION";

    // WiFi HTTP server (NanoHTTPD) — Method 1
    private static final int WIFI_HTTP_PORT = 6942;
    private FlipperHttpServer wifiHttpServer;

    private UsbManager usbManager;
    private UsbReaderThread usbReaderThread;

    private BluetoothLeScanner bleScanner;
    private BluetoothGatt bleGatt;
    private final ScanCallback bleScanCallback = new FlipperBleScanCallback();
    private boolean bleConnected = false;

    private final ExecutorService httpExecutor = Executors.newCachedThreadPool();

    // -----------------------------------------------------------------------
    // BroadcastReceiver — USB attach + permission grant
    // -----------------------------------------------------------------------
    private final BroadcastReceiver usbReceiver = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            String action = intent.getAction();
            if (UsbManager.ACTION_USB_DEVICE_ATTACHED.equals(action)) {
                UsbDevice device = intent.getParcelableExtra(UsbManager.EXTRA_DEVICE);
                if (isFlipperZero(device)) {
                    Log.i(TAG, "Flipper Zero attached via USB");
                    requestUsbPermission(device);
                }
            } else if (ACTION_USB_PERMISSION.equals(action)) {
                UsbDevice device = intent.getParcelableExtra(UsbManager.EXTRA_DEVICE);
                boolean granted = intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false);
                if (granted && device != null) {
                    Log.i(TAG, "USB permission granted — opening Flipper serial");
                    startUsbReader(device);
                } else {
                    Log.w(TAG, "USB permission denied for " + (device != null ? device.getDeviceName() : "null"));
                }
            }
        }
    };

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    @Override
    public void onCreate() {
        super.onCreate();
        usbManager = (UsbManager) getSystemService(Context.USB_SERVICE);
        createNotificationChannel();

        // Register USB events
        IntentFilter filter = new IntentFilter();
        filter.addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED);
        filter.addAction(ACTION_USB_PERMISSION);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(usbReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(usbReceiver, filter);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Notification notification = new NotificationCompat.Builder(this, CHANNEL_ID)
                .setContentTitle("BTCPC Relay")
                .setContentText("BTCPC Relay — WiFi · USB · Bluetooth")
                .setSmallIcon(android.R.drawable.ic_menu_upload)
                .setOngoing(true)
                .build();

        startForeground(NOTIFICATION_ID, notification);

        // Method 1: Start WiFi HTTP server (NanoHTTPD on port 6942)
        startWifiServer();

        // Method 2: Check for already-attached Flipper Zero (USB OTG)
        probeAttachedUsb();

        // Method 3: Start BLE scan for Flipper UART bridge
        startBleScan();

        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        stopWifiServer();
        try { unregisterReceiver(usbReceiver); } catch (Exception ignored) {}
        stopUsbReader();
        stopBleScan();
        if (bleGatt != null) {
            bleGatt.close();
            bleGatt = null;
        }
        httpExecutor.shutdownNow();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    // -----------------------------------------------------------------------
    // WiFi / HTTP (NanoHTTPD) — Method 1
    // -----------------------------------------------------------------------

    private void startWifiServer() {
        try {
            wifiHttpServer = new FlipperHttpServer(WIFI_HTTP_PORT);
            wifiHttpServer.start();
            Log.i(TAG, "WiFi HTTP server started on port " + WIFI_HTTP_PORT);
        } catch (IOException e) {
            Log.e(TAG, "Failed to start WiFi HTTP server: " + e.getMessage());
        }
    }

    private void stopWifiServer() {
        if (wifiHttpServer != null) {
            wifiHttpServer.stop();
            wifiHttpServer = null;
            Log.i(TAG, "WiFi HTTP server stopped");
        }
    }

    /**
     * NanoHTTPD server that accepts POST /sensors/:id/readings with a JSON body.
     * Funnels readings into the same handleSensorLine / forwardReading pipeline.
     */
    private class FlipperHttpServer extends NanoHTTPD {

        FlipperHttpServer(int port) {
            super(port);
        }

        @Override
        public Response serve(IHTTPSession session) {
            String uri = session.getUri();           // e.g. /sensors/temp01/readings
            Method method = session.getMethod();

            // Only handle POST /sensors/:id/readings
            if (!Method.POST.equals(method) || !uri.matches("/sensors/[^/]+/readings")) {
                return newFixedLengthResponse(Response.Status.NOT_FOUND,
                        "application/json", "{\"error\":\"not found\"}");
            }

            // Extract sensor id from path
            String[] parts = uri.split("/");
            // parts: ["", "sensors", "<id>", "readings"]
            if (parts.length < 4) {
                return newFixedLengthResponse(Response.Status.BAD_REQUEST,
                        "application/json", "{\"error\":\"bad path\"}");
            }
            String sensorId = parts[2];

            // Read body
            try {
                Map<String, String> bodyMap = new java.util.HashMap<>();
                session.parseBody(bodyMap);
                String body = bodyMap.containsKey("postData") ? bodyMap.get("postData") : "";
                if (body == null || body.isEmpty()) {
                    return newFixedLengthResponse(Response.Status.BAD_REQUEST,
                            "application/json", "{\"error\":\"empty body\"}");
                }

                // Ensure body has "id" field — inject it if the Flipper sent a minimal payload
                JSONObject json = new JSONObject(body);
                if (!json.has("id")) {
                    json.put("id", sensorId);
                }

                Log.d(TAG, "[WiFi] received reading for " + sensorId);
                final String finalBody = json.toString();
                final String finalId = sensorId;
                httpExecutor.execute(() -> forwardReading(finalId, finalBody, "WiFi"));

                return newFixedLengthResponse(Response.Status.OK,
                        "application/json", "{\"status\":\"queued\"}");

            } catch (IOException | NanoHTTPD.ResponseException | JSONException e) {
                Log.e(TAG, "[WiFi] Error parsing request: " + e.getMessage());
                return newFixedLengthResponse(Response.Status.INTERNAL_ERROR,
                        "application/json", "{\"error\":\"" + e.getMessage() + "\"}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // USB OTG — Method 2
    // -----------------------------------------------------------------------

    private boolean isFlipperZero(UsbDevice device) {
        return device != null
                && device.getVendorId()  == FLIPPER_VID
                && device.getProductId() == FLIPPER_PID;
    }

    /**
     * Check already-attached devices at service start (e.g. cable plugged before app launch).
     */
    private void probeAttachedUsb() {
        if (usbManager == null) return;
        for (UsbDevice device : usbManager.getDeviceList().values()) {
            if (isFlipperZero(device)) {
                Log.i(TAG, "Flipper Zero already attached — requesting permission");
                requestUsbPermission(device);
                return;
            }
        }
    }

    private void requestUsbPermission(UsbDevice device) {
        if (usbManager.hasPermission(device)) {
            startUsbReader(device);
            return;
        }
        Intent permIntent = new Intent(ACTION_USB_PERMISSION);
        permIntent.setPackage(getPackageName());
        int flags = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                ? PendingIntent.FLAG_MUTABLE
                : 0;
        PendingIntent pi = PendingIntent.getBroadcast(this, 0, permIntent, flags);
        usbManager.requestPermission(device, pi);
    }

    private void startUsbReader(UsbDevice device) {
        stopUsbReader();
        UsbDeviceConnection connection = usbManager.openDevice(device);
        if (connection == null) {
            Log.e(TAG, "Failed to open USB device");
            return;
        }
        // Find CDC ACM bulk-IN endpoint
        UsbEndpoint bulkIn = findBulkInEndpoint(device, connection);
        if (bulkIn == null) {
            Log.e(TAG, "No bulk-IN endpoint found on Flipper Zero");
            connection.close();
            return;
        }
        usbReaderThread = new UsbReaderThread(connection, bulkIn);
        usbReaderThread.start();
        Log.i(TAG, "USB serial reader started");
    }

    private void stopUsbReader() {
        if (usbReaderThread != null) {
            usbReaderThread.cancel();
            usbReaderThread = null;
        }
    }

    /**
     * Claim all interfaces and return the first bulk-IN endpoint (CDC ACM data interface).
     */
    private UsbEndpoint findBulkInEndpoint(UsbDevice device, UsbDeviceConnection conn) {
        for (int i = 0; i < device.getInterfaceCount(); i++) {
            UsbInterface iface = device.getInterface(i);
            conn.claimInterface(iface, true);
            for (int e = 0; e < iface.getEndpointCount(); e++) {
                UsbEndpoint ep = iface.getEndpoint(e);
                if (ep.getType() == UsbConstants.USB_ENDPOINT_XFER_BULK
                        && ep.getDirection() == UsbConstants.USB_DIR_IN) {
                    return ep;
                }
            }
        }
        return null;
    }

    /**
     * Background thread: reads raw bytes from USB bulk-IN, reassembles newline-delimited JSON lines.
     */
    private class UsbReaderThread extends Thread {
        private final UsbDeviceConnection connection;
        private final UsbEndpoint endpoint;
        private volatile boolean running = true;
        private static final int TIMEOUT_MS = 500;
        private static final int BUF_SIZE = 512;

        UsbReaderThread(UsbDeviceConnection connection, UsbEndpoint endpoint) {
            super("USB-Flipper-Reader");
            this.connection = connection;
            this.endpoint = endpoint;
            setDaemon(true);
        }

        void cancel() {
            running = false;
            connection.close();
        }

        @Override
        public void run() {
            byte[] buf = new byte[BUF_SIZE];
            StringBuilder lineBuffer = new StringBuilder();
            while (running) {
                int read = connection.bulkTransfer(endpoint, buf, BUF_SIZE, TIMEOUT_MS);
                if (read > 0) {
                    String chunk = new String(buf, 0, read, StandardCharsets.UTF_8);
                    lineBuffer.append(chunk);
                    // Process complete lines
                    int nl;
                    while ((nl = lineBuffer.indexOf("\n")) >= 0) {
                        String line = lineBuffer.substring(0, nl).trim();
                        lineBuffer.delete(0, nl + 1);
                        if (!line.isEmpty()) {
                            handleSensorLine(line, "USB");
                        }
                    }
                } else if (read < 0) {
                    Log.w(TAG, "USB bulk transfer error — Flipper may have disconnected");
                    running = false;
                }
            }
            Log.i(TAG, "USB reader thread exiting");
        }
    }

    // -----------------------------------------------------------------------
    // BLE UART (Nordic UART Service) — Method 3
    // -----------------------------------------------------------------------

    private void startBleScan() {
        BluetoothManager bm = (BluetoothManager) getSystemService(Context.BLUETOOTH_SERVICE);
        if (bm == null) return;
        BluetoothAdapter adapter = bm.getAdapter();
        if (adapter == null || !adapter.isEnabled()) {
            Log.i(TAG, "Bluetooth not available/enabled — skipping BLE scan");
            return;
        }
        bleScanner = adapter.getBluetoothLeScanner();
        if (bleScanner == null) return;

        ScanFilter nusFilter = new ScanFilter.Builder()
                .setServiceUuid(new ParcelUuid(NUS_SERVICE_UUID))
                .build();
        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_POWER)
                .build();
        bleScanner.startScan(Collections.singletonList(nusFilter), settings, bleScanCallback);
        Log.i(TAG, "BLE scan started — looking for Flipper UART bridge (NUS)");
    }

    private void stopBleScan() {
        if (bleScanner != null) {
            try { bleScanner.stopScan(bleScanCallback); } catch (Exception ignored) {}
            bleScanner = null;
        }
    }

    private class FlipperBleScanCallback extends ScanCallback {
        @Override
        public void onScanResult(int callbackType, ScanResult result) {
            if (bleConnected) return;
            BluetoothDevice device = result.getDevice();
            Log.i(TAG, "Found NUS device: " + device.getAddress()
                    + " name=" + device.getName());
            stopBleScan();
            bleConnected = true;
            // Connect on main thread is fine; GATT callbacks are delivered on Binder thread
            bleGatt = device.connectGatt(LocalRelayService.this, false, new FlipperGattCallback());
        }

        @Override
        public void onScanFailed(int errorCode) {
            Log.e(TAG, "BLE scan failed: errorCode=" + errorCode);
        }
    }

    private class FlipperGattCallback extends BluetoothGattCallback {
        private StringBuilder lineBuffer = new StringBuilder();

        @Override
        public void onConnectionStateChange(BluetoothGatt gatt, int status, int newState) {
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                Log.i(TAG, "BLE connected — discovering services");
                gatt.discoverServices();
            } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                Log.i(TAG, "BLE disconnected — will re-scan");
                bleConnected = false;
                gatt.close();
                bleGatt = null;
                lineBuffer.setLength(0);
                // Re-start scan after a short delay via retry thread
                new Thread(() -> {
                    try { Thread.sleep(5000); } catch (InterruptedException ignored) {}
                    startBleScan();
                }, "BLE-Rescan").start();
            }
        }

        @Override
        public void onServicesDiscovered(BluetoothGatt gatt, int status) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
                Log.e(TAG, "Service discovery failed: " + status);
                return;
            }
            BluetoothGattService nus = gatt.getService(NUS_SERVICE_UUID);
            if (nus == null) {
                Log.e(TAG, "NUS service not found after discovery");
                return;
            }
            BluetoothGattCharacteristic txChar = nus.getCharacteristic(NUS_TX_CHAR_UUID);
            if (txChar == null) {
                Log.e(TAG, "NUS TX characteristic not found");
                return;
            }
            // Enable notifications on TX characteristic
            gatt.setCharacteristicNotification(txChar, true);
            BluetoothGattDescriptor descriptor = txChar.getDescriptor(CLIENT_CHARACTERISTIC_CONFIG);
            if (descriptor != null) {
                descriptor.setValue(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE);
                gatt.writeDescriptor(descriptor);
                Log.i(TAG, "BLE NUS TX notifications enabled");
            }
        }

        @Override
        public void onCharacteristicChanged(BluetoothGatt gatt,
                                            BluetoothGattCharacteristic characteristic) {
            if (!NUS_TX_CHAR_UUID.equals(characteristic.getUuid())) return;
            byte[] data = characteristic.getValue();
            if (data == null || data.length == 0) return;

            String chunk = new String(data, StandardCharsets.UTF_8);
            lineBuffer.append(chunk);
            // Process complete lines
            int nl;
            while ((nl = lineBuffer.indexOf("\n")) >= 0) {
                String line = lineBuffer.substring(0, nl).trim();
                lineBuffer.delete(0, nl + 1);
                if (!line.isEmpty()) {
                    handleSensorLine(line, "BLE");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Shared sensor-line handler
    // -----------------------------------------------------------------------

    /**
     * Parse a newline-delimited JSON line from Flipper Zero and forward to BTCPC API.
     * Expected format: {"id":"sensor_id","value":23.5,"unit":"C"}
     */
    private void handleSensorLine(String line, String transport) {
        Log.d(TAG, "[" + transport + "] line: " + line);
        JSONObject json;
        try {
            json = new JSONObject(line);
        } catch (JSONException e) {
            Log.w(TAG, "[" + transport + "] Non-JSON line ignored: " + line);
            return;
        }

        String sensorId = json.optString("id", "").trim();
        if (sensorId.isEmpty()) {
            Log.w(TAG, "[" + transport + "] Missing 'id' in sensor reading");
            return;
        }
        if (!json.has("value")) {
            Log.w(TAG, "[" + transport + "] Missing 'value' in sensor reading");
            return;
        }

        // Build the payload to forward (preserve all fields)
        final String sensorIdFinal = sensorId;
        final String payload = line;

        httpExecutor.execute(() -> forwardReading(sensorIdFinal, payload, transport));
    }

    /**
     * POST sensor reading to BTCPC API. Tries local node first, falls back to remote.
     */
    private void forwardReading(String sensorId, String jsonBody, String transport) {
        String localUrl  = LOCAL_API_BASE  + "/sensors/" + sensorId + "/readings";
        String remoteUrl = REMOTE_API_BASE + "/sensors/" + sensorId + "/readings";

        boolean sent = false;
        try {
            String resp = httpPost(localUrl, jsonBody);
            Log.i(TAG, "[" + transport + "] local API OK for " + sensorId + ": " + resp);
            sent = true;
        } catch (IOException localEx) {
            Log.d(TAG, "[" + transport + "] local API unreachable (" + localEx.getMessage()
                    + ") — trying remote");
        }

        if (!sent) {
            try {
                String resp = httpPost(remoteUrl, jsonBody);
                Log.i(TAG, "[" + transport + "] remote API OK for " + sensorId + ": " + resp);
            } catch (IOException remoteEx) {
                Log.e(TAG, "[" + transport + "] both endpoints failed for " + sensorId
                        + ": " + remoteEx.getMessage());
            }
        }
    }

    /**
     * HTTP POST with JSON body; returns response body string.
     */
    private String httpPost(String urlStr, String jsonBody) throws IOException {
        URL url = new URL(urlStr);
        HttpURLConnection conn = (HttpURLConnection) url.openConnection();
        conn.setRequestMethod("POST");
        conn.setRequestProperty("Content-Type", "application/json");
        conn.setRequestProperty("Accept", "application/json");
        conn.setRequestProperty("User-Agent", "BTCPC-Android-Relay/1.0");
        conn.setConnectTimeout(5_000);
        conn.setReadTimeout(10_000);
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
                String line2;
                while ((line2 = reader.readLine()) != null) sb.append(line2);
            }
        }
        if (status < 200 || status >= 300) {
            throw new IOException("HTTP " + status + ": " + sb);
        }
        return sb.length() > 0 ? sb.toString() : "{\"status\":" + status + "}";
    }

    // -----------------------------------------------------------------------
    // Notification channel
    // -----------------------------------------------------------------------

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "BTCPC Relay",
                    NotificationManager.IMPORTANCE_LOW
            );
            channel.setDescription("Relays sensor data from Flipper Zero via USB or Bluetooth");
            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }
}
