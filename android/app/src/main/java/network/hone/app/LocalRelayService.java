package network.hone.app;

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
import android.content.pm.PackageManager;
import android.location.Location;
import android.location.LocationManager;
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
 * LocalRelayService — Flipper Zero sensor relay for HONE.
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
 * Each method parses JSON sensor lines and forwards to the HONE sensor
 * readings API. Flipper packets now preserve device identity in metadata:
 *   {"id":"sensor_id","value":23.5,"metadata":{"flipper_device_id":"...",
 *    "flipper_session_id":"...", "flipper_packet_tag":"..."}}
 * Legacy payloads without this envelope still work.
 *
 * API target: hone.net
 */
public class LocalRelayService extends Service {

    private static final String TAG = "HONERelay";
    private static final int NOTIFICATION_ID = 9420;
    private static final String CHANNEL_ID = "hone_relay";

    // Flipper Zero USB identifiers (STM32 CDC ACM)
    private static final int FLIPPER_VID = 0x0483;
    private static final int FLIPPER_PID = 0x5740;

    // Flipper Zero serial service UUID (custom, not standard NUS)
    private static final UUID NUS_SERVICE_UUID =
            UUID.fromString("8fe5b3d5-2e7f-4a98-2a48-7acc60fe0000");
    // TX characteristic UUID resolved dynamically at connect time (NOTIFY property)
    private volatile UUID activeTxCharUuid = null;
    // RX characteristic UUID — Android writes commands TO the Flipper
    private static final UUID NUS_RX_CHAR_UUID =
            UUID.fromString("19ed82ae-ed21-4c9d-4145-228e62fe0000");
    private volatile BluetoothGattCharacteristic bleRxChar = null;
    private final StringBuilder bleResponseBuf = new StringBuilder();
    // Binary reassembly buffer for signed Flipper frames (magic "BTPC"). BLE
    // notifications are MTU-sized chunks, so a frame may span several; we
    // accumulate here and extract complete frames by their header payload_len.
    private final java.io.ByteArrayOutputStream bleBinaryBuf = new java.io.ByteArrayOutputStream();
    private static final byte[] HONE_FRAME_MAGIC = { 'B', 'T', 'P', 'C' };
    private static final int HONE_FRAME_HEADER_LEN = 71; // 4 magic + 1 type + 2 len + 64 sig
    private static final int HONE_MAX_FRAME = 4096;       // sanity cap
    private static final long BLE_FLUSH_INTERVAL_MS = 30_000;
    private static final UUID CLIENT_CHARACTERISTIC_CONFIG =
            UUID.fromString("00002902-0000-1000-8000-00805f9b34fb");

    // API endpoints — local node first, remote fallback
    private volatile String remoteApiBase = AppPrefs.DEFAULT_API_URL + "/api";

    private static final String ACTION_USB_PERMISSION = "network.hone.app.USB_PERMISSION";
    /** Broadcast when a Flipper Zero is detected via USB and not yet BLE-paired. */
    public static final String ACTION_FLIPPER_USB_DETECTED = "network.hone.app.FLIPPER_USB_DETECTED";
    /** Broadcast when we auto-read the BLE MAC from the Flipper CLI over USB. */
    public static final String ACTION_FLIPPER_MAC_FOUND = "network.hone.app.FLIPPER_MAC_FOUND";
    public static final String EXTRA_FLIPPER_MAC = "mac";

    // WiFi HTTP server (NanoHTTPD) — Method 1
    private static final int WIFI_HTTP_PORT = 6942;
    private FlipperHttpServer wifiHttpServer;

    private UsbManager usbManager;
    private UsbReaderThread usbReaderThread;

    private BluetoothLeScanner bleScanner;
    private BluetoothGatt bleGatt;
    private final ScanCallback bleScanCallback = new FlipperBleScanCallback();
    private volatile boolean bleConnected  = false;
    private volatile boolean bleConnecting = false;

    private final ExecutorService httpExecutor = Executors.newCachedThreadPool();
    private volatile String lastReading = null;

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
                    notifyFlipperUsbDetected();
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
                .setContentTitle("HONE Relay")
                .setContentText("HONE Relay — WiFi · USB · Bluetooth")
                .setSmallIcon(android.R.drawable.ic_menu_upload)
                .setOngoing(true)
                .build();

        try {
            startForeground(NOTIFICATION_ID, notification);
        } catch (Exception e) {
            android.util.Log.w("HONERelay", "startForeground failed: " + e.getMessage());
            stopSelf();
            return START_NOT_STICKY;
        }

        // Method 1: Start WiFi HTTP server (NanoHTTPD on port 6942)
        startWifiServer();

        remoteApiBase = new AppPrefs(this).getEffectiveApiUrl() + "/api";

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

            // CORS preflight
            if (Method.OPTIONS.equals(method)) {
                Response r = newFixedLengthResponse(Response.Status.OK, "application/json", "{}");
                r.addHeader("Access-Control-Allow-Origin", "*");
                r.addHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
                r.addHeader("Access-Control-Allow-Headers", "Content-Type");
                return r;
            }

            // Status endpoint — polled by app UI to show relay health
            if (Method.GET.equals(method) && uri.equals("/_relay/status")) {
                String lastJson = lastReading != null ? lastReading : "null";
                String statusJson = "{\"ok\":true,\"usb_connected\":" + (usbReaderThread != null) +
                        ",\"ble_connected\":" + bleConnected +
                        ",\"last_reading\":" + lastJson + "}";
                Response r = newFixedLengthResponse(Response.Status.OK, "application/json", statusJson);
                r.addHeader("Access-Control-Allow-Origin", "*");
                return r;
            }

            // PC-side pairing: POST /flipper/pair {"mac":"XX:XX:XX:XX:XX:XX"}
            if (Method.POST.equals(method) && uri.equals("/flipper/pair")) {
                try {
                    Map<String, String> bodyMap = new java.util.HashMap<>();
                    session.parseBody(bodyMap);
                    String body = bodyMap.containsKey("postData") ? bodyMap.get("postData") : "";
                    JSONObject json = new JSONObject(body == null ? "{}" : body);
                    String mac = json.optString("mac", "").trim().toUpperCase();
                    if (!mac.matches("([0-9A-F]{2}:){5}[0-9A-F]{2}")) {
                        Response r = newFixedLengthResponse(Response.Status.BAD_REQUEST,
                                "application/json", "{\"error\":\"invalid mac\"}");
                        r.addHeader("Access-Control-Allow-Origin", "*");
                        return r;
                    }
                    new AppPrefs(getApplicationContext()).setFlipperBle(mac, "Flipper Zero");
                    Intent broadcast = new Intent(ACTION_FLIPPER_MAC_FOUND);
                    broadcast.setPackage(getPackageName());
                    broadcast.putExtra(EXTRA_FLIPPER_MAC, mac);
                    sendBroadcast(broadcast);
                    Log.i(TAG, "[PC pair] MAC saved: " + mac);
                    stopBleScan();
                    startBleScan();
                    Response r = newFixedLengthResponse(Response.Status.OK,
                            "application/json", "{\"ok\":true,\"mac\":\"" + mac + "\"}");
                    r.addHeader("Access-Control-Allow-Origin", "*");
                    return r;
                } catch (Exception e) {
                    Response r = newFixedLengthResponse(Response.Status.INTERNAL_ERROR,
                            "application/json", "{\"error\":\"" + e.getMessage() + "\"}");
                    r.addHeader("Access-Control-Allow-Origin", "*");
                    return r;
                }
            }

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

    /** Notify UI that a Flipper is connected via USB (only when not yet BLE-paired). */
    private void notifyFlipperUsbDetected() {
        String savedMac = new AppPrefs(this).getFlipperBleMac();
        if (savedMac.isEmpty()) {
            Intent i = new Intent(ACTION_FLIPPER_USB_DETECTED);
            i.setPackage(getPackageName());
            sendBroadcast(i);
        }
    }

    /**
     * Check already-attached devices at service start (e.g. cable plugged before app launch).
     */
    private void probeAttachedUsb() {
        if (usbManager == null) return;
        for (UsbDevice device : usbManager.getDeviceList().values()) {
            if (isFlipperZero(device)) {
                Log.i(TAG, "Flipper Zero already attached — requesting permission");
                notifyFlipperUsbDetected();
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
        UsbEndpoint[] endpoints = findBulkEndpoints(device, connection);
        UsbEndpoint bulkIn  = endpoints[0];
        UsbEndpoint bulkOut = endpoints[1];
        if (bulkIn == null) {
            Log.e(TAG, "No bulk-IN endpoint found on Flipper Zero");
            connection.close();
            return;
        }
        usbReaderThread = new UsbReaderThread(connection, bulkIn, bulkOut);
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
     * Claim all interfaces; return [bulkIn, bulkOut] from the CDC ACM data interface.
     * Either element may be null if not found.
     */
    private UsbEndpoint[] findBulkEndpoints(UsbDevice device, UsbDeviceConnection conn) {
        UsbEndpoint bulkIn = null, bulkOut = null;
        for (int i = 0; i < device.getInterfaceCount(); i++) {
            UsbInterface iface = device.getInterface(i);
            conn.claimInterface(iface, true);
            for (int e = 0; e < iface.getEndpointCount(); e++) {
                UsbEndpoint ep = iface.getEndpoint(e);
                if (ep.getType() == UsbConstants.USB_ENDPOINT_XFER_BULK) {
                    if (ep.getDirection() == UsbConstants.USB_DIR_IN  && bulkIn  == null) bulkIn  = ep;
                    if (ep.getDirection() == UsbConstants.USB_DIR_OUT && bulkOut == null) bulkOut = ep;
                }
            }
        }
        return new UsbEndpoint[]{bulkIn, bulkOut};
    }

    /**
     * Background thread: reads raw bytes from USB bulk-IN, reassembles newline-delimited lines.
     * On start, sends "bt info\r\n" to the Flipper CLI and watches the response for the BLE
     * MAC address — saving it automatically so no manual entry is needed.
     */
    private class UsbReaderThread extends Thread {
        private final UsbDeviceConnection connection;
        private final UsbEndpoint inEndpoint;
        private final UsbEndpoint outEndpoint; // may be null
        private volatile boolean running = true;
        private static final int TIMEOUT_MS = 500;
        private static final int BUF_SIZE = 512;

        UsbReaderThread(UsbDeviceConnection connection, UsbEndpoint inEndpoint, UsbEndpoint outEndpoint) {
            super("USB-Flipper-Reader");
            this.connection  = connection;
            this.inEndpoint  = inEndpoint;
            this.outEndpoint = outEndpoint;
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
                int read = connection.bulkTransfer(inEndpoint, buf, BUF_SIZE, TIMEOUT_MS);
                if (read > 0) {
                    String chunk = new String(buf, 0, read, StandardCharsets.UTF_8);
                    lineBuffer.append(chunk);
                    int nl;
                    while ((nl = lineBuffer.indexOf("\n")) >= 0) {
                        String line = lineBuffer.substring(0, nl).trim();
                        lineBuffer.delete(0, nl + 1);
                        if (!line.isEmpty()) {
                            checkForMac(line);
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

        /** Watches incoming USB lines for a MAC address and saves it if not yet paired. */
        private void checkForMac(String line) {
            AppPrefs prefs = new AppPrefs(LocalRelayService.this);
            if (!prefs.getFlipperBleMac().isEmpty()) return; // already paired

            String mac = null;

            // Primary: explicit announce from hone_relay.fap — "HONE_MAC:AA:BB:CC:DD:EE:FF"
            if (line.startsWith("HONE_MAC:")) {
                String candidate = line.substring("HONE_MAC:".length()).trim();
                if (candidate.matches("([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}")) {
                    mac = candidate.toUpperCase();
                }
            }

            // Fallback: any line containing a MAC pattern (e.g. Flipper CLI bt info output)
            if (mac == null) {
                java.util.regex.Matcher m = java.util.regex.Pattern
                        .compile("([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}")
                        .matcher(line);
                if (m.find()) mac = m.group(0).toUpperCase();
            }

            if (mac != null) {
                Log.i(TAG, "Auto-detected Flipper BLE MAC: " + mac);
                prefs.setFlipperBle(mac, "Flipper Zero");
                Intent broadcast = new Intent(ACTION_FLIPPER_MAC_FOUND);
                broadcast.setPackage(getPackageName());
                broadcast.putExtra(EXTRA_FLIPPER_MAC, mac);
                sendBroadcast(broadcast);
            }
        }
    }

    // -----------------------------------------------------------------------
    // BLE UART (Nordic UART Service) — Method 3
    // -----------------------------------------------------------------------

    private void startBleScan() {
        if (bleConnected || bleConnecting) return;
        bleConnecting = true;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (checkSelfPermission(android.Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                Log.i(TAG, "BLUETOOTH_CONNECT permission not granted — skipping BLE");
                return;
            }
        }
        BluetoothManager bm = (BluetoothManager) getSystemService(Context.BLUETOOTH_SERVICE);
        if (bm == null) return;
        BluetoothAdapter adapter = bm.getAdapter();
        if (adapter == null || !adapter.isEnabled()) {
            Log.i(TAG, "Bluetooth not available/enabled — skipping BLE");
            return;
        }

        String savedMac = new AppPrefs(this).getFlipperBleMac();
        if (!savedMac.isEmpty()) {
            // Known MAC — connect directly without scanning. This is more reliable than
            // scanning when the device may not include the service UUID in its ad packet.
            try {
                BluetoothDevice device = adapter.getRemoteDevice(savedMac);
                Log.i(TAG, "BLE direct connect to paired Flipper " + savedMac);
                bleGatt = device.connectGatt(this, false, new FlipperGattCallback(),
                        BluetoothDevice.TRANSPORT_LE);
            } catch (Exception e) {
                Log.e(TAG, "BLE direct connect failed: " + e.getMessage());
            }
            return;
        }

        // No MAC yet — scan for the Flipper serial service to discover it
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (checkSelfPermission(android.Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED) {
                Log.i(TAG, "BLUETOOTH_SCAN permission not granted — skipping BLE scan");
                return;
            }
        }
        bleScanner = adapter.getBluetoothLeScanner();
        if (bleScanner == null) return;
        java.util.List<ScanFilter> filters = new java.util.ArrayList<>();
        filters.add(new ScanFilter.Builder().setServiceUuid(new ParcelUuid(NUS_SERVICE_UUID)).build());
        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build();
        bleScanner.startScan(filters, settings, bleScanCallback);
        Log.i(TAG, "BLE scan started — discovering Flipper UART devices");
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
            AppPrefs prefs = new AppPrefs(LocalRelayService.this);
            if (prefs.getFlipperBleMac().isEmpty()) {
                prefs.setFlipperBle(device.getAddress().toUpperCase(),
                        device.getName() != null ? device.getName() : "Flipper Zero");
            }
            stopBleScan();
            // bleConnected set to true in onConnectionStateChange once GATT is up
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
                bleConnected = true;
                bleConnecting = false;
                gatt.discoverServices();
            } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                Log.i(TAG, "BLE disconnected status=" + status + " — will retry");
                bleConnected = false;
                bleConnecting = false;
                bleRxChar = null;
                synchronized (bleResponseBuf) { bleResponseBuf.setLength(0); }
                gatt.close();
                if (bleGatt == gatt) bleGatt = null;
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
            for (BluetoothGattService s : gatt.getServices()) {
                Log.i(TAG, "discovered service: " + s.getUuid());
            }
            BluetoothGattService nus = gatt.getService(NUS_SERVICE_UUID);
            if (nus == null) {
                Log.e(TAG, "Flipper serial service not found — Flipper may not be in relay mode yet; retrying in 8s");
                bleConnected = false;
                bleConnecting = false;
                gatt.disconnect();  // triggers onConnectionStateChange → retry after 5s
                return;
            }
            // Find the TX characteristic: NOTIFY or INDICATE property
            BluetoothGattCharacteristic txChar = null;
            boolean useIndicate = false;
            for (BluetoothGattCharacteristic c : nus.getCharacteristics()) {
                Log.i(TAG, "  char " + c.getUuid() + " props=0x" + Integer.toHexString(c.getProperties()));
                if ((c.getProperties() & BluetoothGattCharacteristic.PROPERTY_NOTIFY) != 0) {
                    txChar = c; useIndicate = false; break;
                }
                if ((c.getProperties() & BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0) {
                    txChar = c; useIndicate = true;
                }
            }
            if (txChar == null) {
                Log.e(TAG, "Flipper serial TX characteristic (NOTIFY/INDICATE) not found");
                return;
            }
            activeTxCharUuid = txChar.getUuid();
            Log.i(TAG, "Flipper serial TX char: " + activeTxCharUuid + " indicate=" + useIndicate);

            // Find RX write characteristic (Android writes commands TO Flipper)
            BluetoothGattCharacteristic rxChar = nus.getCharacteristic(NUS_RX_CHAR_UUID);
            if (rxChar != null) {
                bleRxChar = rxChar;
                Log.i(TAG, "Flipper RX write char found");
            } else {
                for (BluetoothGattCharacteristic c : nus.getCharacteristics()) {
                    if ((c.getProperties() & BluetoothGattCharacteristic.PROPERTY_WRITE) != 0
                            && !c.getUuid().equals(activeTxCharUuid)) {
                        bleRxChar = c;
                        Log.i(TAG, "Flipper RX char fallback: " + c.getUuid());
                        break;
                    }
                }
            }

            // Enable notifications/indications on TX characteristic
            gatt.setCharacteristicNotification(txChar, true);
            BluetoothGattDescriptor descriptor = txChar.getDescriptor(CLIENT_CHARACTERISTIC_CONFIG);
            if (descriptor != null) {
                descriptor.setValue(useIndicate
                        ? BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
                        : BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE);
                gatt.writeDescriptor(descriptor);
                Log.i(TAG, "Flipper serial TX subscribed (indicate=" + useIndicate + ")");
            } else {
                Log.w(TAG, "CCCD descriptor not found on TX char");
            }
        }

        @Override
        public void onDescriptorWrite(BluetoothGatt gatt, BluetoothGattDescriptor descriptor, int status) {
            Log.i(TAG, "CCCD write status=" + status);
            if (status == BluetoothGatt.GATT_SUCCESS) {
                // Request larger MTU so the full JSON response fits in one notify
                gatt.requestMtu(512);
            }
        }

        @Override
        public void onMtuChanged(BluetoothGatt gatt, int mtu, int status) {
            Log.i(TAG, "MTU=" + mtu + " status=" + status);
            // Send first flush command immediately, then start periodic loop
            sendBleFlushCommand(gatt);
            startBleFlushLoop(gatt);
        }

        @Override
        public void onCharacteristicWrite(BluetoothGatt gatt,
                                          BluetoothGattCharacteristic characteristic, int status) {
            Log.i(TAG, "BLE write status=" + status);
        }

        @Override
        public void onCharacteristicChanged(BluetoothGatt gatt,
                                            BluetoothGattCharacteristic characteristic) {
            byte[] data = characteristic.getValue();
            if (data == null || data.length == 0) return;

            // Binary signed-frame path (new firmware): if we're already
            // reassembling a binary frame, or this chunk begins with the "BTPC"
            // magic, treat it as binary and do NOT run it through the JSON path.
            synchronized (bleBinaryBuf) {
                boolean assembling = bleBinaryBuf.size() > 0;
                boolean startsMagic = startsWithMagic(data);
                if (assembling || startsMagic) {
                    bleBinaryBuf.write(data, 0, data.length);
                    drainBinaryFrames();
                    return;
                }
            }

            String chunk = new String(data, StandardCharsets.UTF_8);
            Log.i(TAG, "BLE notify: " + data.length + "b: " + chunk.replaceAll("[\\x00-\\x1f]", "."));

            synchronized (bleResponseBuf) {
                bleResponseBuf.append(chunk);
                String json = extractJsonObject(bleResponseBuf.toString());
                if (json != null) {
                    bleResponseBuf.setLength(0);
                    processBleResponse(json);
                }
            }
        }

        private boolean startsWithMagic(byte[] data) {
            if (data.length < HONE_FRAME_MAGIC.length) return false;
            for (int i = 0; i < HONE_FRAME_MAGIC.length; i++) {
                if (data[i] != HONE_FRAME_MAGIC[i]) return false;
            }
            return true;
        }

        /**
         * Extract every complete binary frame currently buffered and hand each to
         * the native verifier. A frame's total length is header (71) +
         * payload_len (little-endian u16 at offset 5). Leaves any trailing
         * partial frame in the buffer for the next chunk. Resets the buffer if it
         * grows past the sanity cap or loses magic sync.
         */
        private void drainBinaryFrames() {
            byte[] buf = bleBinaryBuf.toByteArray();
            int consumed = 0;

            while (buf.length - consumed >= HONE_FRAME_HEADER_LEN) {
                // Verify magic at the current offset; if lost, drop a byte and resync.
                boolean magicOk = true;
                for (int i = 0; i < HONE_FRAME_MAGIC.length; i++) {
                    if (buf[consumed + i] != HONE_FRAME_MAGIC[i]) { magicOk = false; break; }
                }
                if (!magicOk) { consumed++; continue; }

                int payloadLen = (buf[consumed + 5] & 0xFF) | ((buf[consumed + 6] & 0xFF) << 8);
                int frameLen = HONE_FRAME_HEADER_LEN + payloadLen;
                if (frameLen > HONE_MAX_FRAME) {
                    // Bogus length — resync by dropping the magic and moving on.
                    consumed += HONE_FRAME_MAGIC.length;
                    continue;
                }
                if (buf.length - consumed < frameLen) break; // wait for more chunks

                byte[] frame = new byte[frameLen];
                System.arraycopy(buf, consumed, frame, 0, frameLen);
                consumed += frameLen;

                String pubkey = new AppPrefs(LocalRelayService.this).getFlipperPubkey();
                NativeFlipperService.ingest(frame, pubkey);
                Log.i(TAG, "Flipper binary frame (" + frameLen + "b) handed to native verifier");
            }

            // Retain the unconsumed tail.
            bleBinaryBuf.reset();
            if (consumed < buf.length) {
                bleBinaryBuf.write(buf, consumed, buf.length - consumed);
            }
            if (bleBinaryBuf.size() > HONE_MAX_FRAME) {
                Log.w(TAG, "Flipper binary buffer overflow — resetting");
                bleBinaryBuf.reset();
            }
        }

        private void sendBleFlushCommand(BluetoothGatt gatt) {
            if (bleRxChar == null || !bleConnected) return;
            byte[] cmd = "{\"cmd\":\"flush_readings\"}\n".getBytes(StandardCharsets.UTF_8);
            bleRxChar.setValue(cmd);
            bleRxChar.setWriteType(BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT);
            boolean ok = gatt.writeCharacteristic(bleRxChar);
            Log.i(TAG, "BLE flush command written: " + ok);
        }

        private void startBleFlushLoop(BluetoothGatt gatt) {
            new Thread(() -> {
                while (bleConnected) {
                    try { Thread.sleep(BLE_FLUSH_INTERVAL_MS); } catch (InterruptedException e) { break; }
                    if (bleConnected) sendBleFlushCommand(gatt);
                }
            }, "BLE-Flush").start();
        }
    }

    /** Extract first complete top-level JSON object from s using brace matching. */
    private static String extractJsonObject(String s) {
        int start = s.indexOf('{');
        if (start < 0) return null;
        int depth = 0;
        boolean inStr = false, esc = false;
        for (int i = start; i < s.length(); i++) {
            char c = s.charAt(i);
            if (esc)        { esc = false; continue; }
            if (c == '\\')  { esc = true;  continue; }
            if (c == '"')   { inStr = !inStr; continue; }
            if (!inStr) {
                if (c == '{') depth++;
                else if (c == '}') { depth--; if (depth == 0) return s.substring(start, i + 1); }
            }
        }
        return null;
    }

    private void processBleResponse(String json) {
        Log.i(TAG, "BLE response: " + json);
        try {
            JSONObject obj = new JSONObject(json);
            if (obj.has("readings")) {
                org.json.JSONArray readings = obj.getJSONArray("readings");
                for (int i = 0; i < readings.length(); i++) {
                    JSONObject r = readings.getJSONObject(i);
                    String id = r.optString("id", "flipper/unknown").trim();
                    final String fId  = id;
                    final String fLine = r.toString();
                    httpExecutor.execute(() -> forwardReading(fId, fLine, "BLE"));
                }
                Log.i(TAG, "BLE: forwarding " + readings.length() + " readings");
            }
        } catch (JSONException e) {
            Log.w(TAG, "BLE JSON error: " + e.getMessage());
        }
    }

    // -----------------------------------------------------------------------
    // Shared sensor-line handler
    // -----------------------------------------------------------------------

    /**
     * Parse a newline-delimited JSON line from Flipper Zero and forward to HONE API.
     * Legacy format: {"id":"sensor_id","value":23.5,"unit":"C"}
     * Current Flipper format keeps device/session/auth fields in the top-level
     * payload and this service copies them into metadata before forwarding.
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

        // Build the payload to forward
        final String sensorIdFinal = sensorId;
        final String payload = line;

        httpExecutor.execute(() -> forwardReading(sensorIdFinal, payload, transport));
    }

    /**
     * POST sensor reading to hone.net API.
     */
    private void forwardReading(String sensorId, String jsonBody, String transport) {
        lastReading = jsonBody;
        jsonBody = attachGps(jsonBody);
        jsonBody = injectAccount(jsonBody);

        String url = remoteApiBase + "/sensors/" + sensorId + "/readings";
        try {
            String resp = httpPost(url, jsonBody);
            Log.i(TAG, "[" + transport + "] API OK for " + sensorId + ": " + resp);
        } catch (IOException ex) {
            Log.e(TAG, "[" + transport + "] API failed for " + sensorId + ": " + ex.getMessage());
        }
    }

    /** Inject relay_gps into jsonBody using last-known location (no active scan). */
    private String attachGps(String jsonBody) {
        try {
            if (checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION)
                    != PackageManager.PERMISSION_GRANTED
                && checkSelfPermission(android.Manifest.permission.ACCESS_COARSE_LOCATION)
                    != PackageManager.PERMISSION_GRANTED) {
                return jsonBody;
            }
            LocationManager lm = (LocationManager) getSystemService(Context.LOCATION_SERVICE);
            if (lm == null) return jsonBody;

            Location loc = null;
            for (String provider : new String[]{LocationManager.GPS_PROVIDER,
                                                LocationManager.NETWORK_PROVIDER}) {
                try {
                    Location candidate = lm.getLastKnownLocation(provider);
                    if (candidate != null) {
                        if (loc == null || candidate.getTime() > loc.getTime()) loc = candidate;
                    }
                } catch (Exception ignored) {}
            }

            if (loc == null) return jsonBody;
            long ageMs = System.currentTimeMillis() - loc.getTime();
            if (ageMs > 5 * 60 * 1000L) return jsonBody; // skip if > 5 min old

            JSONObject json = new JSONObject(jsonBody);
            JSONObject gps  = new JSONObject();
            gps.put("lat",       loc.getLatitude());
            gps.put("lon",       loc.getLongitude());
            gps.put("accuracy",  loc.getAccuracy());
            gps.put("timestamp", loc.getTime());
            gps.put("provider",  loc.getProvider());
            json.put("relay_gps", gps);
            return json.toString();
        } catch (Exception e) {
            Log.w(TAG, "GPS attach failed: " + e.getMessage());
            return jsonBody;
        }
    }

    /** Inject account from prefs as relay_account so the server can attribute the reading. */
    private String injectAccount(String jsonBody) {
        try {
            String account = new AppPrefs(this).getAccount();
            if (account == null || account.isEmpty()) return jsonBody;
            JSONObject json = new JSONObject(jsonBody);
            if (!json.has("account")) json.put("account", account);
            // Also set relay_account so multi-gateway witnessing tracks this phone
            if (!json.has("relay_account")) json.put("relay_account", account);
            return json.toString();
        } catch (Exception e) {
            return jsonBody;
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
        conn.setRequestProperty("User-Agent", "HONE-Android-Relay/1.0");
        String jwt = new AppPrefs(this).getJwt();
        if (jwt != null && !jwt.isEmpty()) {
            conn.setRequestProperty("Authorization", "Bearer " + jwt);
        }
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
                    "HONE Relay",
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
